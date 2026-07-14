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

- **`discover` == registered cards only, not "who's connected":** the hub's `discover`/`/api/directory`
  query is `FROM directory d JOIN agents a` — an agent that connects (Hello upserts the `agents` row +
  presence) but never `register`s a card is **invisible** to discovery. So any "watch an agent come
  online" UX (the CLI `connect --verify`, the desktop dial-in list, even the desktop Agents screen)
  needs the agent to have a card. Root-cause fix chosen: `parler mcp` now `auto_register`s a private
  (same-hub) card on connect, so "connected" means "discoverable". If you build presence-style UX,
  don't assume a bare connection lists — either read the card or make the agent self-list.

- **`toml_edit` index-assignment on a *fresh* doc makes an empty inline table:** `doc["mcp_servers"]["parler"]
  = item` on a `DocumentMut` that has no `mcp_servers` yet renders `mcp_servers = {}` and **drops the
  entry** — a silent data loss that only bit the first-time-Codex path (a seeded config round-tripped
  fine, which is why the existing test missed it). Materialize the parent as a real implicit table
  first: `doc.entry("mcp_servers").or_insert(Item::Table({ let mut t=Table::new(); t.set_implicit(true); t }))`,
  then index into that. Test the empty-file path, not just the merge-into-existing path.

- **Verify a UX loop end-to-end with the *real* entry point, not a proxy:** `connect --verify` looked
  done against a `parler presence` stand-in, but presence doesn't register a card, so the real
  `parler mcp` restart is what had to be simulated — that's what surfaced the "connected ≠ discoverable"
  gap. When a feature waits on a side effect of *how users actually run the thing* (here: the wired
  agent runs `parler mcp`), drive that exact binary in the verification, not a lighter command that
  merely connects.

- **Sign only fields the hub doesn't rewrite:** the hub `normalize_mentions()`-es `mentions` in flight
  but stores `reply_to`/`parts` verbatim. A signature must cover the verbatim fields and **exclude the
  normalized ones**, or it fails verification on the receive side for messages the hub legitimately
  touched. (Why `canonical_message_bytes` covers parts/target/replyTo/ts/uid but not mentions.)

- **Verify an audit agent's "critical" against the source before acting on it:** a full-app audit's
  headline CRITICAL ("panics on network input", `parler-protocol/src/hub.rs:862/868…`) was **test
  code** — `panic!("expected register")` inside `#[test] fn visibility_defaults_to_private()`,
  unreachable from the network. Explore/subagent audits routinely inflate severity and mistake
  `#[cfg(test)]` panics for production paths; read the cited line yourself. One false headline
  discredits an otherwise-solid report. (2026-07-03 audit.)

- **On-disk secrets: temp-file + rename, never `write`-then-`chmod`:** `fs::write` creates at the
  default umask (~`0644`) and a later `set_permissions(0o600)` leaves a window where the nkey seed /
  join secret is world-readable — and on an *overwrite* the new bytes sit under the old file's loose
  perms the whole write. Use `parler_auth::write_private_file` (creates a `0600` temp with
  `create_new`, then atomic `rename`). Same helper for both `config.rs` (seed) and `secret.rs` (join
  secret). Test the property (mode is `0600` immediately; overwriting a `0644` file yields `0600`), not
  just the happy path. (2026-07-03 SEC-1.)

- **Check a dep is already in-tree before adding it (cargo-deny is strict on licenses):**
  `grep '^name = "<dep>"' Cargo.lock` first. `parking_lot` was already transitive, so declaring it a
  direct workspace dep kept the `audit` gate green and only needed **one plain (non-`--locked`)
  `cargo build`** to record the new dependency edge before `make ci` (which builds `--locked`) passes.
  (2026-07-03 W4a.)

- **Flip a CLI default: `match Option`, not `.filter()`, so "absent" ≠ "explicitly off".** When
  turning retention on by default, `args.retention_days.filter(|d| *d > 0)` can't tell "flag omitted"
  (→ want the default) from "operator passed `0`" (→ want disabled). Use `match args.x { None =>
  default, Some(0) => None, Some(n) => Some(n) }`. (2026-07-03 W3a — the deployed hub now prunes by
  default; an operator opts out with an explicit `0`/negative.)

- **Re-Read the exact region right before an Edit across a turn/context boundary.** An earlier Read
  (especially an offset/partial read) may no longer authorize a later Edit — the tool errors "File has
  not been read yet." Re-Read the few lines you're about to change immediately before editing rather
  than relying on a Read from a prior turn. (Hit twice wiring Wave 2 in `mcp.rs`/`lib.rs`.)

- **A process-`env` opt-out can't be asserted from a *parallel* test — extract the decision as a pure
  fn.** Testing `PARLER_MCP_VERBOSE=1` by `set_var` then driving the hub flow races with every other
  test in the binary (cargo runs them in threads, one process): a concurrent test reading the same
  global env could see verbose=on and lose its "more waiting" assertion. Fix: pull the branch into a
  pure `recv_limit(explicit, re_read, verbose) -> Option<u32>` and unit-test *that* (deterministic,
  no env), keeping the env read as a thin one-liner at the call site. Only mutate global env in a test
  when the key is uniquely named and never read by a parallel test. (2026-07-03 P0.4.)

- **Clippy `-D warnings` flags single-arg `concat!` and `&"…".repeat(n)`:** a one-argument `concat!(...)`
  trips `clippy::useless_concat` (use a plain string literal), and `Part::text(&"z".repeat(n))` trips
  `needless_borrows_for_generic_args` because `text` takes `impl Into<String>` (drop the `&`). Both
  only surface under the gate's `-D warnings`, not a plain `cargo test` of a single test. Run the full
  `scripts/verify.sh --rust-only` (which runs clippy on `--all-targets`) after adding *test* code, not
  just the one test — new test helpers get linted too. (2026-07-03 P0.4/P1.1.)

- **Server-side long-poll parks in `handle_socket`, not `dispatch` — and the notify-then-check order is
  load-bearing.** `dispatch` is synchronous (the store never blocks across an await), so a parked
  `Pull { wait_secs }` is intercepted *before* `dispatch` in the WS text-frame arm and served by an async
  `waited_pull` that re-runs the plain synchronous `store.pull` on each wakeup — the store lock is never
  held across the await (a `parking_lot::Mutex` guard held across `.await` would deadlock the runtime).
  The park loop must **arm `notify.notified()` first, then re-check the store, then await**, or a `Send`
  that lands between the check and the await is a lost wakeup (the timer would still bound it, but the
  point is early completion). The test writer must call `state.notify_room()` itself — appending straight
  to the store bypasses the real `Send`→`fanout`→`notify_room` path, so a parked waiter never wakes and
  the test hangs to timeout. An empty `store.pull` never advances the cursor (`new_cursor > cur` guard),
  so repeated empty re-checks are harmless and the wait resolves through normal Pull/cursor semantics.
  (2026-07-04 #90.)

- **A client heartbeat during a long-poll must be timeout-wrapped and bypass the reconnect wrapper.** The
  heartbeat pings via `self.transport.request(Ping)` *directly* inside a `tokio::time::timeout`, not via
  `MeshAgent::request` (whose own reconnect would double-handle). A half-open socket doesn't error — the
  read just never completes — so only the `timeout` elapsing catches it; on elapse (or an outright error)
  the heartbeat calls `reconnect()` to rebuild the transport, and the caller's next op runs on the fresh
  one. The long-poll is chunked into heartbeat-sized parked pulls so the ping runs between chunks. **Test
  the half-open path with a fault-injecting transport that's *armed after setup*, not on its first
  request** — arming the very first request hangs `join()` (a plain `request` with no heartbeat), which
  has no timeout and hangs the whole test. Needs a `with_transport_and_identity` constructor so
  `reconnect()` (which requires an identity) actually fires. (2026-07-04 #87.)

- **Query live subscription state; never cache a startup boolean.** `McpState.push` set once at connect
  goes stale after any reconnect that re-subscribed (or failed to). Drop the cached bool; expose
  `MeshAgent::push_active()` (reads the connector's live `subscribed`) and make `reconnect()` write the
  *actual* re-subscribe result back into `subscribed` (don't `let _ =` it — a failed re-subscribe must
  flip the flag to false). The honest-degraded-mode note is a pure decision (`degraded_wait(empty, waited,
  push)`), unit-tested without a hub; with server-side wait it's `false` against any current hub (the note
  is reserved for a genuinely-old, no-push hub). Watch the `TOOL_DESC`/`TOOL_SPECS` byte budgets when
  documenting a new tool arg — 25 B of schema prose failed `tool_specs_stay_lean`; trim the description to
  fit. (2026-07-04 #87.)

- **A capped MCP render is lossless *because* a limited `Pull` advances the cursor only through its
  returned batch** (`store.rs`: `new_cursor = raws.last().seq`, updated only when `since.is_none()`).
  So `parler_recv` default-limit 30 / auto-pull 10 lose nothing — the remainder stays unread for the
  next call. The invariants that keep this true: never budget/cap an explicit-`since` re-read (it's the
  documented full-detail path and must not advance the cursor), and the "more waiting" hint keys off the
  *raw* pull length (`msgs.len() >= limit`), not the post-filter count. Digest joins rely on the same
  thing: `join_session` keeps `pull(None, None)` (advances past the whole backlog) and only changes the
  *render* to a seed+tail digest — cursor and render are decoupled. (2026-07-03 P0.3/P0.4.)

- **clap `env=` participates in `conflicts_with`, so it can break a sibling flag:** adding
  `#[arg(long, env = "PARLER_HUB")]` to `--hub` (which `conflicts_with_all` `--local`/`--team`/
  `--shared`) made an *exported* `PARLER_HUB` count as "`--hub` is present" — so anyone with the var
  in their shell got `error: '--local' cannot be used with '--hub'` on `parler connect --local`.
  clap can't tell an env-sourced value from a typed flag for conflict checks. Fix: when a flag has a
  `conflicts_with`, DON'T give it `env=`; read the env var by hand in the command (here: only when no
  hub-mode flag is given) so the flag still wins and the env only fills a bare run. Test both: the
  bare-run one-liner resolves via env, AND `--local` + exported env still picks `--local`. (2026-07-04
  #100.) Real CLI-behavior regression only a `parler connect --local` run under an exported env surfaces.

- **Factor CLI env/precedence decisions into a pure fn; keep the env read a one-liner at the call
  site:** #99/#100/#101/#102 all had a "resolve X from env > config > default" or "reuse vs mint"
  choice. Testing them by `set_var` races other threads on process-global env (cargo runs tests in
  one process). Extracting `apply_overrides`, `resolve_connect_hub(HubInputs)`, `pick_team_secret`,
  `start_hub_hint` made each deterministic and dodged the `too_many_arguments` clippy gate (group the
  inputs in a struct once you pass ~7). The thin env-reading wrapper stays untested-but-trivial.
  (2026-07-04.)

- **A commit deferred to "the next call" strands every caller that never makes a next call:** the #85
  deferred-ack model (pull → ack rides the *next* pull) kept `members.cursor` at 0 for every one-shot
  process — each `parler recv`/`session join` CLI invocation and every MCP cold start does exactly one
  pull and exits with the ack in an in-memory map, so recv re-read the whole history forever and unread
  counts lied. The e2e suite masked it because `reconnect_resumes_from_durable_cursor` pulls twice
  before dropping the connection. Fix idiom: an ack-only `Pull { since: None, limit: Some(0), ack }` is
  a pure cursor commit (store applies `ack` before the read); call `commit_reads` at every consumption
  boundary (batch rendered / returned to the host) and audit **all** pull call sites for one-shot-ness
  (long-lived loops self-ack; `--since`/`--all` re-reads stay pure). When testing durability, model the
  real process shape — one pull, then process exit — not a convenient two-pull session. (2026-07-09
  E2E audit; the HIGH finding.)

- **`scripts/verify.sh --rust-only` does not run `cargo doc`; `make ci` does:** a public item's doc
  comment intra-doc-linking a private method (`[`commit_reads`]` → private `request`) passes verify.sh
  but fails `make ci`'s `cargo doc -D warnings` gate. Run `CI_SKIP_WEB=1 make ci` before calling a task
  done even when verify.sh is green, and don't bracket-link private items from public docs. (2026-07-09.)

- **A *dangling* intra-doc link (to a removed/renamed item) also only fails `cargo doc -D warnings`, not
  verify.sh:** when I moved `ServerFrame::from_error` out of `parler-protocol`, a `[`ServerFrame::from_error`]`
  in a *sibling* doc comment became a broken link that verify.sh happily passed and `make ci` failed. After
  removing/renaming any item, `grep -rn '<old_name>' crates/**/*.rs` for lingering `[`…`]` references in doc
  comments. (2026-07-10, ACP borrows.)

- **Adding a field to a struct-enum variant ripples to *every* construction AND pattern-match site, in
  every crate:** `ServerFrame::Error { message }` → `{ message, code }` broke ~20 hub construction sites +
  client.rs/lib.rs match arms across three crates. Keep it terse and additive: give the variant serde
  `#[serde(default, skip_serializing_if=...)]` on the new field, add `error()`/`error_coded()` constructor
  helpers so call sites don't spell the field, and use `{ field, .. }` in every *match* so old patterns
  compile. `grep -rn 'Variant::Name' crates/` workspace-wide before assuming you found them all. (2026-07-10.)

- **`parler-protocol` is std-only (no `anyhow`) — keep it that way:** an `anyhow::Error → ServerFrame`
  mapper I first wrote as `ServerFrame::from_error(&anyhow::Error)` didn't compile there. A shared coded
  error type belongs in protocol as a hand-rolled `std::error::Error` (`CodedError`, `Display == message`
  so `.to_string()`/downcast both work); the `anyhow`-flavored projection (`downcast_ref::<CodedError>()`
  → frame) lives in the hub crate, which *does* depend on anyhow. Test the protocol half with
  `Box<dyn std::error::Error>` downcast, the anyhow half in a crate that has anyhow (the connector e2e).
  (2026-07-10.)

- **Every new MCP tool costs ~900 B of `TOOL_SPECS_BUDGET` — it's a bloat guard, not a hard cap:** the
  27th tool (`parler_task`) overshot the 13,200 B ceiling no matter how lean, because a whole tool's
  schema is inherently ~900 B. Right move: trim the *new* tool hard (drop schema-prose descriptions where
  the name is self-evident), then raise the ceiling by the minimum with a one-line reason in the budget
  doc-comment (the comment records every prior raise — follow the pattern). A new *capability* is a
  legitimate cost; the guardrail scales with tool count. (2026-07-10.)

- **clippy `needless_borrows_for_generic_args` also fires on borrowing a *method-call temporary*:**
  `serde_json::to_value(&minimal.to_part())` trips it (pass the owned temporary: `to_value(minimal.to_part())`),
  same family as the `&"…".repeat(n)` case. Only surfaces under `-D warnings` on `--all-targets`, so run the
  full gate after adding *test* code, not just the one test. (2026-07-10.)

- **MCP tool "trim" = diet the descriptions, not retire tools (unless told + given usage data):** the
  `tools/list` payload is a permanent per-session context cost, and it's dominated by *description* prose,
  not schema structure. A description diet (drop verbosity, keep the load-bearing steering like "PLAN/LOG
  reflex", read-after-write, truncation/re-read semantics) cut 27 tools 5,190→4,297 B desc / 13,908→12,727 B
  specs with **zero capability loss** — enough to absorb a newly-added tool *and* net below baseline, so the
  budgets came *down* (13,200→13,000, 5,000→4,600). Retiring/merging a tool removes a public name → breaking
  for MCP hosts + doc churn, so it's gated on a deliberate call + real usage data (backlog). Measure per-tool
  bytes first (a throwaway test serializing each spec) so the audit is data-driven, not vibes. (2026-07-10.)

- **`ServerFrame::error_coded(CODE, msg)` beats string-encoding a code into the message:** to make a wire
  error carry a machine-readable classifier without a store-wide typed-error refactor, code the *frame-
  construction* sites structurally and, for store-origin `bail!`s, raise a downcastable `CodedError` (via a
  local `coded()` helper) that survives `?` to a single `error_frame()` projection at the reply boundary.
  Structural, non-fragile, centralized — not a substring match on the human message (which the repo forbids
  as hacky). The client reconstructs `CodedError::from_wire(code, message)` so callers `downcast` one type.
  (2026-07-10, ACP error codes.)

- **Identity isolation must cover agent-run CLI commands, not only the MCP child:** the live room behind
  `parler://127.0.0.1:7099/join/CQXL5SJN` had one member named `probe` and invite `uses=0`: both agent
  terminals had executed through the same flat `~/.parler/config.json`, while only `parler mcp` applied
  workspace scoping. Scope agent-hosted CLI/hook commands too, include a stable host session id when
  available for same-directory terminals, and preserve ordinary human `parler init` behavior. A full
  `/join/<code>` link must be parsed *before dialing* so its hub is not discarded. (2026-07-12.)

- **Markdown backticks inside a double-quoted shell search are command substitution:** a diagnostic
  `rg` pattern containing `` `parler work` `` accidentally invoked the installed `parler` binary.
  Put regex/search patterns in shell single quotes (or remove the backticks) before passing them to
  `exec_command`; never let documentation punctuation become executable shell syntax. (2026-07-13.)

- **Unsigned routing metadata cannot authorize agent execution:** `mentions` are normalized by the
  hub and excluded from message signatures, so a worker must not use them as the gate for a
  workspace-writing turn. Require a signed addressed `HandoffRef`, or an explicit trusted-room
  `--all-messages` opt-in. (2026-07-13.)
