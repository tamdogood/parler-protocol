---
description: Autonomously advance the Parler Protocol backlog by one verified item (the loop's single iteration)
argument-hint: "[optional: a specific backlog item or focus to override the top item]"
---

You are running one iteration of the Parler Protocol autonomous loop. Do exactly ONE backlog item, prove it
works, commit it, and stop. Another iteration will pick up the next item — do not try to do more.

## 0. Orient (read before acting)
- Read `tasks/lessons.md` in full — these are hard-won rules; obey them. (Notably: **never run
  `cargo fmt`**; additive/backward-compatible changes only; the hub is deployed live.)
- Read `tasks/backlog.md`. Your item is the **top unchecked `[ ]` item in "Now"**, unless
  `$ARGUMENTS` names a different one — then do that instead.
- Skim `CLAUDE.md` for workflow rules.

## 1. Pick & plan
- State the one item you're doing in a sentence. If it is too large to land behind one green
  `scripts/verify.sh` run, **split it**: replace it in `backlog.md` with 2–4 smaller sub-items, do the
  first, and stop. Splitting counts as a valid iteration.
- If the top item is blocked (e.g. needs a human decision, an external dep, or a non-additive wire
  change), mark it `[BLOCKED] <reason>` in `backlog.md`, move to the next eligible item, and note it in
  your summary. Don't silently skip.

## 2. Establish the baseline signal
- Run `scripts/verify.sh --rust-only` **before** changing anything, to confirm the gate is green on a
  clean tree. If it's already red, your job this iteration is to get it green — treat that as the item.
  (`--rust-only` is the loop's standard gate: the `web/` app is human-driven and out of scope — see the
  guardrails. The full `scripts/verify.sh` exists for Tam's manual pre-push checks, not the loop.)

## 3. Modify (smallest coherent change)
- Make the minimal change that completes the item. Match the surrounding hand-formatted style. Touch
  only what's necessary. Add/extend a test that would fail without your change.

## 4. Verify (close the loop)
- Run `scripts/verify.sh --rust-only`. It must end in `VERIFY: PASS`.
- On failure: read the output, fix the root cause, re-run. **No-progress guard:** if the *same* failure
  survives two fix attempts, stop — write `[BLOCKED] <stage>: <what you saw>` next to the item in
  `backlog.md`, append the finding to `tasks/lessons.md`, and report. Do not thrash.

## 5. Land it
- Only after `VERIFY: PASS`:
  - Check the item off (`[x]`) in `tasks/backlog.md`.
  - Append a short write-up to `tasks/todo.md` (what changed, why, tests, verification line).
  - If you learned something correction-worthy, append a rule to `tasks/lessons.md`.
  - Commit on the **current branch** (never `main` — if on `main`, create a branch first):
    `git add -A && git commit`. Message: `feat|fix|refactor(<scope>): <item>` and the trailer
    `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
  - Do **not** push or open a PR unless the surrounding `/loop` invocation explicitly asked for it.

## 6. Report & stop
- Summarize in 3–5 lines: the item, the change, the verify result, what's now next in the backlog.
- Stop. One item per iteration.

## Hard guardrails
- **The `web/` app is human-driven and out of scope.** Never edit anything under `web/` and never run
  the web build. If a backlog item needs a UI/site change, do only the non-web part (Rust/CLI/protocol)
  and leave a `[HUMAN] web: <what's needed>` note on the item for Tam to pick up by hand.
- Never `cargo fmt`. Never relax `-D warnings` to pass. Never make a non-additive wire-protocol change.
- Never commit a red tree. Never touch the Icebox without a human.
- If anything feels architecturally wrong, stop and surface it instead of pushing through.
