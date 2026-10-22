// Fails the build from blossom-compliance's results.json, which the suite writes but never
// exits non-zero for.
//
// A case that is not a pass is judged by `expected-failures.json`:
//   - BUD-07 is never judged (ADR 0003: the blob app does not implement it).
//   - A case listed there may fail, warn or skip, but turns the build red once it passes: that
//     ticket's surface is built, so it removes its own markers.
//   - Any other failure or warning is unexpected and fails the build.
//   - A marker naming a case the suite no longer reports is stale and fails the build.
// Every BUD except 00 and 01 is non-mandatory in the suite, which reports its failures as
// `warn`, so warnings are judged the same as failures.
//
// Usage: node check.mjs <results.json> [expected-failures.json]

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const [resultsPath, markersPath = join(dirname(fileURLToPath(import.meta.url)), "expected-failures.json")] =
  process.argv.slice(2);
if (!resultsPath) {
  console.error("usage: node check.mjs <results.json> [expected-failures.json]");
  process.exit(2);
}

const IGNORED_BUDS = new Set(["BUD-07"]);
const key = (bud, name) => `${bud} ${name}`;

const results = JSON.parse(readFileSync(resultsPath, "utf8"));
const markers = new Map(
  JSON.parse(readFileSync(markersPath, "utf8")).map((m) => [key(m.bud, m.case), m]),
);

const problems = [];
const seen = new Set();
const counts = { pass: 0, expected: 0, ignored: 0 };

if (results.length !== 1) {
  problems.push(`expected results for exactly one server, got ${results.length}`);
}
for (const server of results) {
  for (const bud of server.buds) {
    for (const c of bud.cases) {
      const id = key(bud.budId, c.name);
      const marker = markers.get(id);
      if (marker) seen.add(id);
      if (IGNORED_BUDS.has(bud.budId)) {
        counts.ignored++;
      } else if (marker) {
        if (c.status === "pass") {
          problems.push(`now passing, remove its marker (${marker.surface}): ${id}`);
        } else {
          counts.expected++;
        }
      } else if (c.status === "fail" || c.status === "warn") {
        problems.push(`unexpected ${c.status}: ${id}: ${c.message ?? ""}`);
      } else {
        counts.pass++;
      }
    }
  }
}
for (const [id, marker] of markers) {
  if (!seen.has(id)) problems.push(`stale marker (${marker.surface}), the suite reports no such case: ${id}`);
}

console.log(
  `blossom-compliance: ${counts.pass} ok, ${counts.expected} expected failures, ${counts.ignored} BUD-07 ignored`,
);
if (problems.length > 0) {
  for (const p of problems) console.error(`  ${p}`);
  process.exit(1);
}
