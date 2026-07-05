---
name: code-standards
description: >-
  Apply Parler Protocol's engineering contract to a code change. Use whenever
  implementing a feature, fixing a bug, or refactoring in this repo — before
  writing code, not after. Walks orient → baseline → smallest change → test →
  verify → self-review, and enforces the hard gates (never cargo fmt, clippy
  -D warnings, make ci green, additive wire changes only, security invariants).
license: Apache-2.0
compatibility: claude-code
---

# Code standards — the change workflow

You are making a code change in the Parler Protocol repo. The full contract is
[`docs/engineering-guidelines.md`](../../../docs/engineering-guidelines.md) — read it once per
session (it's short). This skill is the execution order.

## Steps

1. **Orient.** Read `tasks/lessons.md` (hard-won traps; obey them). Find the subsystem's doc in
   `AGENTS.md`'s index and read only that one.
2. **Plan if non-trivial** (3+ steps or an architectural choice): short plan in `tasks/todo.md` —
   files touched, tests to add, risks. Going sideways → stop and re-plan.
3. **Baseline:** `scripts/verify.sh --rust-only` green *before* touching anything. A pre-existing
   red is the first task.
4. **Implement** the smallest root-cause change. Hand-match the surrounding style.
   **Never `cargo fmt`.**
5. **Test:** new behavior gets a test that fails without the change; security gates get
   negative-assertion tests (wrong token → 401, cursor unchanged, no leak).
6. **Verify:** `CI_SKIP_WEB=1 make ci` while iterating; full `make ci` if `web/` or dependencies
   changed. Green before "done".
7. **Self-review** your own diff against `docs/code-review-guidelines.md` (or invoke
   `parler-review`). Fix findings before presenting.
8. **Close out:** docs updated in the same change; append to `tasks/lessons.md` if you were
   corrected or surprised; conventional commit message (`feat|fix|docs(scope): …`).

## Tripwires — stop and check the guidelines when you're about to…

- change anything in `crates/parler-protocol/` → ripple rules + additive-only wire contract
- add a dependency → transitive check, one non-`--locked` build, strict `cargo-deny` licenses
- touch membership, tokens, or any auth path → audit every writer/reader path, negative tests
- write a secret to disk → `parler_auth::write_private_file`, never write-then-chmod
- hold a lock near an `.await`, or read env inside logic → extract a pure decision fn
- edit `web/` during an autonomous loop → don't; leave a `[HUMAN] web: …` note instead

## No-progress guard

The same failure surviving two fix attempts means stop: append the finding to `tasks/lessons.md`,
mark the item `[BLOCKED]`, and surface it. Don't thrash.
