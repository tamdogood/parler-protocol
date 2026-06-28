# Lessons — the loop's self-improvement memory

Append a short rule here after **any** correction, surprise, or hard-won discovery. Each rule should
be specific enough to prevent the same mistake next time. Reviewed at the start of every loop
iteration (the `/work-next` command reads this file first). Newest at the bottom is fine.

Format: `- **<short trigger>:** <the rule>. <why, in a clause>`

---

- **Never `cargo fmt`:** this repo is hand-formatted; a repo-wide `cargo fmt` rewrites every file and
  buries the real diff. Format new code to match the surrounding style by hand. (Confirmed standing
  preference — see CLAUDE.md / project memory.)

- **CI denies warnings:** `RUSTFLAGS=-D warnings` and `clippy -- -D warnings`. A warning fails the
  gate. `scripts/verify.sh` already sets this — don't relax it to make something "pass".

- **`auth_live` self-skips:** the `parler-auth` `auth_live` test needs a vendored `nats-server`; it
  self-skips when the broker is absent, so a green `cargo test` without it is expected, not a miss.

- **`quick_check` runs on the writer:** FTS5 index validation needs write access, so the store's
  integrity check must use the writer connection, not a read pool member. (Caught by the file-backed
  pool test.)

- **Additive / backward-compatible only:** the hub is deployed live (parler-hub.fly.dev) and old
  clients are in the wild. New frames/extension kinds are fine; non-additive wire changes need a human
  sign-off (they live in the backlog's Icebox).

- **`web/` is human-driven, out of the loop:** the autonomous loop gates with `scripts/verify.sh
  --rust-only` and never edits `web/` or runs the web build — Tam handles the site by hand. Loop items
  are Rust/CLI/protocol only; leave a `[HUMAN] web: …` note for any UI part. (Also keeps the loop off the
  disk-constrained `npm ci` path.)

- **`todo.md` is a log, `backlog.md` is the queue:** pull work from `tasks/backlog.md`; write the
  finished-work summary into `tasks/todo.md`. Don't mine the stale pre-pivot sections of `todo.md` for
  work — those crates were deleted (cc686ea).

- **A joiner's own "joined" announce sits past its own cursor:** `join_session` posts "X joined"
  *after* its catch-up pull, so the very next `pull`/`parler_recv` returns that own message. When
  writing a test that needs an *empty* inbox (e.g. to exercise the `parler_recv wait_secs` long-poll),
  drain it with a throwaway `parler_recv` first — otherwise the initial pull is non-empty and
  short-circuits the wait. (Caught by `recv_wait_secs_long_polls_for_a_push`.)

- **Push is a latency layer, never a delivery guarantee:** real-time `Delivery` pushes are best-effort
  and in-memory (full/closed channel → drop); the per-(room,agent) cursor stays the source of truth, so
  a push must never advance the cursor and a missed push is always recovered by `Pull`. Keep that
  invariant if you touch `fanout`/`next_delivery` — it's what makes push additive + crash-safe.
