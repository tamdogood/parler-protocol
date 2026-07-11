# Engineering Guidelines

The contract every code change in this repo is written — and reviewed — against. It applies to
**any** contributor: human, Claude, Codex, OpenCode, Cursor, or whatever comes next. If your tool
auto-loads `AGENTS.md`, that file sent you here; read this once per session, then follow it.

The docs are layered so you load only what you need — that layering is itself the token budget:

| Layer | File | Role |
|-------|------|------|
| Map | [`AGENTS.md`](../AGENTS.md) | What the project is, where things live |
| Contract | this file + [`code-review-guidelines.md`](code-review-guidelines.md) | How to write and how to review a change |
| Trap log | [`tasks/lessons.md`](../tasks/lessons.md) | Append-only repo-specific gotchas — read at session start |

`tasks/lessons.md` is upstream of this file: corrections land there first, with the war story; when
one hardens into a durable rule it gets promoted here. If the two ever disagree, the newer lesson
wins — and fix the drift in the same change.

## The workflow every change follows

1. **Orient.** Read `tasks/lessons.md`. Find your subsystem's doc in the `AGENTS.md` index and read
   only that one. Don't start editing until you can say what the change touches and what it must
   not touch.
2. **Plan (non-trivial work only).** 3+ steps or an architectural choice → write a short plan to
   `tasks/todo.md`: files touched, tests to add, risks. If implementation goes sideways, stop and
   re-plan; don't push through.
3. **Baseline.** Confirm the gate is green *before* changing anything:
   `scripts/verify.sh --rust-only` (fast) or `make ci`. A pre-existing red is its own task first.
4. **Implement.** The smallest coherent change that fixes the root cause. No temporary patches, no
   drive-by refactors, no scope creep. Match the surrounding hand-formatted style exactly.
5. **Test.** New behavior ships with a test that fails without the change. Security behavior ships
   with negative assertions — the thing the gate must *prevent*, proven prevented.
6. **Verify.** Run `make ci` until green before calling it done. Green means done; "should work"
   doesn't.
7. **Self-review.** Walk [`code-review-guidelines.md`](code-review-guidelines.md) against your own
   diff before presenting it. Fix what you find; don't ship it for the reviewer to catch.

## Hard gates (violating any of these fails the change)

- **Never run `cargo fmt`.** The repo is hand-formatted, deliberately, with no rustfmt gate. A
  repo-wide reflow buries the real diff.
- **`clippy -D warnings` passes on `--all-targets`** — test code gets linted too. No `#[allow]`
  without a one-line justifying comment.
- **`make ci` green before done.** It mirrors the cloud pipeline exactly.
- **Wire changes are additive only.** The hub is deployed (parler-hub.fly.dev) with old clients in
  the wild. New frames and `com.parler.<x>` extension kinds are fine; renaming, removing, or
  retyping anything on the wire needs a human sign-off first.
- **New dependency?** Check it isn't already transitive (`grep '^name = "<dep>"' Cargo.lock`), run
  one plain (non-`--locked`) `cargo build` to record the lockfile edge, and know that `cargo-deny`
  enforces licenses strictly.
- **Conventional commits** (`feat:` / `fix:` / `docs:` / … optionally scoped), small focused PRs,
  docs updated in the same PR as the behavior change.

## Invariants (the architecture-level contracts)

Security:

- The **seed never leaves the device** — never serialized, logged, or sent. Cards are
  **self-signed** and re-verifiable against `card.id`; the hub is a relay, not a root of trust.
  Visibility is **private by default**. A public-URL private hub requires a `--join-secret`,
  compared constant-time.
- The hub sees plaintext. Crypto protects identity, not confidentiality from the operator — never
  claim end-to-end privacy.
- Secrets on disk go through `parler_auth::write_private_file` (0600 `create_new` temp + atomic
  rename), never `write`-then-`chmod`.
- A gate is only as strong as every path around it. Gate membership → audit **every** writer of
  `members` (Invite, Redeem, Serve, DM/Service resolution), not just the new one. Expose data to a
  new audience → enumerate exactly what the new capability reaches and prove the rest unreachable
  with negative tests.
- Capabilities sharing a table are separated by **scope, checked both ways** — a watch token must
  not validate as a hub token, nor the reverse.

Messaging:

- The per-(room, agent) **cursor is the source of truth**; push (`Delivery`) is a best-effort
  latency layer. A push must never advance a cursor; a missed push is always recovered by `Pull`.
- An explicit-`since` re-read never advances the cursor. Render caps and cursor advancement are
  decoupled: a capped render is lossless *because* the cursor advances only through the returned
  batch.
- Sign only fields the hub stores verbatim (parts / target / replyTo / ts / uid), never fields it
  rewrites (mentions).

Concurrency & resources:

- Never hold a lock guard across an `.await` — a `parking_lot` guard held there deadlocks the
  runtime. The store is synchronous; long-poll parking lives in the socket layer, and the
  arm-notify → re-check → await order is load-bearing (anything else is a lost wakeup).
- Blocking I/O (blob reads/writes) goes off the async runtime.
- Bound everything a stranger can grow: connection counts, handshake time, message / WS / blob
  sizes.
- Per-connection budgets multiply by the pool. Budget a **total** and divide by the connection
  count (see `TOTAL_CACHE_KIB` in `Store::open`).

## Rust quality bar

- No `unwrap` / `expect` / `panic!` on any path reachable from the network or user input. Tests and
  provably-infallible cases are fine — say why in a word when it isn't obvious.
- Errors bubble with `?` and carry enough context to act on; don't stringify and rethrow.
- Factor env / precedence / mode decisions into **pure functions** (group inputs in a struct once
  you pass ~7 args), keeping the env read a one-liner at the call site. Never mutate process env in
  a test — cargo runs tests in parallel threads of one process.
- A flag with `conflicts_with` must not also take `env=` — clap counts an exported var as "flag
  present". Read the env by hand where explicit flags should win.
- Distinguish "absent" from "explicitly zero/off" with a `match` on the `Option`, not `.filter()`.
- Reach for a `com.parler.<x>` `Part::Extension` before touching the hub: parts are persisted and
  returned verbatim, so an additive feature can ship with zero protocol/schema change and work
  against the deployed hub. First-class frame fields are for data the hub itself must act on.

## Boundaries and ripple

- A change to `parler-protocol` ripples into `parler-hub`, `parler-connector`, `parler-cli` (CLI
  *and* MCP), and the hub's REST API consumers. Update and test all of them, not one crate.

## Testing standards

- Unit tests live next to the code (`#[cfg(test)]`), e2e in `crates/parler-connector/tests/`, the
  HTTP contract in `crates/parler-hub/tests/smoke.rs`.
- Test the **empty / first-time path**, not just merge-into-existing — the `toml_edit` fresh-doc
  data-loss bug lived exactly there.
- Verify UX flows through the **real entry point** users run (e.g. an actual `parler mcp` restart),
  not a lighter stand-in that merely connects.
- Security tests are negative assertions: wrong token → 401, no id leak, cursor unchanged.
- A test that needs an empty inbox drains the joiner's own "joined" announce first.

## Effort & token discipline (for agents)

Spend tokens reading the code you're changing, not on ceremony:

- Read the slice you need, not the whole file; don't re-read what you already saw unchanged.
- Batch independent reads in one go — but never batch an `Edit` with the `Read` that authorizes it,
  and re-read the exact region right before an `Edit` across a context boundary.
- Run the one targeted test while iterating; the full gate once at the end.
- Don't paste long build/test output into notes or replies — extract the failing line.
- One focused subagent per task, if your harness has them; don't re-derive what's already
  established in the conversation.
- **No-progress guard:** the same failure surviving two fix attempts means stop, append the finding
  to `tasks/lessons.md`, mark the item `[BLOCKED]`, and surface it. Thrashing burns tokens and
  buries the signal.

## Definition of done

Every box, no exceptions:

- [ ] Root cause fixed; no temporary patch left behind
- [ ] Smallest coherent diff; surrounding style matched by hand; no `cargo fmt`
- [ ] New behavior has a test that fails without the change; security behavior has negative tests
- [ ] `make ci` green — build, clippy, tests, doc, audit
- [ ] Protocol ripple handled across all crates, if the wire changed
- [ ] Docs updated in the same change; lesson appended if you were corrected or surprised
- [ ] Self-reviewed against [`code-review-guidelines.md`](code-review-guidelines.md)
