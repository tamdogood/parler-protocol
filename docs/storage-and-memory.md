# Storage & Memory — design, audit, and roadmap

> The reader-friendly tour of this design (one writer, the read-only WAL pool, the pragma set, and the
> retention janitor) is the blog post [Running SQLite as a server for a multi-agent
> hub](https://www.parlerprotocol.com/blog/sqlite-as-a-server-agent-hub). This file is the full audit
> behind it.

*How the hub records agent traffic and memory, whether it scales as the public hub grows, and where
semantic / vector search fits. Audit date: **2026-06-28**, against `crates/parler-hub/src/store.rs`
(rusqlite 0.31 `bundled`, SQLite 3.45) + `server.rs`. Updated **2026-07-04** to match the shipped P1/P2
retention + read-pool work (see "Implementation status" below) — the original audit narrative is kept
for context but no longer describes the current defaults.*

---

## TL;DR — the verdict

* **Correctness & corruption-safety: PASS.** Messages are recorded atomically against a monotonic
  per-hub `seq`, retrieved by a covered index, and resumed via per-`(agent, room)` cursors. WAL +
  `busy_timeout` + single-writer discipline means the database is **not** exposed to the classic
  SQLite corruption traps (torn writes, multi-writer races, FTS desync). There is **no known
  corruption bug today.**
* **Scalability: the two original gaps are both closed.**
  1. *Throughput* — the store now splits **one dedicated writer connection** from **a pool of
     read-only WAL connections** (`Store::w()`/`Store::r()`; sized to `available_parallelism().clamp(1,
     8)`, falling back to the writer for in-memory DBs), and every call runs off the async runtime via
     `spawn_blocking`. Hot reads (`recall`/`discover`/`is_member`/`roster`/`rooms_of`/`pull`'s backlog
     read) fan out across the pool; only the cursor advance and all writes stay on the single writer.
  2. *Unbounded growth* — retention now ships **on by default**: messages older than 30 days are
     pruned (always keeping the newest 10,000 per room), unkeyed facts are capped at 500 per
     `(author, room)`, and blobs untouched for 14 days are GC'd, all via an hourly background janitor.
     An operator opts out of any one knob with an explicit `0` (or a negative `--keep-facts`) — see
     §3.4 and the flags in `crates/parler-hub/src/main.rs`.
* **Big code transfers: architected right, two efficiency ceilings.** Code rides **content-addressed
  blobs on disk** (git bundles), not the message log — exactly correct. But uploads are **fully
  buffered in RAM** (no streaming/resume), and blob GC (see above) bounds idle growth but not a burst
  of concurrent large uploads.
* **Vector database: don't build a separate one — and this is shipped, not a proposal.** FTS5/BM25 was
  already the right default; **`sqlite-vec` is now integrated in the *same* file** (`vec_facts`, a
  `vec0` virtual table) and `recall` does **hybrid BM25 + vector search fused with RRF** whenever a
  client supplies an embedding (graceful fallback to pure BM25 otherwise). Embeddings are
  **client-supplied** (agents already have model access) — the hub never calls an embedding API. A
  standalone vector DB (Qdrant/Pinecone/…) would add infra, ops, and a second source of truth for **no**
  benefit at this scale.

Everything below is split into: **what exists** (verified from code), **the audit** (findings +
severity), **recommendations** (concrete pragmas/SQL/Rust), the **agent-memory research** that informs
the memory model, the **vector decision**, and a **phased roadmap**.

## Implementation status (2026-07-04)

Phases **P0–P2 are implemented, tested, and clippy-clean** (44 tests green: 22 hub unit incl. a
file-backed read-pool test, 15 connector e2e, 7 CLI/MCP); the production binary builds in `--release`.
All changes are additive and backward-compatible — an older on-disk DB self-migrates (`add_column_if_missing`),
retention is **on by default** with a per-knob `0`/negative opt-out (see P1 below), and the connection
pool degrades to the historical single connection for in-memory DBs.

| Phase | Status | What landed |
|---|---|---|
| **P0 config & integrity** | ✅ done | Per-connection pragmas (`synchronous=NORMAL`, 64 MiB cache **total, split across the writer + read pool** — see §1.5 — 256 MiB mmap, `temp_store=MEMORY`, `busy_timeout=5s`, `foreign_keys=ON`), `auto_vacuum=INCREMENTAL`, `idx_members_agent`, `Store::quick_check()` |
| **P1 durability & growth** | ✅ done, **retention on by default** (Litestream = opt-in scaffold) | `prune_messages`/`prune_facts`/`gc_blobs`/`sweep_expired`/`incremental_vacuum` + `blobs.last_fetched`; a background **janitor** task (off the runtime via `spawn_blocking`) runs hourly by default and prunes messages older than 30 days (floor: newest 10,000/room kept), unkeyed facts beyond the newest 500 per `(author, room)`, and blobs idle 14+ days — all configurable via `--retention-days`/`--keep-messages-per-room`/`--keep-facts`/`--blob-ttl-days`/`--janitor-interval-secs` (`PARLER_HUB_*` env equivalents), with `0` (or a negative `--keep-facts`) opting a knob out to keep-everything; `deploy/litestream.yml` + deploy docs |
| **P2 concurrency unlock** | ✅ done (S4 deliberately skipped) | One **writer** + a pool of **read-only** WAL connections (`w()`/`r()`); hot reads (`recall`/`discover`/`is_member`/`roster`/`rooms_of`/`pull`'s backlog read/…) fan out across cores; `pull` reads on a reader and advances the cursor on the writer. *S4 (`rooms.last_seq`) intentionally not done — it would tax every `append_message` to speed the infrequent `rooms` listing, whose unread `COUNT(*)` is already index-backed.* |
| **P3 big-blob efficiency** | ◑ partial | Blob **GC + LRU** landed in P1 (`gc_blobs` + `last_fetched`). **Remaining:** chunked/streaming + resumable upload (B1) — an additive protocol change spanning `parler-protocol`/`-hub`/`-connector`/`-cli`; scoped as a focused follow-up (the current single-frame path works to the 25 MiB cap). The `SUM(size)` scan (B3) is left as-is — measured "Low", the `blobs` table is small. |
| **P4 semantic memory** | ✅ done | `sqlite-vec` (vec0 virtual table) integrated; `facts` has `embedding_model` column; `vec_facts` stores client-supplied embeddings; `recall` does hybrid BM25⊕vector via RRF when an embedding is provided (graceful fallback to pure BM25 when absent); dimension pinned at 768 (`VEC_DIMENSION`); `prune_facts` cleans vec_facts in sync; 7 new unit tests; protocol extended with optional `embedding`/`embeddingModel` on Remember and `embedding` on Recall (backward-compatible — old clients unaffected). |

The roadmap table in Part 6 is the original plan; the statuses above supersede it.

---

# Part 1 — What exists today

## 1.1 The schema at a glance

One SQLite file (default `~/.parler/hub.sqlite`, `/data/hub.sqlite` in the Fly container), opened per
connection with the full pragma set in §1.5 (`journal_mode=WAL`, `busy_timeout=5000`,
`synchronous=NORMAL`, a budgeted `cache_size`, `mmap_size=256MiB`, `temp_store=MEMORY`,
`foreign_keys=ON`) and the tables below. FTS5 is compiled in via rusqlite's `bundled` feature (verified:
the `facts_fts` virtual table + `bm25()` recall tests pass).

| Table | Purpose | Key / index | Growth |
|---|---|---|---|
| `agents` | identity (id, name, role, first/last seen) | PK `id` | bounded by #agents |
| `presence` | self-reported status, decays to offline by staleness | PK `agent` | bounded by #agents |
| `rooms` | room name + kind (channel/dm/service) | PK `name` | bounded by #rooms |
| `members` | room membership **+ per-member read `cursor`** | PK `(room, agent)` | bounded by #memberships |
| `messages` | **the per-room message log** | PK `seq` AUTOINCR; UNIQUE `id`; `idx(room, seq)` | **pruned by the janitor** — age 30d default, 10,000/room floor |
| `facts` | memory: keyed/unkeyed text facts | PK `id` | keyed facts self-bound (upsert); **unkeyed facts pruned** — 500 per (author, room) default |
| `facts_fts` | external-content FTS5 over `facts.text`, trigger-synced | fts5 | tracks `facts` |
| `invites` | paste-a-code join tokens | PK `code` | swept on expiry (unconditional, not opt-in) |
| `directory` | one signed AgentCard/agent + denormalized tags/skills | PK `agent`; `idx(visibility)` | bounded by #agents |
| `directory_tokens` | expiring read tokens for private-hub directory | PK `token` | swept on expiry (unconditional, not opt-in) |
| `blobs` | content-addressed blob **metadata** (bytes on disk) | PK `id` = sha256 | **GC'd by the janitor** — 14-day idle TTL default |
| `blob_rooms` | which rooms a blob was handed off to (authz) | PK `(blob, room)` | rows removed alongside their `blobs` row on GC |

## 1.2 The message log — how messages are recorded and retrieved

This is the heart of the "are messages recorded correctly and easy to retrieve" question. The model is
clean:

* **Write** (`append_message`): one `INSERT` into `messages`. The id is a **UUIDv7** (`Uuid::now_v7()`
  — time-ordered, so the random string id is also roughly chronological). `seq` is `INTEGER PRIMARY KEY
  AUTOINCREMENT` — a **monotonic, gap-tolerant, per-hub** counter that is *also the cursor unit*.
  `parts`/`mentions` are stored as JSON `TEXT`. The call returns `(id, seq)`.
* **Read** (`pull`): `WHERE room = ? AND seq > ? ORDER BY seq ASC LIMIT ?`, fully served by
  `idx_messages_room_seq(room, seq)`. The default limit is 200, capped at 1000.
* **Resume** (cursors): each `members` row carries a `cursor`. A cursor-mode `pull` reads everything
  after the member's cursor and then advances it; an explicit `since` re-reads history **without**
  moving the cursor. So a reconnecting agent pulls only what it missed — the token-efficiency property
  the whole design is built around.

**This is correct and well-indexed.** Two properties worth stating explicitly:

* **At-least-once, never lost.** `pull` does the `SELECT` and the cursor `UPDATE` as two separate
  autocommitted statements (no wrapping transaction). Under the single connection they can't interleave
  with another op, but a crash *between* them simply leaves the cursor un-advanced → the agent re-reads
  those messages on reconnect. For a message bus that's the **right** failure mode (re-deliver, never
  drop). Messages themselves are durably committed before `Sent` is returned.
* **`seq` is the contract.** Ordering, cursors, and "unread" counts all key off `seq`, not wall-clock
  `ts` (which is only display/metadata and can be non-monotonic across clients). Good separation.

## 1.3 Memory — facts + FTS5

`remember`/`recall` is a deliberately small, **token-cheap keyword memory**:

* `remember` with a `key` **upserts** within `(author, room, key)` (idempotent updates — keyed facts
  are bounded by the number of distinct keys). Without a key it **appends** at write time; the janitor
  (§3.4) later trims unkeyed facts back to the newest 500 per `(author, room)` by default.
* `facts_fts` is a textbook **external-content FTS5** table kept in sync by `AFTER INSERT/DELETE/UPDATE`
  triggers using the `'delete'` sentinel pattern — the correct, non-duplicating way to do it.
* `recall` runs an FTS5 `MATCH` ordered by **`bm25()`** (lower = better), scoped either to one room or
  to the agent's reachable memory (its own private facts ∪ every room it belongs to). Query terms are
  sanitized to alphanumeric prefix-match tokens OR'd together — injection-safe.

This is a solid lexical memory. Its one *semantic* limitation (synonyms/paraphrase don't match) is the
subject of Part 5.

## 1.4 Big artifacts — content-addressed blobs (the code-handoff path)

The "transmit big messages with code changes" requirement is **already a first-class, well-separated
path** — code does **not** go through the message log:

* A handoff = a **git bundle** → hashed to a **content id** (sha256) → stored as **bytes on disk** at
  `<blob_dir>/<id>`, with only *metadata* (`id, media_type, size, created`) in the `blobs` table and an
  authz binding in `blob_rooms`. The room message just carries a small `Part::Extension { kind:
  "com.parler.bundle", … }` reference. (Full spec: [`code-handoff.md`](./code-handoff.md).)
* **Bytes move as WebSocket binary frames** over the already-authenticated socket — no second HTTP
  channel, no capability tokens. The WS message/frame size cap is correctly raised to
  `max_blob_bytes + 1 MiB`, so a 25 MiB bundle actually fits.
* **Content-addressing dedups**: identical bytes → one disk file + one `blobs` row, bound to N rooms.
* **I/O is off the async runtime**: hashing + file write (`finish_blob_upload`) and the download read
  both run on `spawn_blocking`, so a 25 MiB transfer never stalls a tokio worker.
* **Text is capped at 1 MiB** (`max_message_bytes`) precisely to force code onto the blob path. Defense
  is in place: 25 MiB/blob cap, 1 GiB total disk budget, 120 blobs/hour rate limit, sha256 + size
  verified on receipt.

Architecturally this is the right call (keep big BLOBs out of SQLite; let git pack the delta). The
efficiency ceilings are in §2.3.

## 1.5 Concurrency & durability model

* **One writer + a pooled read-only WAL connections.** `Store::open` opens a dedicated writer
  connection plus, for a file-backed DB, a pool of read-only connections sized to
  `available_parallelism().clamp(1, 8)` (an in-memory DB can't share a file across connections, so it
  falls back to the writer for both roles — the historical single-connection behavior, preserved for
  tests). `Store::w()` locks the writer for every mutation (including the cursor advance inside
  `pull`); `Store::r()` round-robins a pooled reader for pure reads (`recall`/`discover`/`is_member`/
  `roster`/`rooms_of`/`pull`'s backlog read). Every guard is a `parking_lot::MutexGuard` that is
  **never** held across `.await` (verified) — no async deadlock — and every call is dispatched via
  `tokio::task::spawn_blocking`, so DB work never runs on a tokio worker thread.
* **`cache_size` is a *total* budget, not per connection.** SQLite's `cache_size` pragma is
  per-connection, so a fixed value would multiply by `1 + n_readers`. `Store::open` instead divides one
  64 MiB total (`TOTAL_CACHE_KIB`) across the writer and every reader before opening them, so the pool's
  summed resident page cache stays bounded regardless of how many cores (hence readers) the host has.
* **Durability:** WAL journal, `busy_timeout=5000`, `synchronous=NORMAL` (the WAL sweet spot — never
  corrupts, only risks the last transaction on power loss), `cache_size` (see above), `mmap_size=256
  MiB`, `temp_store=MEMORY`, `foreign_keys=ON` — all set per connection in `configure_conn`.
* **Deployment:** a single file on a single Fly volume. No replication or streaming backup by default
  (Litestream is an opt-in scaffold — §3.5); `Store::quick_check()` exists for on-demand integrity
  checks but nothing calls it on a schedule yet; no periodic `PRAGMA optimize`/`ANALYZE`.

---

# Part 2 — The audit

*This section is the original 2026-06-28 audit, kept as the historical record of what was found and
why. **S1/S2/S5 (and the single-connection corruption-table rows) have since shipped fixes** — see
the "resolved" notes inline and the Implementation status table up top for current behavior.*

## 2.1 Correctness & corruption-safety — PASS (with hardening notes)

The user's explicit worry is "memory corruption." For SQLite, real corruption comes from a short list
of causes; here is each one and this hub's exposure (as audited 2026-06-28; the single-connection
premise of several rows no longer holds — see the note after the table):

| Corruption cause | Exposure here | Status |
|---|---|---|
| Multiple writers racing without locking | Single process — one writer connection, enforced by construction — physically impossible | ✅ Safe |
| Multiple *processes* on one file (e.g. two Fly instances on one volume) | Possible **only** if you scale the hub to >1 instance on the same volume | ⚠ See §3.5 — keep it single-writer |
| Torn write / power loss | WAL + `synchronous=NORMAL` (shipped, was `FULL`) → atomic commits, no corruption; at most the last txn is lost | ✅ Safe |
| `busy`/lock timeout under contention | `busy_timeout=5000` (shipped, was 3000); one writer means no intra-process writer/writer `SQLITE_BUSY` | ✅ Safe |
| **FTS5 external-content desync** | The fragile one: if `facts` is ever written *outside* the triggers, `facts_fts` rowids drift and `bm25()`/joins corrupt-read | ⚠ Low risk today (all writes go through the triggers, on the single writer connection); add a guard — §3.1 |
| WAL growth / checkpoint starvation | Read pool is read-only WAL connections (no long-lived write-blocking readers); SQLite auto-checkpoints at 1000 pages | ✅ Safe |
| `last_insert_rowid()` on the wrong connection | **Resolved by construction:** `append_message` and every other mutation run only on the single writer connection (`Store::w()`); the read pool (`Store::r()`) is opened read-only, so a write there fails loudly instead of silently reading a stale rowid | ✅ Safe |

**Bottom line:** the database integrity is sound, including after the P2 read-pool change — the pool
was built read-only-by-construction specifically to preserve the `last_insert_rowid()` invariant this
row originally flagged as latent risk.

Two cheap hardening adds still open: ship a `PRAGMA integrity_check`/`PRAGMA quick_check` path on a
schedule (the method `Store::quick_check()` exists but nothing calls it periodically yet — see §1.5),
and keep all fact writes in `store.rs`'s trigger-guarded methods (still true; no known violation).

## 2.2 Scalability findings

*Original severities below are as audited 2026-06-28. S1, S2, and S5 have shipped fixes (P1/P2) and are
marked resolved; S3/S4/S6/S7/S8/S9 reflect the current state.*

| # | Finding | Severity | Why it bites as the hub grows | Fix (→ section) |
|---|---|---|---|---|
| S1 | ~~All reads + writes serialized on one connection, on the async runtime~~ **RESOLVED** | ~~High~~ | Was: throughput capped at one core's worth of serial SQLite, blocking a tokio worker per query | **Shipped:** 1 writer + N read-only pooled connections, all dispatched via `spawn_blocking` (§1.5, §3.2) |
| S2 | ~~`messages` / `facts` grow unbounded~~ (no retention) **RESOLVED** | ~~High~~ | Was: a public hub is append-only forever → DB file and page cache grow without limit | **Shipped:** retention on by default — 30-day/10k-per-room message prune, 500-per-`(author,room)` unkeyed fact cap, hourly janitor (§3.4) |
| S3 | **Missing `members(agent)` index** | Resolved | `members` PK is `(room, agent)`; "all rooms of an agent" (`rooms_of`, and the `recall` `room IN (SELECT … WHERE agent=?)` subquery) can't use the PK prefix → full scan of `members`, growing with total memberships | **Shipped:** `idx_members_agent` (§3.3, Appendix A) |
| S4 | **`rooms_of` counts unread with a correlated `COUNT(*)` per room** | **Medium** | `(SELECT COUNT(*) FROM messages WHERE room=? AND seq>cursor)` is a range scan **per room** on every `rooms` call; cost grows with log size × rooms | Cache a per-room `max(seq)` and compute `unread = max_seq − cursor` (§3.3) — deliberately not done yet, see the P2 row in Implementation status |
| S5 | ~~`synchronous=FULL` (default) + no `cache_size`/`mmap`/`temp_store`~~ **RESOLVED** | ~~Medium~~ | Was: leaves ~10-100x write throughput on the table vs the WAL sweet spot | **Shipped:** the full pragma set (§3.1), with `cache_size` budgeted as one total split across the pool (§1.5) |
| S6 | **No backup / replication** (single file, single volume) | **Medium** | A lost/corrupted volume = total loss of all agent history and memory; no PITR | Litestream (stream to S3) or LiteFS (§3.5) — scaffold exists (`deploy/litestream.yml`), not wired as a default |
| S7 | **`messages.id` `UNIQUE` index never read** | Low | An extra btree maintained on every insert (write amplification) with no query using it | Keep only if clients dedup by id; else drop the `UNIQUE` (§3.3) — not done, still open |
| S8 | ~~`invites` / `directory_tokens` never pruned~~ **RESOLVED** | ~~Low~~ | Was: expired rows accumulate; tiny, but unbounded | **Shipped:** `Store::sweep_expired` runs `WHERE expires < now` unconditionally on every janitor pass (§3.4) |
| S9 | **No `ANALYZE` / `PRAGMA optimize`** | Low | Planner stats go stale as distributions shift → worse plans at scale | `PRAGMA optimize` on a timer / shutdown (§3.1) — not done, still open |

## 2.3 Big-message / code-transfer efficiency

The path is correct (§1.4); these are the ceilings for "**efficiently** transmit big code changes":

| # | Finding | Severity | Detail | Fix (→ section) |
|---|---|---|---|---|
| B1 | **Uploads fully buffered in RAM** | **High (at scale)** | A blob arrives as **one** WS binary frame; tungstenite buffers the whole frame, then `finish_blob_upload` holds the entire `Vec<u8>` again. Peak RAM ≈ (concurrent uploads × up to ~26 MiB). No streaming, no **resume** on a dropped 25 MiB transfer | Chunked/streaming upload (§3.6) — not yet done |
| B2 | ~~Blobs never garbage-collected~~ **RESOLVED** | ~~High (at scale)~~ | Was: `total_blob_bytes` only grows; at 1 GiB the hub hard-rejects *all* new handoffs ("storage is full") | **Shipped:** LRU/TTL GC via `last_fetched`, 14-day idle default, hourly janitor (§3.4) |
| B3 | **`SUM(size)` full scan of `blobs` on every `PutBlob`** | Low | The pre-upload budget check aggregates the whole table under the global mutex; grows with #blobs | Maintain a running byte counter (§3.6) |
| B4 | **Orphan files possible** | Low | If `put_blob_meta` fails *after* `std::fs::write`, a disk file exists with no row (and isn't GC'd); a `PutBlob` that never sends bytes leaves no trace (fine) | Periodic disk↔table reconcile (§3.6) |
| B5 | **Budget check is racy** | Low | Two concurrent reservations can both pass `used+size ≤ budget` and jointly exceed it (soft cap; the code intentionally errs toward rejection) | Acceptable; tighten with the counter in B3 |

**Efficiency lever that already exists:** `parler push` bundles a **git revision range** (e.g.
`main..feature`), so the bundle is the *delta*, not the whole repo — agents should hand off ranges, not
full history. Worth documenting as the primary "send big changes efficiently" guidance. The optional
Phase-3 "frontier" index (latest bundle per room) would let a joiner grab just the tip.

---

# Part 3 — Recommendations

*Sections 3.1, 3.2, and 3.4 below are the original proposals — **all have since shipped** (see
Implementation status and §1.5). They're kept as-written for the reasoning; treat the pragma values,
connection split, and retention policy they describe as **current shipped behavior**, not a future
plan.*

## 3.1 SQLite configuration (pragmas) — shipped

Replace the two-line pragma header with the documented server profile. All are runtime-safe and
backward-compatible:

```sql
PRAGMA journal_mode = WAL;        -- already set: readers don't block the writer
PRAGMA busy_timeout = 5000;       -- 3000 → 5000ms; ride out checkpoint/contention
PRAGMA synchronous  = NORMAL;     -- NEW: WAL sweet spot — never corrupts, only risks the last txn
                                  --      on power loss; ~10–100× write throughput vs FULL
PRAGMA cache_size   = -65536;     -- NEW: 64 MiB page cache (negative = KiB), fewer disk reads as data grows
PRAGMA temp_store   = MEMORY;     -- NEW: sorts/temp b-trees in RAM (helps ORDER BY / FTS)
PRAGMA mmap_size    = 268435456;  -- NEW: 256 MiB memory-mapped I/O for reads
PRAGMA foreign_keys = ON;         -- NEW: enforce referential integrity once FKs/retention land
PRAGMA wal_autocheckpoint = 1000; -- explicit default; keep the WAL bounded
```

Run `PRAGMA optimize;` periodically (e.g. every few hours and on graceful shutdown) so the planner keeps
good stats. Expose `PRAGMA quick_check` on `/health` (or a `--check` flag) so corruption is *detected*,
not discovered. `synchronous=NORMAL` is the single highest-value line — it is explicitly the
"never corrupts the database" WAL setting, trading only a possible loss of the **last** transaction on
power loss for a large write speedup. (Sources: SQLite WAL docs; Kerkour; oneuptime; PowerSync.)

## 3.2 Connection architecture — the scalability unlock (S1) — shipped

Then: one `Mutex<Connection>` for everything, on the runtime. Now shipped (`Store::w()`/`Store::r()`
in `store.rs`): the idiomatic **"SQLite for servers"** pattern, which WAL is built for:

* **One dedicated writer connection** (keep the serialization — SQLite is single-writer anyway), and
* **A small pool of read-only connections** (e.g. `N = num_cpus`), since **WAL readers run concurrently
  with the writer and with each other**, and
* **Run the blocking rusqlite calls off the async runtime** — either `tokio::task::spawn_blocking` or a
  dedicated DB thread/`rayon` pool — so a slow query never starves tokio workers.

This turns "one serial core for the whole hub" into "writes serialized (fine — single-writer is
SQLite's model) + reads scale across cores," which is exactly where the read-heavy hub traffic
(`pull`, `recall`, `discover`, `roster`, `rooms`) wants to be.

> **Corruption guard when you do this:** `append_message`'s `last_insert_rowid()` is correct **only**
> because writes share one connection. Keep **all writes on the single writer connection** (never the
> read pool), or switch to `INSERT … RETURNING seq` so the seq comes from the same statement. This is
> why the pool must be *read-only* + one writer, not a generic N-connection pool.

A pragmatic first step (smaller change, most of the win): wrap the existing synchronous `Store` calls in
`spawn_blocking` at the call sites so DB work leaves the async runtime, then add the read pool. The
`Store` API doesn't have to change.

## 3.3 Indexes & query fixes

* **S3 — add the membership-by-agent index** (one line, big effect on `rooms_of` + `recall`):
  ```sql
  CREATE INDEX IF NOT EXISTS idx_members_agent ON members(agent);
  ```
* **S4 — drop the per-room `COUNT(*)`.** Keep a denormalized `rooms.last_seq` (bump it in
  `append_message`) and compute `unread = max(0, last_seq − cursor)` — O(1) per room instead of a range
  scan. (Or accept S4 until a room's log is large; it's index-backed, just not free.)
* **S7 — reconsider `messages.id UNIQUE`.** Nothing queries `messages` by `id`. If clients don't dedup
  by id, drop `UNIQUE` (keep the column) to save an index write per message. If they do, keep it.
* **Optional** `idx_facts_room`/`idx_facts_author` only if non-FTS scans over `facts` ever appear; today
  FTS narrows first, so skip.

## 3.4 Retention & growth — the "works as it grows" fix (S2, S8, B2) — shipped, on by default

A public hub **must** bound its append-only state. Policy proposal (all configurable) — **shipped as
the default policy** (see `Retention::default()` in `crates/parler-hub/src/server.rs` and the CLI flags
in `crates/parler-hub/src/main.rs`; an operator opts a knob out with an explicit `0`, or a negative
value for `--keep-facts`):

* **Messages** — `Store::prune_messages` deletes rows older than `--retention-days` (default **30**,
  `PARLER_HUB_RETENTION_DAYS`; `0` disables age pruning entirely) **and** beyond the newest
  `--keep-messages-per-room` (default **10,000**, `PARLER_HUB_KEEP_MESSAGES_PER_ROOM`) — both
  conditions must hold, so the per-room floor always protects recent history regardless of age, and a
  room under the floor is never trimmed by age alone. No cursor fix-up is needed: `pull` reads
  `seq > cursor`, so a cursor below a pruned `seq` just resumes at the next surviving row.
* **Facts** — keyed facts are self-bounding (upsert). `Store::prune_facts` caps **unkeyed** facts per
  `(author, room)` at `--keep-facts` (default **500**, `PARLER_HUB_KEEP_FACTS`; a negative value keeps
  all). Deletes flow through the FTS triggers automatically, and orphaned `vec_facts` rows are cleaned
  up in the same call.
* **Blobs (B2)** — `Store::gc_blobs` GC's by **LRU/TTL**: a blob neither created nor fetched within
  `--blob-ttl-days` (default **14**, `PARLER_HUB_BLOB_TTL_DAYS`; `0` disables) has its row **and** disk
  file removed. `last_fetched` is bumped on every download.
* **Expired rows (S8)** — `Store::sweep_expired` runs `DELETE FROM invites/directory_tokens WHERE
  expires < now` on every janitor pass, unconditionally (not a retention knob — expired rows are always
  dead weight).
* **Reclaim space** — `PRAGMA auto_vacuum = INCREMENTAL` is set at migration time, and
  `Store::incremental_vacuum()` runs after every janitor pass, so the file actually shrinks.

All of this runs as a single periodic **janitor** task (`run_janitor` in `server.rs`), on an interval
set by `--janitor-interval-secs` (default **3600**, i.e. hourly, `PARLER_HUB_JANITOR_INTERVAL_SECS`).
The DB work runs via `spawn_blocking` so a large prune never stalls the async runtime; only the
resulting blob file unlinks happen back on the async side.

## 3.5 Durability & backup (S6) and the single-writer rule

* **Litestream** (sidecar, streams the WAL to S3/R2) gives continuous backup + point-in-time restore
  with **zero app changes** — the lowest-effort, highest-value durability win for a single-node hub on
  Fly. **LiteFS** (FUSE, replicated SQLite) is the step up if you later want read replicas.
* **Stay single-writer.** SQLite scales *up* (one big machine) beautifully; it does **not** want two
  hub processes writing one file. If you ever run >1 Fly instance, either (a) pin writes to one
  primary (LiteFS leases) or (b) that's the signal to graduate the *transport+log* to NATS/Postgres
  (the design already anticipates a pluggable `MeshTransport`). Document this as the explicit horizontal
  trigger so nobody points two writers at one volume (the one way to actually corrupt this DB).

## 3.6 Big-blob efficiency (B1, B3, B4)

* **B1 — stream blobs in chunks.** Add a chunked upload (`PutBlob{…, chunks}` → many `BlobChunk{seq,
  bytes}` frames → `BlobCommit`), hashing incrementally and writing to a temp file, then atomic-rename
  to `<id>` on commit. Bounds RAM to one chunk, enables **resume** of a dropped large transfer, and
  lifts the practical artifact-size ceiling. (The current single-frame path can remain for small
  blobs.)
* **B3 — running byte total.** Replace `SUM(size)` per upload with a maintained counter (a one-row
  `meta` table, or `SUM` cached in `HubState`) updated on insert/GC.
* **B4 — reconcile.** The janitor (3.4) also deletes disk files with no `blobs` row and rows with no
  file. Write order is already correct (file then meta); the reconcile closes the crash window.
* **Guidance:** document "hand off a **range** (`git bundle … main..HEAD`), not the whole repo" as the
  primary efficiency practice; consider the Phase-3 frontier index so a joiner fetches just the tip.

---

# Part 4 — Agent-memory research (what should inform the model)

Across 2025–2026 the agent ecosystem converged on a consistent, cognitively-inspired **memory
taxonomy**, and a consistent **retrieval** stack. Summary of the current findings and how Parler Protocol maps:

### The taxonomy everyone converged on
* **Working / context memory** — the live conversation window. In Parler Protocol: the room message log an
  agent `pull`s.
* **Episodic memory** — *what happened* (events, interactions, time-stamped). In Parler Protocol: the
  `messages` log itself is already an episodic store (per-room, `seq`/`ts`-ordered).
* **Semantic memory** — *distilled facts / knowledge*, decoupled from when they were said. In Parler Protocol:
  the `facts` table (`remember`/`recall`).
* **Procedural memory** — *how to do things* (skills, prompts, tool recipes). In Parler Protocol: partially the
  signed AgentCard `skills`; otherwise not yet modeled.

### How the leading frameworks do it (and the lesson for Parler Protocol)
* **Letta / MemGPT** — OS-style tiers: a full **recall** DB of history (beyond the context window) +
  an **archival** semantic tier, with **agent-directed** consolidation (the agent decides what graduates
  from history → long-term). *Lesson:* Parler Protocol already has the "recall DB" (the message log) and an
  archival tier (`facts`); the missing piece is **consolidation** — letting an agent promote salient
  messages into facts.
* **Mem0** — an LLM **extract-then-update** pipeline: pull salient candidates from a conversation, then
  add/update/dedup against existing memories by semantic similarity. Strong on the **LoCoMo** long-
  conversation benchmark. *Lesson:* the highest-leverage memory feature is **automatic salience
  extraction + dedup**, not more storage. This is a *client-side* job (the agent has the LLM); the hub
  just needs to store + retrieve well.
* **Zep / Graphiti** — a **bitemporal knowledge graph** (every edge carries *event time* and *ingestion
  time*), reporting strong Deep-Memory-Retrieval accuracy and low latency. *Lesson:* temporal validity
  ("this fact was true *as of*…") matters for agents that reason over changing state. Parler Protocol's `facts`
  already keep `ts`; a future `valid_from`/`superseded_by` is the cheap nod to bitemporality **without**
  adopting a graph DB.
* **AutoMem (Wu et al., 2026)** — treats **memory management as its own trainable skill** ("metamemory").
  The agent runs a **LOG/PLAN reflex** — after each step, *what is worth recording?*; before each action,
  *what must I recall?* — over a small set of **typed memory files** (`status`, `strategy`, `progress`,
  `knowledge`, …) with first-class memory actions (append, search, keyed upsert). Optimizing *only* the
  memory scaffold, leaving task behavior untouched, yielded **2–4× on long-horizon tasks** and let a 32B
  model rival frontier systems. *Lesson:* the cheap, model-agnostic win is **discipline and structure, not
  more storage** — a stable key vocabulary plus a "record-after / recall-before" habit. Parler Protocol's
  `remember`/`recall` already supply the actions (unkeyed `remember` = append, keyed `remember` = upsert,
  `recall` = search); we now bake the reflex and a typed-key convention into the tool copy itself (see the
  `parler_remember`/`parler_recall` descriptions in `crates/parler-cli/src/mcp.rs`).

### Net guidance for Parler Protocol
1. **Keep the hub a thin, fast store; keep intelligence in the clients.** Extraction, summarization,
   salience, and embedding all belong on the agent side (they have the model). The hub's job is to
   **record correctly and retrieve cheaply** — which it already does well.
2. **The episodic log is an asset, not just a buffer.** With retention (§3.4) it *is* the recall tier.
3. **The near-term memory win is consolidation + hybrid recall**, not a new datastore: let agents
   promote messages → facts, and make `recall` semantic (Part 5).
4. **Add lightweight temporality to facts** (supersede/`valid_from`) before reaching for a graph DB.
   Knowledge-graph memory (Graphiti/Cognee) is powerful but is a *much* larger build; it is not
   warranted yet and would break the low-ops, single-file ethos.
5. **Scaffold metamemory as a client-side habit** (AutoMem). The tool descriptions now nudge a LOG/PLAN
   reflex ("record after a decision, recall before acting") and a small, stable key vocabulary
   (`status`/`strategy`/`progress`/`knowledge`/`session-digest`). This is **purely additive** — keys are
   already free strings, no protocol change — but it turns the flat fact bag into typed, deduped state that
   `recall` can target, which is where AutoMem's 2–4× came from. The heavier follow-ons AutoMem implies —
   an offline "scaffold-evolution" loop (a strong model reviews real mesh transcripts and proposes better
   memory conventions/tool copy, gated on a retrieval metric) and consolidation of episodic notes into
   keyed summaries — line up with our existing loop-engineering harness and the retention work in §3.4.
   Its proficiency/LoRA-training half is **out of scope**: Parler is an MCP client to whatever model the
   user runs; it neither owns nor trains weights.

---

# Part 5 — Should we build a vector database?

**Recommendation (shipped): no separate vector database. `sqlite-vec` is added to the existing file,
and `recall` runs hybrid BM25 + vector search fused with RRF whenever a client supplies an embedding.**

### Why not a dedicated vector DB
A standalone vector DB (Qdrant, Pinecone, Weaviate, Milvus…) would add a network service, ops/HA burden,
a second source of truth to keep consistent with SQLite, and cost — for **no** benefit at this scale.
The whole product thesis is *low-ops, single-file, runs-everywhere*. A separate vector store breaks
exactly that. The threshold where dedicated vector infra earns its keep — **>~10M vectors**, strict
sub-10ms distributed latency, or thousands of concurrent vector writes/sec — is far beyond a chat-style
agent hub's memory.

### Why `sqlite-vec` + hybrid is the right fit
* **`sqlite-vec`** is a single-file, dependency-free **loadable SQLite extension** (the maintained
  successor to `sqlite-vss`, by Alex Garcia) that runs everywhere SQLite does. It stores vectors in a
  `vec0` virtual table and does **brute-force KNN**:
  ```sql
  CREATE VIRTUAL TABLE vec_facts USING vec0(fact_id INTEGER PRIMARY KEY, embedding FLOAT[768]);
  -- KNN:
  SELECT fact_id, distance FROM vec_facts
   WHERE embedding MATCH :query_vec AND k = 20;
  ```
* **Brute force is fine at this scale.** Reported numbers: ~1M × 1024-dim is a few seconds (fine for
  occasional/CLI), and for the dimensions an agent hub would use (384/768) latency is well under ~75 ms
  for hundreds-of-thousands of vectors. Parler Protocol's `facts` are scoped (per agent / per room), so each
  `recall` searches a **small partition**, not the whole corpus — comfortably in brute-force territory.
* **Hybrid > either alone.** BM25 finds exact terms/abbreviations but misses meaning; vectors capture
  meaning but miss rare tokens. The current best practice (Simon Willison / Alex Garcia, and a wave of
  2025–26 local-first agent-memory projects) is to run **both** and fuse with **Reciprocal Rank
  Fusion**:
  ```sql
  -- combine FTS5 rank and vec distance ranks; rrf_k = 60 (standard)
  ( coalesce(1.0/(:rrf_k + fts.rank),  0.0) * :w_fts
  + coalesce(1.0/(:rrf_k + vec.rank),  0.0) * :w_vec ) AS score
  ```
  This keeps the **excellent, cheap BM25** that already exists and *adds* semantic recall on top — best
  of both, in one query, in one file.

### The one real constraint: where do embeddings come from?
The hub is a pure-Rust router with **no ML runtime** and (rightly) no API keys. So don't embed on the
hub. The clean fit, consistent with Part 4's "intelligence in the clients" principle:

* **Clients supply embeddings.** Agents already have model access; extend `Fact` with an optional
  `embedding: Vec<f32>` (+ `embedding_model` id) on `remember`, and let `recall` accept an optional
  query vector. The hub just **stores** the vector in `vec_facts` and does the KNN + RRF. No server-side
  model, no key, nothing on the hot path. (Fallback for clients that don't send a vector: pure BM25 —
  graceful degradation.)
* *Alternative considered:* the hub calls an embedding API. Rejected for the MVP — adds a network
  dependency, latency on `remember`/`recall`, a cost center, and a server-side secret. Revisit only if
  "clients supply embeddings" proves impractical.

### Phasing the vector work
* **Phase 0:** keep FTS5/BM25. It's good, and most recall queries are keyword-shaped. (Superseded by
  Phase 1 below, but pure-BM25 remains the fallback when a client sends no embedding.)
* **Phase 1 — ✅ shipped:** `sqlite-vec` is loaded (statically linked via `sqlite3_vec_init`, registered
  as an auto-extension in `store.rs`); `vec_facts` (a `vec0` virtual table, dimension pinned at 768 via
  `VEC_DIMENSION`) stores client-supplied embeddings; the protocol carries optional `embedding`/
  `embeddingModel` on `Remember` and `embedding` on `Recall`; `recall` runs hybrid BM25 ⊕ vector search
  fused with RRF (`rrf_k = 60`) whenever an embedding is supplied, falling back to pure BM25 otherwise.
  One file, one extension, no new service.
* **Phase 2 (only if a partition ever exceeds brute-force comfort — not yet needed):** partition `vec0`
  by room/author (sqlite-vec supports partition/metadata keys in current versions), or move that tier to
  an ANN extension (`vectorlite`/`usearch`) for approximate search. Still inside SQLite. A dedicated
  vector DB remains unnecessary until the >10M-vector / distributed thresholds above.

---

# Part 6 — Phased roadmap

| Phase | Items | Effort | Risk | Payoff |
|---|---|---|---|---|
| **P0 — config & integrity** (ready: Appendix A) | Pragmas (§3.1: `synchronous=NORMAL`, cache, mmap, temp_store, busy 5s); `idx_members_agent` (S3); `quick_check` on boot; FTS-write guard comment | ~½ day | Very low (additive, backward-compatible) | Big write speedup; fixes the worst missing index; corruption *detection* |
| **P1 — durability & growth** | Litestream backup (S6); janitor task for retention/GC of messages, facts, blobs, expired tokens (S2/S8/B2); `auto_vacuum=INCREMENTAL` | ~2–3 days | Low–med (pruning needs cursor-safe watermarks) | Hub stops growing without bound; survives volume loss |
| **P2 — concurrency unlock** | DB calls off the async runtime (`spawn_blocking`); 1 writer + N read-only connections (§3.2) with the `last_insert_rowid` guard; `rooms.last_seq` to kill the unread `COUNT(*)` (S4) | ~3–5 days | Med (touches the hot path; needs load test) | Reads scale across cores; removes the throughput ceiling |
| **P3 — big-blob efficiency** | Chunked/streaming + resumable upload (B1); running byte counter (B3); disk↔table reconcile (B4); document `git bundle` ranges; optional frontier index | ~3–5 days | Med | Bounded upload RAM, resumable large code handoffs |
| **P4 — semantic memory** | `sqlite-vec` + `vec_facts`; client-supplied embeddings in the protocol; hybrid BM25⊕vector recall via RRF; optional fact temporality (`valid_from`/supersede); optional message→fact consolidation API | ~1–2 wks | Med | Semantic recall; aligns with Mem0/Letta findings |

**Suggested order:** P0 → P1 → P2, then P3/P4 by demand. P0 is pure upside and I can land it on request
as a single small, backward-compatible PR.

---

## Appendix A — Phase-0 diff (shipped)

This landed in `crates/parler-hub/src/store.rs` (`configure_conn`, called per connection — writer and
every reader), with one refinement over the original sketch below: **`cache_size` is a total budget
divided across the pool** (`TOTAL_CACHE_KIB = 65_536` KiB ÷ `1 + n_readers`, floored at
`MIN_CACHE_KIB_PER_CONN`), not a flat `-65536` on every connection — a fixed per-connection value would
have silently multiplied the resident page cache by the pool size.

```sql
PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 5000;            -- was 3000
PRAGMA synchronous  = NORMAL;
PRAGMA cache_size   = -{cache_kib};    -- TOTAL_CACHE_KIB split across writer + readers, not a flat 64 MiB each
PRAGMA temp_store   = MEMORY;
PRAGMA mmap_size    = 268435456;       -- 256 MiB
PRAGMA foreign_keys = ON;
```
…and after the `messages` index:
```sql
CREATE INDEX IF NOT EXISTS idx_members_agent ON members(agent);   -- S3, shipped
```
> Note: these are *connection-level* pragmas, so `configure_conn` runs them on the writer **and** on
> every pooled reader, not only inside the one-time `execute_batch(MIGRATION)`. `Store::quick_check()`
> exists for the boot/`/health` `PRAGMA quick_check` path but isn't wired into a periodic schedule yet.

## Appendix B — Phase-4 vector schema sketch

```sql
-- loaded extension: sqlite-vec (vec0). Dimension pinned to the chosen embedding model.
CREATE VIRTUAL TABLE IF NOT EXISTS vec_facts USING vec0(
  fact_id   INTEGER PRIMARY KEY,   -- == facts.id
  embedding FLOAT[768]
);
-- store the model id alongside facts so dimensions never silently mix:
ALTER TABLE facts ADD COLUMN embedding_model TEXT;   -- NULL = lexical-only fact
```
`recall` becomes: run FTS5 (BM25) **and** `vec_facts` KNN over the same room/author scope, fuse by RRF
(`rrf_k = 60`), return top-k. Clients pass an optional `embedding` on `remember` and an optional query
vector on `recall`; absent either, recall degrades to today's pure BM25.

---

## Sources

Agent memory landscape & frameworks:
- [AI Agent Memory Architectures — Zylos Research](https://zylos.ai/research/2026-04-05-ai-agent-memory-architectures-persistent-knowledge/)
- [Best AI Agent Memory Frameworks in 2026 — Atlan](https://atlan.com/know/best-ai-agent-memory-frameworks-2026/)
- [Survey of AI Agent Memory Frameworks — Graphlit](https://www.graphlit.com/blog/survey-of-ai-agent-memory-frameworks)
- [Agent Memory Techniques (Letta/Mem0/Zep/Graphiti, LoCoMo) — NirDiamant](https://github.com/NirDiamant/Agent_Memory_Techniques)
- [Agent Memory Systems & Knowledge Graphs: Letta, Mem0, Graphiti, Cognee](https://codepointer.substack.com/p/agent-memory-systems-and-knowledge)
- [AutoMem: Automated Learning of Memory as a Cognitive Skill — Wu, Zhu, Zhang, Wang, Yeung-Levy, 2026 (arXiv:2607.01224)](https://arxiv.org/abs/2607.01224)

SQLite + vector / hybrid search:
- [Hybrid full-text + vector search with SQLite — Simon Willison](https://simonwillison.net/2024/Oct/4/hybrid-full-text-search-and-vector-search-with-sqlite/)
- [Hybrid search: FTS5 + sqlite-vec + RRF — Alex Garcia](https://alexgarcia.xyz/blog/2024/sqlite-vec-hybrid-search/index.html)
- [Introducing sqlite-vec v0.1.0 — Alex Garcia](https://alexgarcia.xyz/blog/2024/sqlite-vec-stable-release/index.html)
- [vectorlite (ANN alternative to brute-force)](https://github.com/1yefuwang1/vectorlite)
- [Choosing an embeddable vector DB (sqlite-vec vs alternatives)](https://shaharia.com/blog/choosing-embeddable-vector-database-go-application/)

SQLite at scale / production:
- [Write-Ahead Logging — sqlite.org](https://sqlite.org/wal.html)
- [Optimizing SQLite for servers — Kerkour](https://kerkour.com/sqlite-for-servers)
- [How to Set Up SQLite for Production — oneuptime](https://oneuptime.com/blog/post/2026-02-02-sqlite-production-setup/view)
- [SQLite optimizations for ultra high performance — PowerSync](https://powersync.com/blog/sqlite-optimizations-for-ultra-high-performance)
- [SQLite in production — a real-world benchmark](https://shivekkhurana.com/blog/sqlite-in-production/)
```
