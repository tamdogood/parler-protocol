---
name: parler-review
description: >-
  Review a diff, branch, or PR in the Parler Protocol repo against the project
  review contract (docs/code-review-guidelines.md). Use when asked to review
  changes, audit a branch, check a PR, or self-review before landing. Produces
  verified, severity-ranked findings — no style noise, no unverified criticals.
license: Apache-2.0
compatibility: claude-code
allowed-tools:
  - Read
  - Grep
  - Glob
  - Bash
---

# Review a Parler Protocol change

Execute [`docs/code-review-guidelines.md`](../../../docs/code-review-guidelines.md) — read it now
if it isn't in context. That document is the contract; this skill is the runbook.

1. **Scope:** `git diff origin/main...` plus `git status` (include working-tree changes). For a
   PR: `gh pr diff <n>`.
2. **Load:** skim "Hard gates" and "Invariants" in `docs/engineering-guidelines.md`; check
   `tasks/lessons.md` for traps in the touched files.
3. **Read every changed function in the file**, full context — never review from diff hunks alone.
4. **Verify every candidate finding:** production-reachable (not `#[cfg(test)]`), concrete failure
   scenario, cited lines actually read. Drop or downgrade what you can't prove.
5. **Run the gates** for non-doc changes: `scripts/verify.sh --rust-only`, or
   `CI_SKIP_WEB=1 make ci` when more than one crate is touched.
6. **Report:** severity-ranked findings in the contract's format, then a verdict
   (approve / approve with nits / needs changes) plus gate results.

This is a read-only pass: never modify files during the review; Bash is for `git`, `gh`, and the
gates. For a large diff, offload to the `code-reviewer` agent (`.claude/agents/code-reviewer.md`)
to keep the main context clean.
