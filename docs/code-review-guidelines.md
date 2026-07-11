# Code Review Guidelines

The reviewer's half of the contract — [`engineering-guidelines.md`](engineering-guidelines.md) is
the author's half. Applies to **any** reviewer: human, Claude, Codex, OpenCode. In Claude Code the
[`parler-review`](../.claude/skills/parler-review/SKILL.md) skill and the `code-reviewer` agent
([`.claude/agents/code-reviewer.md`](../.claude/agents/code-reviewer.md)) both execute this
document; any other tool just reads it and follows it.

## What a review is here

Judge the diff against the contract: hard gates, invariants, quality bar, tests. The output is a
short list of **verified** findings a maintainer can act on — not a tour of the diff, not style
opinions, not praise padding.

## Process

1. **Scope.** `git diff origin/main...` for a branch, plus `git status` for uncommitted work —
   review the union unless told otherwise. For a PR: `gh pr diff <n>`.
2. **Load the contract.** Skim "Hard gates" and "Invariants" in `engineering-guidelines.md` if not
   already in context. Check `tasks/lessons.md` for traps touching the same files.
3. **Read the changed code in place** — the full function/module around each hunk, never diff
   context lines alone. Follow the data from every new or changed entry point.
4. **Verify every candidate finding** (rules below) before it goes in the report.
5. **Run the gates** for anything beyond docs: `scripts/verify.sh`, or `make ci` when more than one
   crate is touched. Report the result either way.

## Verify before you report

This repo has been burned by an inflated audit — a headline "CRITICAL: panics on network input"
that was a `panic!` inside a `#[test]`, unreachable from the network. So:

- Read the cited lines yourself, in the file, with enough context to know what sits above and
  below them.
- Confirm the path is **production-reachable**: `#[cfg(test)]`, `#[test]`, examples, and benches
  don't count.
- State the failure concretely: *this* input or state produces *this* wrong result. If you can't
  construct it, downgrade or drop the finding.
- One false CRITICAL discredits the whole report. When genuinely unsure, mark the finding
  "unverified — needs a look" instead of dressing it in certainty.

## Severity ladder

| Level | Meaning |
|-------|---------|
| CRITICAL | Security invariant broken, data loss, or a wire change that strands deployed clients |
| HIGH | Wrong behavior on a realistic path; a messaging or concurrency invariant violated |
| MEDIUM | Bug on an edge path; unbounded resource; a gate shipped without its negative tests |
| LOW | Real quality issue, worth fixing while the file is open |
| NIT | Optional polish — three at most, or drop them |

## Report format

Most severe first. Per finding:

```
[SEVERITY] file.rs:123 — one-sentence defect.
  Failure: <input/state> → <wrong result>.
  Fix: <one-line direction>.
```

End with a verdict — **approve** / **approve with nits** / **needs changes** — plus gate results if
you ran them. If nothing survived verification, say so plainly; don't invent findings to look
thorough.

## Checklists

### Correctness

- `Option` flag handling distinguishes "absent" from "explicitly 0/off" (`match`, not `.filter()`).
- Empty / first-time paths handled, not just merge-into-existing (fresh `toml_edit` docs, first
  run, empty tables).
- No `unwrap` / `expect` / `panic!` reachable from network or user input.
- Errors keep context; failure paths exercised, not just happy paths.

### Security (repo invariants)

- Seed never serialized, logged, or sent; cards stay self-signed; visibility defaults private.
- New or changed gate → **every** writer path audited (Invite, Redeem, Serve, DM/Service
  resolution), not just the new one. New read surface → enumerate exactly what the new capability
  reaches, proven with negative tests (wrong token → 401, no id/blob leak, cursor unchanged).
- Token kinds sharing a table are separated by scope **checked both ways**.
- On-disk secrets use `write_private_file` (0600 temp + rename), never write-then-chmod.
- Secret comparisons are constant-time.

### Protocol & compatibility

- Is the wire change additive? Do old deployed clients (parler-hub.fly.dev) still work?
- Could this have been a `com.parler.<x>` extension part instead of a hub/protocol change?
- Did a `parler-protocol` change ripple through hub, connector, CLI, MCP, and the web REST surface?
- Signatures cover only hub-verbatim fields (never `mentions`).

### Concurrency & resources

- No lock guard held across `.await`; blocking I/O off the async runtime.
- Push/notify never advances a cursor; parked waits arm the notify, re-check, then await.
- Everything a stranger can grow is bounded (connections, handshake time, sizes).
- Per-connection budgets divided by pool size, not silently multiplied.

### Tests

- New behavior has a test that fails without the change.
- Tests don't mutate process env (parallel-test races) — pure decision fns are tested instead.
- UX flows verified through the real entry point, not a lighter proxy.
- Test code passes clippy too — the gate lints `--all-targets`.

### Process

- No `cargo fmt` blast radius in the diff (whole-file reflows are a red flag).
- New dep: already-transitive checked, lockfile edge recorded, license passes `cargo-deny`.
- Docs updated with the behavior; conventional commit.

## What not to flag

- Formatting. The repo is hand-formatted by design; never suggest `cargo fmt` or a rustfmt gate.
- Style preferences where the repo already has an idiom — match, don't modernize.
- Refactors beyond the diff's scope, speculative generality, "consider adding" padding.
- Missing features that weren't in scope.
- Panics / unwraps in test code.
