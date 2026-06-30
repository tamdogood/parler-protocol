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

- **A new access gate is only as strong as every path to `add_member`:** when adding the session
  join-approval gate, the gate itself (redeem → pending → owner approves) was correct, but the
  pre-existing `Invite` handler auto-joined its minter via `add_member` on *any* room — so a non-member
  could `Invite{room:"<topic>"}` to a known/guessable topic-named session room and self-join, reading
  the seeded context without the key or approval. `token()` is idempotent for safe strings, so a
  `--topic` room name round-trips exactly. Fix: in `Invite`, refuse the self-join when the room already
  exists and the caller isn't a member. **Rule: when you gate membership, audit *all* writers of the
  `members` table (Invite, Redeem, Serve, resolve_target's DM/Service), not just the new one** — a
  bypass elsewhere makes the gate cosmetic. (Caught by writing the "verify the security" step as a real
  threat-model pass, not a formality.)

- **SQLite `cache_size` is per-connection:** the WAL reader pool opens 1 writer + up to 8 readers, so a
  generous per-connection `cache_size` (was `-65536` = 64 MiB) silently multiplies by the pool → ~576 MiB
  of resident page cache that fills the longer the hub runs. Budget *one total* and divide by the
  connection count in `Store::open` (see `TOTAL_CACHE_KIB`). Same trap applies if more pooled
  connections are ever added — re-divide, don't re-add. (Root cause of the "parler eats memory" report.)

- **A new *read* gate needs the same audit as a write gate — and a separate capability from the key it
  sits beside:** issue #43 wanted "paste your session code → read the chat on the web." But a
  session/join key is *approval-gated* (redeem only requests; can't read the backlog), so reading room
  contents straight from it over the public REST API would silently defeat that gate (a glimpsed key →
  full transcript). Fix: a distinct, owner-only, room-scoped, read-only, expiring **watch token** (a
  new capability), not a new use of the key. Reusing the `directory_tokens` table forced **tightening
  `validate_directory_token` to `scope='hub'`** so the two token kinds can't be replayed for each other
  (same table ⇒ scope is the wall; check it both ways). The viewer read path uses a **pure read**
  (`room_messages`), never `pull` — a non-member viewer must not advance any agent's cursor. Rule: when
  exposing data to a *new audience* (a browser, not an agent), enumerate exactly what the new capability
  reaches, and prove with **negative-assertion tests** (join key → 401, no id leak, cursor unchanged)
  that it can't reach anything else or mutate state. (The read-side twin of the `add_member` audit.)

- **`web/` is human-driven *for the autonomous loop* — a direct user request overrides that:** when Tam
  explicitly asks for a website feature, build and verify `web/` too (`npm ci && npm run build`). The
  "leave a `[HUMAN] web:` note" rule is only for the unattended `/work-next` loop.

- **Adding a crate dep needs a non-`--locked` build first:** `scripts/verify.sh` builds `--locked`, which
  *refuses* to add a new dependency edge to `Cargo.lock` ("cannot update the lock file because --locked
  was passed"). After adding `foo = { workspace = true }` to a crate's `Cargo.toml`, run a plain
  `cargo build -p <crate>` once to record the edge, then the gate passes. (Hit when adding `uuid` to
  `parler-connector`.)

- **Don't batch an `Edit` with the `Read` that authorizes it:** an `Edit` to a file I hadn't Read yet
  fails ("File has not been read yet"), and if it's in the same parallel block as an unrelated `Read`
  that *succeeds*, the failure is easy to miss — here the `uuid` Cargo.toml edit silently no-op'd and
  surfaced as a confusing `unresolved crate uuid` two steps later. Read a file, *then* edit it; don't
  parallelize the two.

- **Extension parts are the additive-feature idiom — use them before touching the hub:** a new
  `com.parler.<x>` [`Part::Extension`] rides inside `parts`, which the hub already persists + returns
  verbatim, so a feature can ship with **zero hub/protocol/schema change and work against the deployed
  hub** (code-handoff did it with `com.parler.bundle`; message signing does it with `com.parler.sig`).
  Reach for a first-class frame field only when the hub itself must act on the data.

- **Sign only fields the hub doesn't rewrite:** the hub `normalize_mentions()`-es `mentions` in flight
  but stores `reply_to`/`parts` verbatim. A signature must cover the verbatim fields and **exclude the
  normalized ones**, or it fails verification on the receive side for messages the hub legitimately
  touched. (Why `canonical_message_bytes` covers parts/target/replyTo/ts/uid but not mentions.)
