/mattpocock-skills:implement {{ISSUE_URL}}

You are running AFK in a sandbox, on branch `{{BRANCH}}`, which is already checked out.
Nobody will answer a question, so do not ask one. Treat the issue, its comments and its
parent spec (if it has one) as settled. Read them with `gh issue view {{ISSUE_NUMBER}} --comments`.

Commit to `{{BRANCH}}`, and reference `#{{ISSUE_NUMBER}}` in each commit message. Do not
push, open a PR or close the issue. The runner does all three once you finish.

## This repository

- This is the TOON Protocol **blob app**: a Blossom server behind connector routes. People
  pay per KiB through the connector to store a blob, and anyone reads it free by its sha256
  hash. It is a plain HTTP app. It speaks no ILP and validates no payment; it records the
  payment the connector states (`X-TOON-Payer` / `-Amount` / `-Chain`) and trusts it.
- The repository is still at the design stage: there is no code, manifest or CI yet. The
  design is `README.md`, the vocabulary is `GLOSSARY.md`, the decisions are `docs/adr/`, and
  the research behind them is `docs/research/blob-storage-for-relay.md`. The README says the
  app will be Rust, on the relay's toolchain pin. A ticket that sets up the first crate
  follows that, and anything it can't see in this tree, ask about on the issue.
- `CLAUDE.md` says how the agent skills and the factory are set up here. Read it, then
  `README.md`, `GLOSSARY.md`, and the ADRs the ticket touches. Where an ADR and another
  document disagree, the ADR wins. Use the glossary's terms, never the ones it lists under
  _Avoid_ (for example, "store" is the separate Arweave app, never this one).
- Never put payment validation in this app (ADR 0003): all of it lives in
  toon-protocol/connector.
- The issues labelled `wayfinder:*` are planning tickets for a human. Never work on one,
  and never edit a wayfinder map.
- After you finish, the runner runs the gate itself and won't open a PR while it is red.
  The gate is the `gate` (or `checks`) job of `.github/workflows/ci.yml` on `main`, and if
  there is none it runs nothing. Run those commands yourself before you commit if the
  file exists. Never weaken, skip or delete a test, and never loosen a lint, to get green.
- A ticket that needs a live box, a funded key or an on-chain write is not something you
  can do from here. That includes a real connector, publishing `ghcr.io/toon-protocol/blossom`,
  and changing another bundle's `connector.toml` or Caddyfile. Say so in a comment on the issue
  rather than guessing.

## When you cannot finish

Stop only when a genuinely new decision is needed, the action is irreversible, it touches
real funds, or it needs a credential that no workflow exposes. In that case, commit nothing
and explain what blocks you in a comment on the issue (`gh issue comment {{ISSUE_NUMBER}}`).
The runner moves an issue with no commits to `needs-triage`.

If your context is getting full (around 150k tokens) before you are done, commit what works,
write the remaining steps to `.sandcastle/logs/handoff-{{ISSUE_NUMBER}}.md`, commit it with
`git add -f`, and end your turn. A fresh session continues from your commits.

When the ticket is done and committed, output <promise>COMPLETE</promise>.

If you stopped because you're blocked, output <promise>BLOCKED</promise> instead, after your
comment on the issue. The runner then ends the run. Otherwise it starts another session, which
hits the same blocker and posts the same comment again.
