import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { test } from 'node:test';
import { gateFromCi, runGate } from './run-gate.ts';

const CI = `
name: CI
on: [push, pull_request]
jobs:
  docs:
    runs-on: ubuntu-latest
    steps:
      - run: echo not the gate
  gate:
    name: Format, test and lint
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
      - name: Install
        run: npm ci
      - run: |
          npm run lint
          npm test
      - name: Only on main
        if: github.ref == 'refs/heads/main'
        run: npm run deploy-check
      - name: Needs a GitHub expression
        run: echo \${{ github.sha }}
      - name: In a folder, with env
        working-directory: web
        env:
          CI: 'true'
          NAME: it's
        run: npm run build
`;

test('a gate job in ci.yml becomes the gate: its run steps, in order', () => {
  const { steps } = gateFromCi(CI);
  assert.deepEqual(
    steps.map((s) => s.command),
    [
      'npm ci',
      'npm run lint\nnpm test',
      "cd 'web' && export CI='true' NAME='it'\\''s' && npm run build",
    ]
  );
  assert.equal(steps[0]!.name, 'Install');
});

test('steps that cannot be run faithfully are skipped, and the notes say which', () => {
  const { notes } = gateFromCi(CI);
  const text = notes.join('\n');
  assert.match(text, /Only on main/);
  assert.match(text, /Needs a GitHub expression/);
  assert.doesNotMatch(text, /actions\/checkout/);
});

test('a job called checks is the gate when there is no gate job', () => {
  const { steps } = gateFromCi('jobs:\n  checks:\n    steps:\n      - run: make check\n');
  assert.deepEqual(
    steps.map((s) => s.command),
    ['make check']
  );
});

test('no ci.yml: the gate runs nothing, and says so', () => {
  const { steps, notes } = gateFromCi(null);
  assert.deepEqual(steps, []);
  assert.match(notes.join('\n'), /no \.github\/workflows\/ci\.yml/i);
  assert.match(notes.join('\n'), /nothing/i);
});

test('a ci.yml with no gate job runs nothing, and says why', () => {
  const { steps, notes } = gateFromCi('jobs:\n  lint:\n    steps:\n      - run: x\n');
  assert.deepEqual(steps, []);
  assert.match(notes.join('\n'), /no `gate` or `checks` job/);
});

test('a gate job with no runnable step runs nothing, and says so', () => {
  const { steps, notes } = gateFromCi(
    'jobs:\n  gate:\n    steps:\n      - uses: actions/checkout@v4\n'
  );
  assert.deepEqual(steps, []);
  assert.match(notes.join('\n'), /no runnable step/);
});

test('a ci.yml that is not valid YAML is an error, not a silent empty gate', () => {
  assert.throws(() => gateFromCi('jobs: [unclosed'));
});

test('a GitHub expression in env or working-directory skips the step too', () => {
  const { steps, notes } = gateFromCi(
    'jobs:\n  gate:\n    steps:\n      - name: Secretive\n        env:\n          T: ${{ secrets.T }}\n        run: make\n'
  );
  assert.deepEqual(steps, []);
  assert.match(notes.join('\n'), /Secretive/);
});

test('the expression note names the expression syntax literally', () => {
  const { notes } = gateFromCi('jobs:\n  gate:\n    steps:\n      - run: echo ${{ x }}\n');
  assert.match(notes.join('\n'), /a \$\{\{ \}\} expression/);
});

// A sandbox whose exec behaves like the sandcastle-agent image's: the script goes to
// `sh`, which there is dash. dash has no `pipefail`, so a top-level `set` asking for it
// fails with exit 2 before anything runs, as it did in the AFK runs. Anything else runs
// for real under the local `sh`, in `cwd`.
const dashLikeSandbox = (cwd: string) =>
  ({
    exec: async (script: string, options?: { onLine?: (line: string) => void }) => {
      if (/^\s*set\b[^\n]*pipefail/m.test(script)) {
        return { exitCode: 2, stdout: '', stderr: 'sh: 1: set: Illegal option -o pipefail' };
      }
      const run = spawnSync('sh', ['-c', script], { cwd, encoding: 'utf8' });
      for (const line of `${run.stdout}${run.stderr}`.split('\n').filter(Boolean)) {
        options?.onLine?.(line);
      }
      return { exitCode: run.status ?? 1, stdout: run.stdout, stderr: run.stderr };
    },
  }) as unknown as Parameters<typeof runGate>[0];

const step = (command: string) => ({ name: 'step', command });
const quietly = async <T>(fn: () => Promise<T>): Promise<T> => {
  const log = console.log;
  console.log = () => {};
  try {
    return await fn();
  } finally {
    console.log = log;
  }
};

test('a passing step passes the gate on a sandbox whose sh is dash', async () => {
  const result = await quietly(() => runGate(dashLikeSandbox(tmpdir()), [step('true')]));
  assert.equal(result.passed, true);
  assert.deepEqual(result.ran, ['step']);
});

test('each step runs under bash, not sh', async () => {
  // $BASH names the running shell: bash invoked as `sh` reports it as sh, and dash leaves it unset.
  const result = await quietly(() =>
    runGate(dashLikeSandbox(tmpdir()), [step('test "${BASH##*/}" = bash')])
  );
  assert.equal(result.passed, true);
});

test('each step runs with -e: a failing line stops the step', async () => {
  const result = await quietly(() =>
    runGate(dashLikeSandbox(tmpdir()), [step('false\necho after')])
  );
  assert.equal(result.passed, false);
  assert.doesNotMatch(result.failure!.output, /after/);
});

test('each step runs with pipefail: a failure early in a pipe fails the step', async () => {
  const result = await quietly(() => runGate(dashLikeSandbox(tmpdir()), [step('false | true')]));
  assert.equal(result.passed, false);
});

test('a step containing single quotes survives the quoting', async () => {
  const result = await quietly(() =>
    runGate(dashLikeSandbox(tmpdir()), [step(`echo 'one quoted' "it's" 'it'\\''s'\nexit 3`)])
  );
  assert.equal(result.failure!.exitCode, 3);
  assert.equal(result.failure!.output.trim(), "one quoted it's it's");
});
