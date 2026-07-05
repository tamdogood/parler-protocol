//! The hub's durable store — embedded SQLite.
//!
//! Holds everything the bus needs to survive a restart: agents, rooms + membership, the per-room
//! message log (keyed by a monotonic `seq` that is also the cursor unit), the full-text `facts`
//! memory, and outstanding invites. Access is serialized through one connection behind a `Mutex`;
//! every method here is synchronous and never held across an `.await`, so the async server can call
//! it directly.

use anyhow::{anyhow, bail, Result};
use parler_protocol::{
    estimate_message_tokens, AgentCard, DirectoryEntry, DiscoverScope, EndpointRef, Fact,
    JoinRequest, Part, RecallHit, RoomInfo, RoomKind, RosterEntry, StoredMessage, Visibility,
};
use rusqlite::{named_params, params, Connection, OpenFlags, OptionalExtension};
use std::collections::HashMap;
use std::ops::Deref;
use std::path::Path;
use parking_lot::{Mutex, MutexGuard};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Once};
use uuid::Uuid;

/// Default embedding dimension for the vec_facts table (pinned at creation time).
pub const VEC_DIMENSION: usize = 768;

/// Register the sqlite-vec extension as an auto-extension (once, before any connection opens).
fn ensure_vec_extension() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        #[allow(clippy::missing_transmute_annotations)]
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
    });
}

const RRF_K: f64 = 60.0;

const MIGRATION: &str = r#"
PRAGMA auto_vacuum = INCREMENTAL;
PRAGMA journal_mode = WAL;

CREATE TABLE IF NOT EXISTS agents (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL,
  role       TEXT,
  first_seen INTEGER NOT NULL,
  last_seen  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS presence (
  agent    TEXT PRIMARY KEY,
  status   TEXT NOT NULL,
  activity TEXT,
  ts       INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS rooms (
  name        TEXT PRIMARY KEY,
  kind        TEXT NOT NULL,
  description TEXT,
  -- The agent that created the room (via its invite). Set-once; it is the only one allowed to
  -- approve/deny pending joins for an approval-gated room. NULL for rooms created before this column
  -- existed or with no distinguished owner (DMs/services).
  owner       TEXT,
  created     INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS members (
  room   TEXT NOT NULL,
  agent  TEXT NOT NULL,
  joined INTEGER NOT NULL,
  cursor INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (room, agent)
);

CREATE TABLE IF NOT EXISTS messages (
  seq         INTEGER PRIMARY KEY AUTOINCREMENT,
  id          TEXT NOT NULL UNIQUE,
  room        TEXT NOT NULL,
  author      TEXT NOT NULL,
  author_name TEXT NOT NULL,
  author_role TEXT,
  parts       TEXT NOT NULL,
  mentions    TEXT,
  reply_to    TEXT,
  ts          INTEGER NOT NULL,
  -- Estimated communication tokens this message carries (see `estimate_message_tokens`), stored at
  -- append time so per-room/per-agent totals are a cheap aggregate. An estimate, not a billed count.
  -- Also added to older DBs via `add_column_if_missing` (which backfills historical rows once).
  tokens      INTEGER NOT NULL DEFAULT 0,
  -- Optional sender-supplied idempotency key (#86). NULL for old clients / unkeyed sends. The partial
  -- unique index below makes a retried send with the same key return the original row instead of
  -- double-posting; NULLs are excluded from the index so unkeyed sends are never deduped.
  client_id   TEXT
);
CREATE INDEX IF NOT EXISTS idx_messages_room_seq ON messages(room, seq);
-- The (room, author, client_id) idempotency index (#86) is created after startup migration, so a DB
-- that predates the client_id column gets the column added first (see Store::open).
-- Retention's age scan (`prune_messages`) filters `WHERE ts < cutoff`; without this it's a full table
-- scan every janitor pass on a large log.
CREATE INDEX IF NOT EXISTS idx_messages_ts ON messages(ts);
-- Membership keyed by agent: `members` PK is (room, agent), so "every room an agent is in" (rooms_of,
-- and the recall room-scope subquery) can't use the PK prefix without this index.
CREATE INDEX IF NOT EXISTS idx_members_agent ON members(agent);

CREATE TABLE IF NOT EXISTS facts (
  id     INTEGER PRIMARY KEY AUTOINCREMENT,
  fkey   TEXT,
  room   TEXT,
  author TEXT NOT NULL,
  text   TEXT NOT NULL,
  ts     INTEGER NOT NULL
);

-- Full-text index over fact text (external-content FTS5, kept in sync by the triggers below).
CREATE VIRTUAL TABLE IF NOT EXISTS facts_fts USING fts5(text, content='facts', content_rowid='id');
CREATE TRIGGER IF NOT EXISTS facts_ai AFTER INSERT ON facts BEGIN
  INSERT INTO facts_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER IF NOT EXISTS facts_ad AFTER DELETE ON facts BEGIN
  INSERT INTO facts_fts(facts_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
CREATE TRIGGER IF NOT EXISTS facts_au AFTER UPDATE ON facts BEGIN
  INSERT INTO facts_fts(facts_fts, rowid, text) VALUES ('delete', old.id, old.text);
  INSERT INTO facts_fts(rowid, text) VALUES (new.id, new.text);
END;

CREATE TABLE IF NOT EXISTS invites (
  code       TEXT PRIMARY KEY,
  room       TEXT NOT NULL,
  kind       TEXT NOT NULL,
  role       TEXT,
  max_uses   INTEGER NOT NULL,
  uses       INTEGER NOT NULL DEFAULT 0,
  expires    INTEGER NOT NULL,
  created_by TEXT NOT NULL,
  -- When 1, redeeming this invite records a pending join request the room owner must approve rather
  -- than joining outright. Default 0 ⇒ the historical "redeem joins immediately" behavior.
  require_approval INTEGER NOT NULL DEFAULT 0,
  created    INTEGER NOT NULL
);

-- Pending/denied join requests for approval-gated rooms. A redeem of an approval invite lands here as
-- `pending` (the requester is NOT yet a member); the room owner approves (→ membership, row removed)
-- or denies (→ status `denied`, which a requester cannot re-request past). Keyed per (room, agent) so
-- re-redeeming the same code is idempotent rather than flooding the owner's queue.
CREATE TABLE IF NOT EXISTS join_requests (
  room      TEXT NOT NULL,
  agent     TEXT NOT NULL,
  status    TEXT NOT NULL DEFAULT 'pending',
  requested INTEGER NOT NULL,
  PRIMARY KEY (room, agent)
);

-- The discovery directory: one signed AgentCard per agent, plus denormalized tags/skills (lowercased,
-- space-delimited) for cheap LIKE filtering. `registered` is the first publish; `updated` each refresh.
CREATE TABLE IF NOT EXISTS directory (
  agent      TEXT PRIMARY KEY,
  visibility TEXT NOT NULL DEFAULT 'private',
  card_json  TEXT NOT NULL,
  card_sig   TEXT,
  verified   INTEGER NOT NULL DEFAULT 0,
  tags       TEXT NOT NULL DEFAULT '',
  skills     TEXT NOT NULL DEFAULT '',
  registered INTEGER NOT NULL,
  updated    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_directory_visibility ON directory(visibility);

-- Read-scoped, expiring bearer tokens that unlock a private hub's directory over the REST API.
CREATE TABLE IF NOT EXISTS directory_tokens (
  token      TEXT PRIMARY KEY,
  scope      TEXT NOT NULL,
  expires    INTEGER NOT NULL,
  created_by TEXT NOT NULL,
  created    INTEGER NOT NULL
);

-- Content-addressed blob store (code handoff). Bytes live on disk under `<blob_dir>/<id>`; this is
-- just metadata, keyed by `id` = lowercase-hex SHA-256, so identical bytes dedup to one row.
CREATE TABLE IF NOT EXISTS blobs (
  id         TEXT PRIMARY KEY,
  media_type TEXT,
  size       INTEGER NOT NULL,
  created    INTEGER NOT NULL
);

-- Which rooms a blob was posted to (a blob may be handed off in several rooms, possibly by different
-- authors, even though the bytes are one). Download is authorized by membership of any such room.
CREATE TABLE IF NOT EXISTS blob_rooms (
  blob    TEXT NOT NULL,
  room    TEXT NOT NULL,
  author  TEXT NOT NULL,
  created INTEGER NOT NULL,
  PRIMARY KEY (blob, room)
);
"#;

/// Self-reported presence older than this (epoch-ms gap to "now") reads as `offline`. Presence is
/// self-reported and persists across disconnects; liveness is *derived* from this window — matching
/// the protocol's intent that `offline` is "derived by observers, not self-set while live".
pub const PRESENCE_STALE_MS: i64 = 300_000;

/// Aggregate communication metrics for one room (see [`Store::room_stats`]) — the numbers behind the
/// session viewer's "activity" panel. Every token figure is an **estimate** stored at append time
/// (see [`estimate_message_tokens`]); the hub relays text and can't see a model's real tokenizer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoomStats {
    /// Total messages appended to the room.
    pub messages: i64,
    /// Estimated tokens summed across every message — the room's communication cost.
    pub tokens: i64,
    /// Epoch-ms of the first / last message, or `None` for an empty room (the activity span).
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    /// Per-agent breakdown, most estimated tokens first. Display identity only — never an agent id.
    pub per_agent: Vec<AgentStat>,
}

/// One agent's slice of a [`RoomStats`] — how much this participant has said, keyed by the display
/// name/role the viewer shows (an agent id is deliberately never exposed here).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentStat {
    pub name: String,
    pub role: Option<String>,
    pub messages: i64,
    pub tokens: i64,
}

/// The durable store. Cheaply cloneable (shares the connections behind an `Arc`).
///
/// SQLite in WAL mode allows one writer and many concurrent readers, so the store keeps a single
/// dedicated **writer** connection plus a small pool of **read-only** connections that hot read paths
/// (`recall`/`discover`/`is_member`/…) fan out across — turning "one serial connection for the whole
/// hub" into "writes serialized (SQLite is single-writer anyway), reads parallel across cores". The
/// single writer is also what keeps `append_message`'s `last_insert_rowid()` correct. An in-memory
/// database can't be shared across connections, so the pool is empty there and reads fall back to the
/// writer (the historical single-connection behavior, preserved for tests).
#[derive(Clone)]
pub struct Store {
    inner: Arc<Inner>,
}

/// The result of [`Store::append_message`]. `deduped` is `true` when a `client_id` idempotency key
/// matched an already-stored message (#86): the `id`/`seq` are the *original* row's, no new row was
/// written, and `tokens` is 0 so the caller doesn't double-count metrics or re-fan-out.
#[derive(Debug, Clone)]
pub struct AppendOutcome {
    pub id: String,
    pub seq: i64,
    pub tokens: i64,
    pub deduped: bool,
}

struct Inner {
    writer: Mutex<Connection>,
    readers: Vec<Mutex<Connection>>,
    next: AtomicUsize,
    vec_dim: usize,
}

/// A borrowed connection guard that derefs to [`Connection`], hiding whether it came from the writer
/// or a pooled reader so call sites stay unchanged (`conn.execute(…)`, `conn.query_row(…)`).
struct ConnRef<'a>(MutexGuard<'a, Connection>);

impl Deref for ConnRef<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.0
    }
}

impl Store {
    /// Open the store at `path`, or in-memory (lost on exit) when `path` is `None`. Runs migrations.
    pub fn open(path: Option<&Path>) -> Result<Store> {
        ensure_vec_extension();

        // Decide the reader-pool size *first*, so the page-cache budget can be split across every
        // connection (writer + readers) rather than handed to each in full. In-memory can't share a
        // file across connections ⇒ no pool (reads fall back to the writer).
        let n_readers = match path {
            Some(_) => std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8),
            None => 0,
        };
        // `cache_size` is *per connection*, so a fixed per-connection value silently multiplies by the
        // pool size (1 writer + N readers). Split one total budget instead, so the hub's resident page
        // cache stays bounded no matter how many cores (hence readers) the host has.
        let cache_kib = (TOTAL_CACHE_KIB / (1 + n_readers as i64)).max(MIN_CACHE_KIB_PER_CONN);

        let writer = match path {
            Some(p) => {
                // Create the parent directory if it's missing, so a fresh DB path opens instead of
                // erroring — e.g. a container's mounted volume at `/data`, or a brand-new `--db` dir.
                if let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
                    std::fs::create_dir_all(dir)
                        .map_err(|e| anyhow!("creating db directory {}: {e}", dir.display()))?;
                }
                Connection::open(p)?
            }
            None => Connection::open_in_memory()?,
        };
        configure_conn(&writer, cache_kib)?;
        writer.execute_batch(MIGRATION)?;
        // Evolve tables created by an older schema without a destructive rebuild.
        add_column_if_missing(&writer, "blobs", "last_fetched", "INTEGER")?;
        add_column_if_missing(&writer, "rooms", "owner", "TEXT")?;
        add_column_if_missing(&writer, "invites", "require_approval", "INTEGER NOT NULL DEFAULT 0")?;
        add_column_if_missing(&writer, "facts", "embedding_model", "TEXT")?;
        // Estimated communication tokens per message (see `estimate_message_tokens`). When freshly
        // added to an existing DB, backfill historical rows once so a watched session's totals aren't
        // skewed toward zero by messages that predate the column.
        if add_column_if_missing(&writer, "messages", "tokens", "INTEGER NOT NULL DEFAULT 0")? {
            backfill_message_tokens(&writer)?;
        }
        // Idempotency key (#86) on older DBs. The partial unique index is created by MIGRATION above
        // once the column exists; add the column first for a DB that predates it.
        add_column_if_missing(&writer, "messages", "client_id", "TEXT")?;
        writer.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_client_id \
             ON messages(room, author, client_id) WHERE client_id IS NOT NULL;",
        )?;

        // Vector index for semantic recall (sqlite-vec, registered via auto_extension above).
        let vec_dim = VEC_DIMENSION;
        create_vec_table(&writer, vec_dim)?;

        // Read-only pool for a file-backed DB (the writer has already created the -wal/-shm files, so
        // read-only connections can attach).
        let mut readers = Vec::new();
        if let Some(p) = path {
            let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
            for _ in 0..n_readers {
                let rc = Connection::open_with_flags(p, flags)?;
                configure_conn(&rc, cache_kib)?;
                readers.push(Mutex::new(rc));
            }
        }
        Ok(Store {
            inner: Arc::new(Inner { writer: Mutex::new(writer), readers, next: AtomicUsize::new(0), vec_dim }),
        })
    }

    /// The configured embedding dimension for this store's vec_facts table.
    pub fn vec_dimension(&self) -> usize {
        self.inner.vec_dim
    }

    /// The writer connection — for every statement that mutates the database (including read-then-write
    /// ops like `pull`'s cursor advance). Writes are single-writer by design.
    fn w(&self) -> ConnRef<'_> {
        ConnRef(self.inner.writer.lock())
    }

    /// A pooled **read-only** connection (round-robin), or the writer when there is no pool (in-memory).
    /// Use only for pure reads — the pooled connections reject writes, so a misclassified write fails
    /// loudly against a file-backed DB rather than silently bypassing the single-writer invariant.
    fn r(&self) -> ConnRef<'_> {
        if self.inner.readers.is_empty() {
            ConnRef(self.inner.writer.lock())
        } else {
            let i = self.inner.next.fetch_add(1, Ordering::Relaxed) % self.inner.readers.len();
            ConnRef(self.inner.readers[i].lock())
        }
    }

    /// Run SQLite's (cheap) integrity check. `Ok(())` when the database reports `ok`, otherwise an
    /// error naming the first problem. Call it on boot / from `/health` so corruption is *detected*
    /// rather than silently read — the store is corruption-safe by design, this is the smoke alarm.
    pub fn quick_check(&self) -> Result<()> {
        // The writer, not a reader: validating the FTS5 inverted index needs write access.
        let conn = self.w();
        let res: String = conn.query_row("PRAGMA quick_check(1)", [], |r| r.get(0))?;
        if res == "ok" {
            Ok(())
        } else {
            bail!("sqlite integrity check failed: {res}");
        }
    }

    // ---- agents / presence ----

    pub fn upsert_agent(&self, id: &str, name: &str, role: Option<&str>, now: i64) -> Result<()> {
        let conn = self.w();
        conn.execute(
            "INSERT INTO agents (id, name, role, first_seen, last_seen) VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, role = excluded.role, last_seen = excluded.last_seen",
            params![id, name, role, now],
        )?;
        Ok(())
    }

    pub fn touch_presence(&self, agent: &str, status: &str, activity: Option<&str>, now: i64) -> Result<()> {
        let conn = self.w();
        conn.execute(
            "INSERT INTO presence (agent, status, activity, ts) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent) DO UPDATE SET status = excluded.status, activity = excluded.activity, ts = excluded.ts",
            params![agent, status, activity, now],
        )?;
        Ok(())
    }

    // ---- rooms / membership ----

    pub fn ensure_room(&self, name: &str, kind: RoomKind, description: Option<&str>, now: i64) -> Result<()> {
        let conn = self.w();
        conn.execute(
            "INSERT OR IGNORE INTO rooms (name, kind, description, created) VALUES (?1, ?2, ?3, ?4)",
            params![name, kind.as_str(), description, now],
        )?;
        Ok(())
    }

    pub fn add_member(&self, room: &str, agent: &str, now: i64) -> Result<()> {
        let conn = self.w();
        conn.execute(
            "INSERT OR IGNORE INTO members (room, agent, joined, cursor) VALUES (?1, ?2, ?3, 0)",
            params![room, agent, now],
        )?;
        Ok(())
    }

    pub fn is_member(&self, room: &str, agent: &str) -> Result<bool> {
        let conn = self.r();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM members WHERE room = ?1 AND agent = ?2",
            params![room, agent],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// The agent ids of every member of `room` — the recipient set for live push fan-out. Indexed by
    /// the `members` primary key (`room`, `agent`), so this is a cheap range scan even for big rooms.
    pub fn room_member_ids(&self, room: &str) -> Result<Vec<String>> {
        let conn = self.r();
        let mut stmt = conn.prepare("SELECT agent FROM members WHERE room = ?1")?;
        let v = stmt
            .query_map(params![room], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(v)
    }

    pub fn room_kind(&self, name: &str) -> Result<Option<RoomKind>> {
        let conn = self.r();
        let k: Option<String> = conn
            .query_row("SELECT kind FROM rooms WHERE name = ?1", params![name], |r| r.get(0))
            .optional()?;
        Ok(k.and_then(|s| RoomKind::parse(&s)))
    }

    /// The one DM room shared by exactly `a` and `b` (i.e. a 2-member `dm` room), if any.
    pub fn find_dm_room(&self, a: &str, b: &str) -> Result<Option<String>> {
        let conn = self.r();
        let room: Option<String> = conn
            .query_row(
                "SELECT m1.room FROM members m1
                   JOIN members m2 ON m1.room = m2.room
                   JOIN rooms r ON r.name = m1.room
                  WHERE m1.agent = ?1 AND m2.agent = ?2 AND r.kind = 'dm'
                    AND (SELECT COUNT(*) FROM members mm WHERE mm.room = m1.room) = 2
                  LIMIT 1",
                params![a, b],
                |r| r.get(0),
            )
            .optional()?;
        Ok(room)
    }

    pub fn rooms_of(&self, agent: &str) -> Result<Vec<RoomInfo>> {
        let conn = self.r();
        let mut stmt = conn.prepare(
            "SELECT m.room, r.kind,
                    (SELECT COUNT(*) FROM members WHERE room = m.room),
                    (SELECT COUNT(*) FROM messages msg WHERE msg.room = m.room AND msg.seq > m.cursor)
               FROM members m JOIN rooms r ON r.name = m.room
              WHERE m.agent = ?1
              ORDER BY m.room",
        )?;
        let rows = stmt
            .query_map(params![agent], |r| {
                let kind: String = r.get(1)?;
                Ok(RoomInfo {
                    name: r.get(0)?,
                    kind: RoomKind::parse(&kind).unwrap_or(RoomKind::Channel),
                    members: r.get::<_, i64>(2)? as u32,
                    unread: r.get::<_, i64>(3)? as u32,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn roster(&self, room: &str, now: i64) -> Result<Vec<RosterEntry>> {
        let conn = self.r();
        let mut stmt = conn.prepare(
            "SELECT a.id, a.name, a.role, p.status, p.activity, p.ts, a.last_seen
               FROM members mb JOIN agents a ON a.id = mb.agent
               LEFT JOIN presence p ON p.agent = a.id
              WHERE mb.room = ?1
              ORDER BY a.name",
        )?;
        let rows = stmt
            .query_map(params![room], |r| {
                let raw_status: Option<String> = r.get(3)?;
                let p_ts: Option<i64> = r.get(5)?;
                let last_seen: i64 = r.get(6)?;
                // Self-reported status, decayed to `offline` once the heartbeat goes stale.
                let status = match (raw_status, p_ts) {
                    (Some(s), Some(ts)) if now - ts <= PRESENCE_STALE_MS => s,
                    _ => "offline".to_string(),
                };
                Ok(RosterEntry {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    role: r.get(2)?,
                    status,
                    activity: r.get(4)?,
                    last_seen: p_ts.unwrap_or(last_seen),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ---- messages ----

    /// Append a message and return `(id, seq, tokens)` — `tokens` is the stored estimate of this
    /// message's communication cost (see [`estimate_message_tokens`]), returned so the caller can also
    /// bump the hub's cumulative counter from the same computation.
    #[allow(clippy::too_many_arguments)]
    pub fn append_message(
        &self,
        room: &str,
        from: &EndpointRef,
        parts: &[Part],
        mentions: Option<&[String]>,
        reply_to: Option<&str>,
        client_id: Option<&str>,
        ts: i64,
    ) -> Result<AppendOutcome> {
        let id = Uuid::now_v7().to_string();
        let parts_json = serde_json::to_string(parts)?;
        let mentions_json = match mentions {
            Some(m) => Some(serde_json::to_string(m)?),
            None => None,
        };
        // Estimate + persist the token cost at write time, so per-room/per-agent totals are a cheap SQL
        // aggregate later instead of a re-parse of every row on each viewer poll.
        let tokens = estimate_message_tokens(parts) as i64;
        let conn = self.w();
        // Idempotent send (#86): with a client_id, a retried send whose first attempt already landed
        // must return the ORIGINAL row, not insert a second. `INSERT … ON CONFLICT DO NOTHING` against
        // the partial unique (room, author, client_id) index makes the insert a no-op on a replay;
        // we then read back the existing row and report it as a (deduped) success. Unkeyed sends
        // (client_id NULL) are excluded from the index and always insert, exactly as before.
        let changed = conn.execute(
            "INSERT INTO messages (id, room, author, author_name, author_role, parts, mentions, reply_to, ts, tokens, client_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(room, author, client_id) WHERE client_id IS NOT NULL DO NOTHING",
            params![id, room, from.id, from.name, from.role, parts_json, mentions_json, reply_to, ts, tokens, client_id],
        )?;
        if changed == 1 {
            return Ok(AppendOutcome { id, seq: conn.last_insert_rowid(), tokens, deduped: false });
        }
        // No row inserted ⇒ a client_id conflict: fetch the original message's id + seq and report it
        // as the same success the first send returned. (Reachable only when client_id is Some.)
        let (orig_id, orig_seq): (String, i64) = conn.query_row(
            "SELECT id, seq FROM messages WHERE room = ?1 AND author = ?2 AND client_id = ?3",
            params![room, from.id, client_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(AppendOutcome { id: orig_id, seq: orig_seq, tokens: 0, deduped: true })
    }

    /// `pull`'s cursor read shares the same already-held connection (see [`Store::pull`]); the
    /// approval path's membership/ownership checks do the same via the free helpers below, since they
    /// run while the writer lock is held and must not re-lock it.
    fn get_cursor(conn: &Connection, room: &str, agent: &str) -> Result<i64> {
        let cur: Option<i64> = conn
            .query_row(
                "SELECT cursor FROM members WHERE room = ?1 AND agent = ?2",
                params![room, agent],
                |r| r.get(0),
            )
            .optional()?;
        Ok(cur.unwrap_or(0))
    }

    /// Messages in `room` newer than the agent's cursor (advanced) or `since` (not advanced).
    /// Returns the messages and the resulting cursor.
    pub fn pull(
        &self,
        room: &str,
        agent: &str,
        since: Option<i64>,
        limit: Option<u32>,
        ack: Option<i64>,
    ) -> Result<(Vec<StoredMessage>, i64)> {
        let lim = limit.unwrap_or(200).min(1000) as i64;
        // Resolve the read floor. For a cursor read, apply a deferred ack first (#85): advance the
        // stored cursor to `ack` (monotonic — never backward) *before* reading, so acknowledged
        // messages are never re-read. When `ack` is present we then read from that floor but do NOT
        // advance past the returned batch below (the next pull acks it), so a batch whose reply is
        // lost on a drop is re-read on retry rather than skipped. A `since` re-read is a pure read.
        let cur = match since {
            Some(s) => s,
            None => {
                let stored = {
                    let conn = self.r();
                    Self::get_cursor(&conn, room, agent)?
                };
                match ack {
                    Some(a) if a > stored => {
                        self.w().execute(
                            "UPDATE members SET cursor = ?1 WHERE room = ?2 AND agent = ?3",
                            params![a, room, agent],
                        )?;
                        a
                    }
                    _ => stored,
                }
            }
        };
        // Read the backlog on a pooled read-only connection (the hot, expensive part); the only write
        // is the tiny cursor advance below, which goes to the writer.
        let conn = self.r();
        let raws = {
            let mut stmt = conn.prepare(
                "SELECT seq, id, room, author, author_name, author_role, parts, mentions, reply_to, ts
                   FROM messages WHERE room = ?1 AND seq > ?2 ORDER BY seq ASC LIMIT ?3",
            )?;
            let v = stmt
                .query_map(params![room, cur, lim], |r| {
                    Ok(RawMsg {
                        seq: r.get(0)?,
                        id: r.get(1)?,
                        room: r.get(2)?,
                        author: r.get(3)?,
                        name: r.get(4)?,
                        role: r.get(5)?,
                        parts: r.get(6)?,
                        mentions: r.get(7)?,
                        reply_to: r.get(8)?,
                        ts: r.get(9)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            v
        };
        drop(conn); // release the read connection before taking the writer
        let new_cursor = raws.last().map(|r| r.seq).unwrap_or(cur);
        // Advance-on-read ONLY for old (ack-less) clients. An ack-aware client (`ack` present) leaves
        // the cursor at the ack floor — it commits this batch on its next pull's `ack` (#85).
        if since.is_none() && ack.is_none() && new_cursor > cur {
            self.w().execute(
                "UPDATE members SET cursor = ?1 WHERE room = ?2 AND agent = ?3",
                params![new_cursor, room, agent],
            )?;
        }
        let mut msgs = Vec::with_capacity(raws.len());
        for raw in &raws {
            msgs.push(raw.to_stored()?);
        }
        Ok((msgs, new_cursor))
    }

    /// Read `room`'s messages newer than `since` (ascending by `seq`), capped at `limit`. A **pure
    /// read**: unlike [`Store::pull`] it advances no member cursor — the read-only `/api/session`
    /// viewer is not a member of the room, so it must never mutate one agent's delivery state. The
    /// caller authorizes access *before* calling this (a valid watch token for exactly this room).
    pub fn room_messages(&self, room: &str, since: i64, limit: u32) -> Result<Vec<StoredMessage>> {
        let lim = limit.min(1000) as i64;
        let conn = self.r();
        let mut stmt = conn.prepare(
            "SELECT seq, id, room, author, author_name, author_role, parts, mentions, reply_to, ts
               FROM messages WHERE room = ?1 AND seq > ?2 ORDER BY seq ASC LIMIT ?3",
        )?;
        let raws = stmt
            .query_map(params![room, since, lim], |r| {
                Ok(RawMsg {
                    seq: r.get(0)?,
                    id: r.get(1)?,
                    room: r.get(2)?,
                    author: r.get(3)?,
                    name: r.get(4)?,
                    role: r.get(5)?,
                    parts: r.get(6)?,
                    mentions: r.get(7)?,
                    reply_to: r.get(8)?,
                    ts: r.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut msgs = Vec::with_capacity(raws.len());
        for raw in &raws {
            msgs.push(raw.to_stored()?);
        }
        Ok(msgs)
    }

    /// Aggregate communication metrics for `room` — total messages + estimated tokens, the activity
    /// span (first/last message time), and a per-agent breakdown by **display identity** (name/role,
    /// never an agent id). Powers the session viewer's activity panel; like [`Store::room_messages`]
    /// the caller authorizes access first (a valid watch token for exactly this room). All token
    /// figures are the estimates stored at append time — see [`estimate_message_tokens`].
    pub fn room_stats(&self, room: &str) -> Result<RoomStats> {
        let conn = self.r();
        let (messages, tokens, first_ts, last_ts) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(tokens), 0), MIN(ts), MAX(ts) FROM messages WHERE room = ?1",
            params![room],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            },
        )?;
        // Group by the display identity we actually surface (name/role) — never the agent id — so a
        // stable agent is one row and no public key can leak into the viewer. Chattiest (most estimated
        // tokens) first, ties broken by message count then name for a deterministic order.
        let per_agent = {
            let mut stmt = conn.prepare(
                "SELECT author_name, author_role, COUNT(*) AS n, COALESCE(SUM(tokens), 0) AS toks
                   FROM messages WHERE room = ?1
                  GROUP BY author_name, author_role
                  ORDER BY toks DESC, n DESC, author_name ASC",
            )?;
            let rows = stmt
                .query_map(params![room], |r| {
                    Ok(AgentStat {
                        name: r.get(0)?,
                        role: r.get(1)?,
                        messages: r.get(2)?,
                        tokens: r.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        Ok(RoomStats { messages, tokens, first_ts, last_ts, per_agent })
    }

    // ---- invites ----

    #[allow(clippy::too_many_arguments)]
    pub fn create_invite(
        &self,
        code: &str,
        room: &str,
        kind: RoomKind,
        role: Option<&str>,
        max_uses: u32,
        expires: i64,
        created_by: &str,
        require_approval: bool,
        now: i64,
    ) -> Result<()> {
        let conn = self.w();
        conn.execute(
            "INSERT INTO invites (code, room, kind, role, max_uses, uses, expires, created_by, require_approval, created)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8, ?9)",
            params![code, room, kind.as_str(), role, max_uses, expires, created_by, require_approval as i64, now],
        )?;
        Ok(())
    }

    /// Mark `owner` as the room's owner if it has none yet (set-once). Only the owner may approve or
    /// deny pending joins for an approval-gated room, so this can't be silently reassigned later.
    pub fn set_room_owner(&self, room: &str, owner: &str) -> Result<()> {
        let conn = self.w();
        conn.execute(
            "UPDATE rooms SET owner = ?2 WHERE name = ?1 AND owner IS NULL",
            params![room, owner],
        )?;
        Ok(())
    }

    /// Redeem `code` for `agent`. For an ordinary invite this validates expiry + remaining uses,
    /// charges a use, and joins the room. For an **approval-gated** invite it instead records a
    /// *pending request* (the agent is not admitted) the room owner must approve — see
    /// [`Store::resolve_join`]. Already-member redeems are idempotent (no use charged), so a pending
    /// joiner can re-redeem the same code to poll for the owner's decision.
    pub fn redeem_invite(&self, code: &str, agent: &str, now: i64) -> Result<Redeemed> {
        let conn = self.w();
        let row: Option<(String, String, i64, i64, i64, i64)> = conn
            .query_row(
                "SELECT room, kind, max_uses, uses, expires, require_approval FROM invites WHERE code = ?1",
                params![code],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()?;
        let (room, kind_s, max_uses, uses, expires, require_approval) =
            row.ok_or_else(|| anyhow!("invalid or unknown invite code"))?;
        let kind = RoomKind::parse(&kind_s).unwrap_or(RoomKind::Channel);

        // Idempotent: already in the room (e.g. the owner, or a re-redeem after approval) → just say
        // joined, without charging a use or re-checking expiry/limits.
        if member_exists(&conn, &room, agent)? {
            return Ok(Redeemed { room, kind, pending: false });
        }
        if now > expires {
            bail!("invite has expired");
        }

        if require_approval != 0 {
            // Approval flow: a redeem becomes a request the owner vets — it does NOT grant access.
            let status: Option<String> = conn
                .query_row(
                    "SELECT status FROM join_requests WHERE room = ?1 AND agent = ?2",
                    params![room, agent],
                    |r| r.get(0),
                )
                .optional()?;
            match status.as_deref() {
                // Already waiting → idempotent poll, no extra use charged or queue entry added.
                Some("pending") => Ok(Redeemed { room, kind, pending: true }),
                // A denial is terminal for the requester (it cannot re-request its way in).
                Some("denied") => bail!("your request to join was denied by the host"),
                _ => {
                    // A fresh requester consumes one use (so `max_uses` also bounds the pending queue).
                    if uses >= max_uses {
                        bail!("invite has already been used up");
                    }
                    conn.execute("UPDATE invites SET uses = uses + 1 WHERE code = ?1", params![code])?;
                    conn.execute(
                        "INSERT INTO join_requests (room, agent, status, requested) VALUES (?1, ?2, 'pending', ?3)",
                        params![room, agent, now],
                    )?;
                    Ok(Redeemed { room, kind, pending: true })
                }
            }
        } else {
            if uses >= max_uses {
                bail!("invite has already been used up");
            }
            conn.execute("UPDATE invites SET uses = uses + 1 WHERE code = ?1", params![code])?;
            conn.execute(
                "INSERT OR IGNORE INTO members (room, agent, joined, cursor) VALUES (?1, ?2, ?3, 0)",
                params![room, agent, now],
            )?;
            Ok(Redeemed { room, kind, pending: false })
        }
    }

    /// The pending join requests for `room`, authorized to its **owner** only. A non-owner (or an
    /// unknown room) gets an error rather than a peek at who is waiting.
    pub fn pending_join_requests(&self, room: &str, owner: &str) -> Result<Vec<JoinRequest>> {
        let conn = self.r();
        if !room_owned_by(&conn, room, owner)? {
            bail!("only the session owner can view join requests for '{room}'");
        }
        let mut stmt = conn.prepare(
            "SELECT jr.agent, a.name, a.role, jr.requested
               FROM join_requests jr LEFT JOIN agents a ON a.id = jr.agent
              WHERE jr.room = ?1 AND jr.status = 'pending'
              ORDER BY jr.requested ASC",
        )?;
        let rows = stmt
            .query_map(params![room], |r| {
                Ok(JoinRequest {
                    agent: r.get(0)?,
                    name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    role: r.get(2)?,
                    requested_at: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Approve or deny a pending join request for `room`, authorized to its **owner** only. On
    /// `approve` the requester is admitted as a member and its request cleared; on deny the request is
    /// marked `denied` (so the requester cannot re-request). Errors if the caller isn't the owner or
    /// there is no such request. Returns whether the requester was admitted.
    pub fn resolve_join(&self, room: &str, owner: &str, agent: &str, approve: bool, now: i64) -> Result<bool> {
        let conn = self.w();
        if !room_owned_by(&conn, room, owner)? {
            bail!("only the session owner can approve or deny join requests for '{room}'");
        }
        let exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM join_requests WHERE room = ?1 AND agent = ?2",
            params![room, agent],
            |r| r.get(0),
        )?;
        if exists == 0 {
            bail!("no pending join request from '{agent}' for '{room}'");
        }
        if approve {
            conn.execute(
                "DELETE FROM join_requests WHERE room = ?1 AND agent = ?2",
                params![room, agent],
            )?;
            conn.execute(
                "INSERT OR IGNORE INTO members (room, agent, joined, cursor) VALUES (?1, ?2, ?3, 0)",
                params![room, agent, now],
            )?;
        } else {
            conn.execute(
                "UPDATE join_requests SET status = 'denied' WHERE room = ?1 AND agent = ?2",
                params![room, agent],
            )?;
        }
        Ok(approve)
    }

    // ---- directory (discovery) ----

    /// Publish or refresh an agent's directory card. `card.id` is the primary key; `registered` is
    /// kept from the first publish, `updated` bumped each time. `tags`/`skills` are denormalized
    /// (lowercased) for cheap filtering.
    pub fn register_card(
        &self,
        card: &AgentCard,
        sig: Option<&str>,
        verified: bool,
        visibility: Visibility,
        now: i64,
    ) -> Result<()> {
        let card_json = serde_json::to_string(card)?;
        let (tags, skills) = card_filter_blobs(card);
        let conn = self.w();
        conn.execute(
            "INSERT INTO directory (agent, visibility, card_json, card_sig, verified, tags, skills, registered, updated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
             ON CONFLICT(agent) DO UPDATE SET
               visibility = excluded.visibility, card_json = excluded.card_json,
               card_sig = excluded.card_sig, verified = excluded.verified,
               tags = excluded.tags, skills = excluded.skills, updated = excluded.updated",
            params![card.id, visibility.as_str(), card_json, sig, verified as i64, tags, skills, now],
        )?;
        Ok(())
    }

    /// Search the directory. [`DiscoverScope::Public`] limits to `public` agents; [`DiscoverScope::Hub`]
    /// returns every registered agent. Optional filters narrow by free-text (name/tags/skills),
    /// `tag`, `skill`, or presence `status`. `hub` stamps each returned entry with the hub name.
    #[allow(clippy::too_many_arguments)]
    pub fn discover(
        &self,
        scope: DiscoverScope,
        hub: &str,
        query: Option<&str>,
        tag: Option<&str>,
        skill: Option<&str>,
        status: Option<&str>,
        limit: Option<u32>,
        now: i64,
    ) -> Result<Vec<DirectoryEntry>> {
        let public_only = matches!(scope, DiscoverScope::Public) as i64;
        let q = query.map(|s| format!("%{}%", s.to_lowercase()));
        let tagp = tag.map(|s| format!("%{}%", s.to_lowercase()));
        let skillp = skill.map(|s| format!("%{}%", s.to_lowercase()));
        let want_status = status.map(|s| s.to_lowercase());
        let lim = limit.unwrap_or(200).min(1000) as usize;
        let conn = self.r();
        let mut stmt = conn.prepare(
            "SELECT d.card_json, d.visibility, d.card_sig, d.verified,
                    p.status, p.activity, p.ts, a.first_seen, a.last_seen
               FROM directory d
               JOIN agents a ON a.id = d.agent
               LEFT JOIN presence p ON p.agent = d.agent
              WHERE (:public_only = 0 OR d.visibility = 'public')
                AND (:q IS NULL OR LOWER(a.name) LIKE :q OR d.tags LIKE :q OR d.skills LIKE :q)
                AND (:tag IS NULL OR d.tags LIKE :tag)
                AND (:skill IS NULL OR d.skills LIKE :skill)
              ORDER BY a.last_seen DESC",
        )?;
        let raws = stmt
            .query_map(
                named_params! { ":public_only": public_only, ":q": q, ":tag": tagp, ":skill": skillp },
                RawDir::from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        // Derive presence staleness + apply the status filter and limit after, since `offline` is
        // computed (not stored).
        let mut out = Vec::new();
        for r in &raws {
            let entry = r.to_entry(hub, now)?;
            if let Some(ws) = &want_status {
                if entry.status.to_lowercase() != *ws {
                    continue;
                }
            }
            out.push(entry);
            if out.len() >= lim {
                break;
            }
        }
        Ok(out)
    }

    /// Fetch one agent's directory entry by id. A `public` card is always returned; a `private` one
    /// only when `hub_scope` (the caller is an authenticated member / holds a valid directory token).
    pub fn lookup_card(&self, id: &str, hub: &str, hub_scope: bool, now: i64) -> Result<Option<DirectoryEntry>> {
        let conn = self.r();
        let raw: Option<RawDir> = conn
            .query_row(
                "SELECT d.card_json, d.visibility, d.card_sig, d.verified,
                        p.status, p.activity, p.ts, a.first_seen, a.last_seen
                   FROM directory d
                   JOIN agents a ON a.id = d.agent
                   LEFT JOIN presence p ON p.agent = d.agent
                  WHERE d.agent = ?1",
                params![id],
                RawDir::from_row,
            )
            .optional()?;
        match raw {
            Some(r) if hub_scope || r.visibility == "public" => Ok(Some(r.to_entry(hub, now)?)),
            _ => Ok(None),
        }
    }

    /// The visibility of an agent's directory card, or `None` if it never registered one.
    /// Used to decide whether a peer may open a DM by id: a registered agent is *reachable*.
    pub fn directory_visibility(&self, agent: &str) -> Result<Option<Visibility>> {
        let conn = self.r();
        let v: Option<String> = conn
            .query_row(
                "SELECT visibility FROM directory WHERE agent = ?1",
                params![agent],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.and_then(|s| Visibility::parse(&s)))
    }

    /// `(total registered, public)` agent counts — for the `/api/hub` summary.
    pub fn directory_counts(&self) -> Result<(i64, i64)> {
        let conn = self.r();
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM directory", [], |r| r.get(0))?;
        let public: i64 = conn.query_row(
            "SELECT COUNT(*) FROM directory WHERE visibility = 'public'",
            [],
            |r| r.get(0),
        )?;
        Ok((total, public))
    }

    // ---- directory tokens (private-hub read access for the website) ----

    pub fn mint_directory_token(
        &self,
        token: &str,
        scope: &str,
        expires: i64,
        created_by: &str,
        now: i64,
    ) -> Result<()> {
        let conn = self.w();
        conn.execute(
            "INSERT INTO directory_tokens (token, scope, expires, created_by, created)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![token, scope, expires, created_by, now],
        )?;
        Ok(())
    }

    /// `true` when `token` exists, has not expired, **and** is a directory-scoped token. The scope
    /// check matters now that the same table also holds room-scoped *watch* tokens (scope `watch:<room>`):
    /// without it, a read-only watch bearer could be replayed to unlock the whole private directory. A
    /// directory token is minted with scope `"hub"` (see the server's `MintDirectoryToken` handler).
    pub fn validate_directory_token(&self, token: &str, now: i64) -> Result<bool> {
        let conn = self.r();
        let exp: Option<i64> = conn
            .query_row(
                "SELECT expires FROM directory_tokens WHERE token = ?1 AND scope = 'hub'",
                params![token],
                |r| r.get(0),
            )
            .optional()?;
        Ok(matches!(exp, Some(e) if now <= e))
    }

    /// Mint a read-only **watch** token bound to one `room`, authorized to the room's **owner** only.
    /// Reuses the `directory_tokens` table with a room-scoped `watch:<room>` scope (so the existing
    /// `sweep_expired` janitor reaps it). A non-owner — including an approved *member* who is not the
    /// owner — cannot mint one, so exposing a session to outside viewers stays the host's call alone.
    pub fn mint_watch_token(&self, token: &str, room: &str, owner: &str, expires: i64, now: i64) -> Result<()> {
        let conn = self.w();
        if !room_owned_by(&conn, room, owner)? {
            bail!("only the session owner can mint a watch link for '{room}'");
        }
        conn.execute(
            "INSERT INTO directory_tokens (token, scope, expires, created_by, created)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![token, format!("watch:{room}"), expires, owner, now],
        )?;
        Ok(())
    }

    /// Resolve a watch `token` to the single room it grants read access to — `Some(room)` iff the token
    /// exists, has not expired, and carries a `watch:<room>` scope. This is the *only* authorization for
    /// the read-only `/api/session` viewer, so it is deliberately narrow: one token unlocks exactly one
    /// room, nothing else.
    pub fn validate_watch_token(&self, token: &str, now: i64) -> Result<Option<String>> {
        let conn = self.r();
        let row: Option<(String, i64)> = conn
            .query_row(
                "SELECT scope, expires FROM directory_tokens WHERE token = ?1",
                params![token],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            Some((scope, exp)) if now <= exp => Ok(scope.strip_prefix("watch:").map(|s| s.to_string())),
            _ => Ok(None),
        }
    }

    // ---- blobs (code handoff) ----

    /// Record a stored blob's metadata and bind it to `room` (idempotent: same bytes/room reuse the
    /// rows). The bytes themselves are written to disk by the caller, keyed by `id`.
    pub fn put_blob_meta(
        &self,
        id: &str,
        room: &str,
        author: &str,
        media_type: Option<&str>,
        size: i64,
        now: i64,
    ) -> Result<()> {
        let conn = self.w();
        conn.execute(
            "INSERT INTO blobs (id, media_type, size, created) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO NOTHING",
            params![id, media_type, size, now],
        )?;
        conn.execute(
            "INSERT INTO blob_rooms (blob, room, author, created) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(blob, room) DO NOTHING",
            params![id, room, author, now],
        )?;
        Ok(())
    }

    /// Total bytes across all stored blobs — used to enforce the hub's disk budget.
    pub fn total_blob_bytes(&self) -> Result<i64> {
        let conn = self.r();
        let n: i64 = conn.query_row("SELECT COALESCE(SUM(size), 0) FROM blobs", [], |r| r.get(0))?;
        Ok(n)
    }

    /// A stored blob's metadata (bytes length + media type), or `None` if unknown.
    pub fn blob_meta(&self, id: &str) -> Result<Option<BlobMeta>> {
        let conn = self.r();
        let m = conn
            .query_row(
                "SELECT id, media_type, size, created FROM blobs WHERE id = ?1",
                params![id],
                |r| {
                    Ok(BlobMeta {
                        id: r.get(0)?,
                        media_type: r.get(1)?,
                        size: r.get(2)?,
                        created: r.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(m)
    }

    /// Whether `agent` may download blob `id` — true iff it is a member of a room the blob was
    /// posted to.
    pub fn blob_readable_by(&self, id: &str, agent: &str) -> Result<bool> {
        let conn = self.r();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM blob_rooms br
               JOIN members m ON m.room = br.room
              WHERE br.blob = ?1 AND m.agent = ?2",
            params![id, agent],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    // ---- memory (facts) ----

    /// Write a fact. With a `key`, this upserts within (author, room, key) — idempotent updates.
    /// When `embedding` is provided, it is stored in the vec_facts table for semantic recall.
    pub fn remember(
        &self,
        author: &str,
        fact: &Fact,
        ts: i64,
        embedding: Option<&[f32]>,
        embedding_model: Option<&str>,
    ) -> Result<()> {
        if let Some(emb) = embedding {
            if emb.len() != self.inner.vec_dim {
                bail!(
                    "embedding dimension mismatch: got {}, expected {}",
                    emb.len(),
                    self.inner.vec_dim
                );
            }
        }
        let conn = self.w();
        let fact_id: i64 = match &fact.key {
            Some(k) => {
                let updated = conn.execute(
                    "UPDATE facts SET text = ?1, ts = ?2, embedding_model = ?6
                       WHERE author = ?3 AND IFNULL(room, '') = IFNULL(?4, '') AND fkey = ?5",
                    params![fact.text, ts, author, fact.room, k, embedding_model],
                )?;
                if updated == 0 {
                    conn.execute(
                        "INSERT INTO facts (fkey, room, author, text, ts, embedding_model) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![k, fact.room, author, fact.text, ts, embedding_model],
                    )?;
                    conn.last_insert_rowid()
                } else {
                    conn.query_row(
                        "SELECT id FROM facts WHERE author = ?1 AND IFNULL(room, '') = IFNULL(?2, '') AND fkey = ?3",
                        params![author, fact.room, k],
                        |r| r.get(0),
                    )?
                }
            }
            None => {
                conn.execute(
                    "INSERT INTO facts (fkey, room, author, text, ts, embedding_model) VALUES (NULL, ?1, ?2, ?3, ?4, ?5)",
                    params![fact.room, author, fact.text, ts, embedding_model],
                )?;
                conn.last_insert_rowid()
            }
        };

        if let Some(emb) = embedding {
            let emb_bytes = floats_to_bytes(emb);
            conn.execute("DELETE FROM vec_facts WHERE fact_id = ?1", params![fact_id])?;
            conn.execute(
                "INSERT INTO vec_facts (fact_id, embedding) VALUES (?1, ?2)",
                params![fact_id, emb_bytes],
            )?;
        }
        Ok(())
    }

    /// Recall from the memory store. Pure text runs FTS5/BM25; with an embedding, runs hybrid
    /// BM25 + vector KNN fused via Reciprocal Rank Fusion. Either query text or embedding (or both)
    /// must be provided.
    pub fn recall(
        &self,
        agent: &str,
        query: &str,
        room: Option<&str>,
        limit: Option<u32>,
        embedding: Option<&[f32]>,
    ) -> Result<Vec<RecallHit>> {
        if let Some(emb) = embedding {
            if emb.len() != self.inner.vec_dim {
                bail!(
                    "embedding dimension mismatch: got {}, expected {}",
                    emb.len(),
                    self.inner.vec_dim
                );
            }
        }
        let lim = limit.unwrap_or(8).min(50) as i64;
        let match_q = build_fts_query(query);
        let has_text = !match_q.is_empty();

        let fts_hits = if has_text {
            self.recall_fts(agent, &match_q, room, lim)?
        } else {
            vec![]
        };

        let vec_hits = if let Some(emb) = embedding {
            self.recall_vec(agent, emb, room, lim)?
        } else {
            vec![]
        };

        if fts_hits.is_empty() && vec_hits.is_empty() {
            return Ok(vec![]);
        }
        if vec_hits.is_empty() {
            return Ok(fts_hits);
        }
        if fts_hits.is_empty() {
            return Ok(vec_hits);
        }

        Ok(rrf_fuse(&fts_hits, &vec_hits, lim as usize))
    }

    /// Deterministic keyed fact fetch (#91): return the fact(s) stored under `key` (`fkey`),
    /// **independent of BM25** — an exact lookup, newest first. Scoped exactly like [`Self::recall`]: a
    /// given `room` restricts to that room; without one, the agent's rooms plus its own unroomed facts.
    /// Membership on an explicit `room` is enforced by the caller (as in the BM25 path).
    pub fn recall_by_key(
        &self,
        agent: &str,
        key: &str,
        room: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<RecallHit>> {
        let lim = limit.unwrap_or(8).min(50) as i64;
        let conn = self.r();
        // Exact keyed hit: no ranking, so score is a fixed 0.0 (best possible, unused by callers).
        let map = |r: &rusqlite::Row| {
            Ok(RecallHit {
                text: r.get(0)?,
                key: r.get(1)?,
                room: r.get(2)?,
                author: r.get(3)?,
                ts: r.get(4)?,
                score: 0.0,
            })
        };
        let hits = match room {
            Some(room) => {
                let mut stmt = conn.prepare(
                    "SELECT f.text, f.fkey, f.room, f.author, f.ts
                       FROM facts f
                      WHERE f.fkey = ?1 AND f.room = ?2
                      ORDER BY f.ts DESC LIMIT ?3",
                )?;
                let v = stmt
                    .query_map(params![key, room, lim], map)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                v
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT f.text, f.fkey, f.room, f.author, f.ts
                       FROM facts f
                      WHERE f.fkey = ?1
                        AND ((f.room IS NULL AND f.author = ?2)
                          OR f.room IN (SELECT room FROM members WHERE agent = ?2))
                      ORDER BY f.ts DESC LIMIT ?3",
                )?;
                let v = stmt
                    .query_map(params![key, agent, lim], map)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                v
            }
        };
        Ok(hits)
    }

    fn recall_fts(
        &self,
        agent: &str,
        match_q: &str,
        room: Option<&str>,
        lim: i64,
    ) -> Result<Vec<RecallHit>> {
        let conn = self.r();
        let map = |r: &rusqlite::Row| {
            Ok(RecallHit {
                text: r.get(0)?,
                key: r.get(1)?,
                room: r.get(2)?,
                author: r.get(3)?,
                ts: r.get(4)?,
                score: r.get(5)?,
            })
        };
        let hits = match room {
            Some(room) => {
                let mut stmt = conn.prepare(
                    "SELECT f.text, f.fkey, f.room, f.author, f.ts, bm25(facts_fts) AS score
                       FROM facts_fts JOIN facts f ON f.id = facts_fts.rowid
                      WHERE facts_fts MATCH ?1 AND f.room = ?2
                      ORDER BY score LIMIT ?3",
                )?;
                let v = stmt
                    .query_map(params![match_q, room, lim], map)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                v
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT f.text, f.fkey, f.room, f.author, f.ts, bm25(facts_fts) AS score
                       FROM facts_fts JOIN facts f ON f.id = facts_fts.rowid
                      WHERE facts_fts MATCH ?1
                        AND ((f.room IS NULL AND f.author = ?2)
                          OR f.room IN (SELECT room FROM members WHERE agent = ?2))
                      ORDER BY score LIMIT ?3",
                )?;
                let v = stmt
                    .query_map(params![match_q, agent, lim], map)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                v
            }
        };
        Ok(hits)
    }

    fn recall_vec(
        &self,
        agent: &str,
        embedding: &[f32],
        room: Option<&str>,
        lim: i64,
    ) -> Result<Vec<RecallHit>> {
        let conn = self.r();
        let emb_bytes = floats_to_bytes(embedding);
        // Over-fetch from vec0 (no scope filter in the KNN query), then filter.
        let fetch_k = (lim * 5).min(200);
        let mut stmt = conn.prepare(
            "SELECT vf.fact_id, vf.distance,
                    f.text, f.fkey, f.room, f.author, f.ts
               FROM vec_facts vf
               JOIN facts f ON f.id = vf.fact_id
              WHERE vf.embedding MATCH ?1 AND vf.k = ?2",
        )?;
        let candidates = {
            let v = stmt
                .query_map(params![emb_bytes, fetch_k], |r| {
                    Ok((
                        r.get::<_, f64>(1)?, // distance
                        RecallHit {
                            text: r.get(2)?,
                            key: r.get(3)?,
                            room: r.get(4)?,
                            author: r.get(5)?,
                            ts: r.get(6)?,
                            score: r.get(1)?,
                        },
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            v
        };

        // Filter by scope (same rules as FTS: room-scoped or agent's reachable memory).
        let agent_rooms: Option<Vec<String>> = if room.is_none() {
            let mut stmt2 = conn.prepare("SELECT room FROM members WHERE agent = ?1")?;
            let v = stmt2
                .query_map(params![agent], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            Some(v)
        } else {
            None
        };

        let filtered: Vec<RecallHit> = candidates
            .into_iter()
            .filter_map(|(_dist, hit)| {
                let in_scope = match room {
                    Some(r) => hit.room.as_deref() == Some(r),
                    None => match &hit.room {
                        None => hit.author == agent,
                        Some(r) => agent_rooms.as_ref().is_some_and(|rooms| rooms.contains(r)),
                    },
                };
                if in_scope { Some(hit) } else { None }
            })
            .take(lim as usize)
            .collect();
        Ok(filtered)
    }

    // ---- retention / janitor (bound the append-only growth of a long-lived public hub) ----

    /// Delete messages older than `retain_ms`, but always keep at least the newest `keep_per_room`
    /// messages in each room. Age-based and deliberately simple: an agent offline longer than the
    /// window may miss expired messages (the bus is at-least-once, not infinite-retention). Cursors
    /// need no fix-up — `pull` reads `seq > cursor`, so a cursor below a pruned `seq` just resumes at
    /// the next surviving row. Returns the number of rows removed.
    pub fn prune_messages(&self, retain_ms: i64, keep_per_room: i64, now: i64) -> Result<usize> {
        let cutoff = now - retain_ms;
        let conn = self.w();
        // The correlated subquery is the `seq` of the (keep_per_room+1)-th newest message in the row's
        // room; `COALESCE(.., -1)` keeps everything when a room has fewer than that (seq starts at 1).
        let n = conn.execute(
            "DELETE FROM messages
              WHERE ts < ?1
                AND seq <= COALESCE(
                      (SELECT seq FROM messages m2 WHERE m2.room = messages.room
                        ORDER BY seq DESC LIMIT 1 OFFSET ?2), -1)",
            params![cutoff, keep_per_room],
        )?;
        Ok(n)
    }

    /// Keep only the newest `keep` *unkeyed* facts per `(author, room-or-private)`. Keyed facts are
    /// already bounded (they upsert) and are never pruned here. The FTS index stays consistent via the
    /// `facts_ad` delete trigger; vec_facts orphans are cleaned up afterwards. Returns the number of
    /// rows removed.
    pub fn prune_facts(&self, keep: i64) -> Result<usize> {
        let conn = self.w();
        let n = conn.execute(
            "DELETE FROM facts
              WHERE fkey IS NULL
                AND id <= COALESCE(
                      (SELECT id FROM facts f2
                        WHERE f2.fkey IS NULL AND f2.author = facts.author
                          AND IFNULL(f2.room,'') = IFNULL(facts.room,'')
                        ORDER BY id DESC LIMIT 1 OFFSET ?1), -1)",
            params![keep],
        )?;
        if n > 0 {
            conn.execute(
                "DELETE FROM vec_facts WHERE fact_id NOT IN (SELECT id FROM facts)",
                [],
            )?;
        }
        Ok(n)
    }

    /// Mark a blob as fetched `now` — the LRU input for [`Store::gc_blobs`].
    pub fn touch_blob_fetched(&self, id: &str, now: i64) -> Result<()> {
        let conn = self.w();
        conn.execute("UPDATE blobs SET last_fetched = ?2 WHERE id = ?1", params![id, now])?;
        Ok(())
    }

    /// Garbage-collect blobs neither fetched nor created within `ttl_ms`. Removes the `blobs` +
    /// `blob_rooms` rows and returns the content ids whose on-disk bytes the caller must unlink (file
    /// I/O is kept off the DB lock). This is the disk-budget counterpart to message retention — an
    /// idle bundle eventually expires and becomes unfetchable.
    pub fn gc_blobs(&self, ttl_ms: i64, now: i64) -> Result<Vec<String>> {
        let cutoff = now - ttl_ms;
        let conn = self.w();
        let ids: Vec<String> = conn
            .prepare("SELECT id FROM blobs WHERE COALESCE(last_fetched, created) < ?1")?
            .query_map(params![cutoff], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if ids.is_empty() {
            return Ok(ids);
        }
        conn.execute(
            "DELETE FROM blob_rooms WHERE blob IN
               (SELECT id FROM blobs WHERE COALESCE(last_fetched, created) < ?1)",
            params![cutoff],
        )?;
        conn.execute("DELETE FROM blobs WHERE COALESCE(last_fetched, created) < ?1", params![cutoff])?;
        Ok(ids)
    }

    /// Delete expired invites + directory tokens. Always safe (expired rows are dead weight); the one
    /// retention task the janitor runs unconditionally. Returns the number of rows removed.
    pub fn sweep_expired(&self, now: i64) -> Result<usize> {
        let conn = self.w();
        let a = conn.execute("DELETE FROM invites WHERE expires < ?1", params![now])?;
        let b = conn.execute("DELETE FROM directory_tokens WHERE expires < ?1", params![now])?;
        Ok(a + b)
    }

    /// Reclaim freed pages when the DB is in `auto_vacuum = INCREMENTAL` mode (a no-op otherwise —
    /// e.g. an older file created before that pragma, which needs a one-time `VACUUM` to convert).
    pub fn incremental_vacuum(&self) -> Result<()> {
        let conn = self.w();
        conn.execute_batch("PRAGMA incremental_vacuum;")?;
        Ok(())
    }
}

/// Whether `agent` is already a member of `room`, on an already-held connection (the approval path
/// holds the writer lock, so it can't call [`Store::is_member`] which would re-lock).
fn member_exists(conn: &Connection, room: &str, agent: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM members WHERE room = ?1 AND agent = ?2",
        params![room, agent],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// Whether `owner` owns `room` (the set-once `rooms.owner`), on an already-held connection. The
/// authorization check behind viewing/resolving join requests.
fn room_owned_by(conn: &Connection, room: &str, owner: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM rooms WHERE name = ?1 AND owner = ?2",
        params![room, owner],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// Idempotently add a column to a table (SQLite has no `ADD COLUMN IF NOT EXISTS`), so an older
/// on-disk schema can gain a column without a destructive rebuild. Returns `true` iff it actually added
/// the column (so a caller can run a one-time backfill only on first upgrade). `table`/`col`/`decl` are
/// internal constants, never user input.
fn add_column_if_missing(conn: &Connection, table: &str, col: &str, decl: &str) -> Result<bool> {
    let present = conn
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<String>>>()?
        .iter()
        .any(|c| c == col);
    if !present {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {decl};"))?;
    }
    Ok(!present)
}

/// One-time backfill of the `messages.tokens` estimate for rows that predate the column: parse each
/// message's stored `parts`, estimate its tokens, and write them in a single transaction. Runs only the
/// first time the column is added (see [`Store::open`]); on a fresh DB there are no rows, so it is a
/// no-op. A row whose `parts` fail to parse is left at 0 rather than failing the whole migration.
fn backfill_message_tokens(conn: &Connection) -> Result<()> {
    let updates: Vec<(i64, i64)> = {
        let mut stmt = conn.prepare("SELECT seq, parts FROM messages")?;
        let mapped = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in mapped {
            let (seq, parts_json) = row?;
            let tokens = serde_json::from_str::<Vec<Part>>(&parts_json)
                .map(|p| estimate_message_tokens(&p) as i64)
                .unwrap_or(0);
            out.push((seq, tokens));
        }
        out
    };
    let tx = conn.unchecked_transaction()?;
    {
        let mut up = tx.prepare("UPDATE messages SET tokens = ?1 WHERE seq = ?2")?;
        for (seq, tokens) in &updates {
            up.execute(params![tokens, seq])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Total SQLite page-cache budget shared across the whole connection pool (1 writer + N readers), in
/// KiB. SQLite's `cache_size` is *per connection*, so this is divided by the connection count in
/// [`Store::open`] — otherwise a generous per-connection cache multiplies by the pool size (a
/// 9-connection pool at 64 MiB each reserved ~576 MiB of resident cache that filled the longer the hub
/// ran). 64 MiB total is ample for this workload and keeps the hub inside a small (256 MB) VM.
const TOTAL_CACHE_KIB: i64 = 65_536; // 64 MiB total across the pool
/// Floor on each connection's slice of the cache, so a large pool still leaves every connection a
/// usable working set rather than shrinking toward zero.
const MIN_CACHE_KIB_PER_CONN: i64 = 4_096; // 4 MiB

/// Per-connection pragmas. These are *not* persisted in the database file, so they must be set on
/// every connection that is opened (the writer + each pooled reader). `journal_mode = WAL` is a
/// database-level setting and stays in `MIGRATION` (applied once).
///
/// `synchronous = NORMAL` is the documented WAL "sweet spot": atomic, never-corrupting commits that
/// only risk losing the *last* transaction on OS/power loss — in exchange for a large write speedup
/// over the default `FULL` (which fsyncs on every commit). `cache_size` is the per-connection page
/// cache (a slice of [`TOTAL_CACHE_KIB`], so the pool's total stays bounded); the mmap/temp_store knobs
/// cut disk I/O; `foreign_keys = ON` enforces referential integrity for future cascades. The 128 MiB
/// mmap is file-backed and shared across connections mapping the same file (and reclaimable under
/// pressure), so unlike the page cache it doesn't multiply per connection.
fn configure_conn(conn: &Connection, cache_kib: i64) -> Result<()> {
    conn.execute_batch(&format!(
        "PRAGMA busy_timeout = 5000;
         PRAGMA synchronous  = NORMAL;
         PRAGMA cache_size   = -{cache_kib};
         PRAGMA temp_store   = MEMORY;
         PRAGMA mmap_size    = 134217728;
         PRAGMA foreign_keys = ON;",
    ))?;
    Ok(())
}

/// Create the vec0 virtual table for vector embeddings (idempotent).
fn create_vec_table(conn: &Connection, dim: usize) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_facts USING vec0(
           fact_id INTEGER PRIMARY KEY,
           embedding float[{dim}]
         );"
    ))?;
    Ok(())
}

/// Encode a float slice as little-endian bytes for sqlite-vec.
fn floats_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Reciprocal Rank Fusion: combine two ranked result lists into one.
fn rrf_fuse(fts: &[RecallHit], vec: &[RecallHit], limit: usize) -> Vec<RecallHit> {
    let mut scores: HashMap<String, (f64, RecallHit)> = HashMap::new();
    for (rank, hit) in fts.iter().enumerate() {
        let key = hit_key(hit);
        let rrf = 1.0 / (RRF_K + rank as f64 + 1.0);
        scores.entry(key).or_insert_with(|| (0.0, hit.clone())).0 += rrf;
    }
    for (rank, hit) in vec.iter().enumerate() {
        let key = hit_key(hit);
        let rrf = 1.0 / (RRF_K + rank as f64 + 1.0);
        let entry = scores.entry(key).or_insert_with(|| (0.0, hit.clone()));
        entry.0 += rrf;
    }
    let mut results: Vec<(f64, RecallHit)> = scores.into_values().collect();
    results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);
    results
        .into_iter()
        .map(|(s, mut h)| {
            h.score = s;
            h
        })
        .collect()
}

fn hit_key(hit: &RecallHit) -> String {
    format!(
        "{}|{}|{}|{}",
        hit.author,
        hit.ts,
        hit.room.as_deref().unwrap_or(""),
        &hit.text[..hit.text.len().min(128)]
    )
}

/// Build a safe FTS5 query: keep alphanumeric terms only, prefix-match each, OR them together.
fn build_fts_query(q: &str) -> String {
    let terms: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| format!("{}*", s.to_lowercase()))
        .collect();
    terms.join(" OR ")
}

/// The outcome of [`Store::redeem_invite`]: the resolved room + kind, and whether the redeem only
/// queued a **pending** approval request (`pending = true`, the caller is not yet a member) rather
/// than joining outright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redeemed {
    pub room: String,
    pub kind: RoomKind,
    pub pending: bool,
}

/// Metadata for a content-addressed blob (the bytes live on disk under `<blob_dir>/<id>`).
#[derive(Debug, Clone, PartialEq)]
pub struct BlobMeta {
    pub id: String,
    pub media_type: Option<String>,
    pub size: i64,
    pub created: i64,
}

struct RawMsg {
    seq: i64,
    id: String,
    room: String,
    author: String,
    name: String,
    role: Option<String>,
    parts: String,
    mentions: Option<String>,
    reply_to: Option<String>,
    ts: i64,
}

impl RawMsg {
    fn to_stored(&self) -> Result<StoredMessage> {
        let parts: Vec<Part> = serde_json::from_str(&self.parts)?;
        let mentions = match &self.mentions {
            Some(m) => Some(serde_json::from_str::<Vec<String>>(m)?),
            None => None,
        };
        Ok(StoredMessage {
            seq: self.seq,
            id: self.id.clone(),
            room: self.room.clone(),
            from: EndpointRef {
                id: self.author.clone(),
                name: self.name.clone(),
                role: self.role.clone(),
            },
            parts,
            mentions,
            reply_to: self.reply_to.clone(),
            ts: self.ts,
        })
    }
}

/// Lowercased, space-delimited `(tags, skills)` blobs derived from a card for `LIKE` filtering.
/// Leading/trailing spaces let a `%term%` pattern match a whole token.
fn card_filter_blobs(card: &AgentCard) -> (String, String) {
    let mut tags = String::new();
    if let Some(ts) = &card.tags {
        for t in ts {
            tags.push(' ');
            tags.push_str(&t.to_lowercase());
        }
        if !tags.is_empty() {
            tags.push(' ');
        }
    }
    let mut skills = String::new();
    if let Some(sk) = &card.skills {
        for s in sk {
            skills.push(' ');
            skills.push_str(&s.id.to_lowercase());
            skills.push(' ');
            skills.push_str(&s.name.to_lowercase());
        }
        if !skills.is_empty() {
            skills.push(' ');
        }
    }
    (tags, skills)
}

/// Raw directory columns, joined across `directory` + `agents` + `presence`.
struct RawDir {
    card_json: String,
    visibility: String,
    sig: Option<String>,
    verified: i64,
    raw_status: Option<String>,
    activity: Option<String>,
    presence_ts: Option<i64>,
    first_seen: i64,
    last_seen: i64,
}

impl RawDir {
    fn from_row(r: &rusqlite::Row) -> rusqlite::Result<RawDir> {
        Ok(RawDir {
            card_json: r.get(0)?,
            visibility: r.get(1)?,
            sig: r.get(2)?,
            verified: r.get(3)?,
            raw_status: r.get(4)?,
            activity: r.get(5)?,
            presence_ts: r.get(6)?,
            first_seen: r.get(7)?,
            last_seen: r.get(8)?,
        })
    }

    fn to_entry(&self, hub: &str, now: i64) -> Result<DirectoryEntry> {
        let card: AgentCard = serde_json::from_str(&self.card_json)?;
        // Self-reported status, decayed to `offline` once the heartbeat goes stale.
        let status = match (&self.raw_status, self.presence_ts) {
            (Some(s), Some(ts)) if now - ts <= PRESENCE_STALE_MS => s.clone(),
            _ => "offline".to_string(),
        };
        Ok(DirectoryEntry {
            card,
            visibility: Visibility::parse(&self.visibility).unwrap_or(Visibility::Private),
            status,
            activity: self.activity.clone(),
            hub: hub.to_string(),
            verified: self.verified != 0,
            sig: self.sig.clone(),
            first_seen: self.first_seen,
            last_seen: self.last_seen,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parler_protocol::{AgentCard, EndpointKind};

    fn eref(id: &str, name: &str) -> EndpointRef {
        EndpointRef { id: id.into(), name: name.into(), role: None }
    }

    #[test]
    fn open_creates_missing_parent_dir() {
        // A fresh `--db` path (e.g. a container volume mounted empty at /data) must open, not error.
        let base = std::env::temp_dir().join(format!("parler-store-{}", Uuid::new_v4()));
        let db = base.join("nested").join("hub.sqlite");
        assert!(!base.exists());
        let _s = Store::open(Some(&db)).unwrap();
        assert!(db.exists(), "db file should be created under the auto-made dir");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn file_backed_pool_reads_see_writes() {
        // A real file DB exercises the read-only connection pool (an in-memory DB falls back to the
        // writer). This is the safety net for the writer/reader split: any write mis-routed to a
        // read-only reader fails here, and every read must observe committed writes through the pool.
        let dir = std::env::temp_dir().join(format!("parler-pool-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("hub.sqlite");
        {
            let s = Store::open(Some(&db)).unwrap();
            // Writes — each must land on the single writer connection.
            s.upsert_agent("U_A", "alice", Some("planner"), 10).unwrap();
            s.upsert_agent("U_B", "bob", None, 10).unwrap();
            s.touch_presence("U_A", "working", Some("things"), 11).unwrap();
            s.ensure_room("team", RoomKind::Channel, None, 1).unwrap();
            s.add_member("team", "U_A", 1).unwrap();
            s.add_member("team", "U_B", 1).unwrap();
            s.append_message("team", &eref("U_A", "alice"), &[Part::text("hello world")], None, None, None, 20).unwrap();
            s.remember("U_A", &Fact { key: None, text: "deploy is blue-green".into(), room: Some("team".into()) }, 21, None, None).unwrap();
            s.register_card(&card("U_A", "alice", &["ops"], &["plan"]), Some("sig"), true, Visibility::Public, 12).unwrap();
            s.create_invite("CODE", "team", RoomKind::Channel, None, 1, 9_999_999_999_999, "U_A", false, 1).unwrap();
            s.mint_directory_token("TOK", "hub", 9_999_999_999_999, "U_A", 1).unwrap();
            s.put_blob_meta("blob1", "team", "U_A", None, 12, 30).unwrap();
            s.touch_blob_fetched("blob1", 31).unwrap();

            // Reads — these now run on read-only pool connections and must observe the writes above.
            assert!(s.is_member("team", "U_A").unwrap());
            assert_eq!(s.room_kind("team").unwrap(), Some(RoomKind::Channel));
            assert_eq!(s.roster("team", 20).unwrap().len(), 2);
            assert_eq!(s.rooms_of("U_A").unwrap().len(), 1);
            assert_eq!(s.recall("U_A", "deploy", Some("team"), None, None).unwrap().len(), 1);
            assert_eq!(s.discover(DiscoverScope::Public, "h", None, None, None, None, None, 20).unwrap().len(), 1);
            assert!(s.lookup_card("U_A", "h", false, 20).unwrap().is_some());
            assert_eq!(s.directory_counts().unwrap(), (1, 1));
            assert_eq!(s.directory_visibility("U_A").unwrap(), Some(Visibility::Public));
            assert!(s.validate_directory_token("TOK", 100).unwrap());
            assert_eq!(s.total_blob_bytes().unwrap(), 12);
            assert!(s.blob_meta("blob1").unwrap().is_some());
            assert!(s.blob_readable_by("blob1", "U_A").unwrap());
            assert_eq!(s.find_dm_room("U_A", "U_B").unwrap(), None); // a channel, not a dm

            // `pull` reads on the pool but advances the cursor on the writer.
            let (msgs, cur) = s.pull("team", "U_B", None, None, None).unwrap();
            assert_eq!(msgs.len(), 1);
            assert_eq!(cur, 1);
            let (again, _) = s.pull("team", "U_B", None, None, None).unwrap();
            assert!(again.is_empty(), "cursor advanced through the writer");

            // Read-then-write op, then confirm the new membership through the pool.
            s.upsert_agent("U_C", "carol", None, 2).unwrap();
            s.redeem_invite("CODE", "U_C", 2).unwrap();
            assert!(s.is_member("team", "U_C").unwrap());

            s.sweep_expired(1).unwrap();
            s.quick_check().unwrap();
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pool_total_page_cache_stays_bounded() {
        // The regression guard: `cache_size` is per-connection, so the pool's *summed* page-cache
        // budget — writer + every reader — must stay within the one shared budget, not 64 MiB × pool.
        let dir = std::env::temp_dir().join(format!("parler-cache-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("hub.sqlite");
        {
            let s = Store::open(Some(&db)).unwrap();
            // `cache_size` is stored as the negative KiB we set; sum |value| over every connection.
            let read_kib = |c: &Connection| -> i64 {
                let v: i64 = c.query_row("PRAGMA cache_size", [], |r| r.get(0)).unwrap();
                assert!(v < 0, "expected a KiB-denominated (negative) cache_size, got {v}");
                -v
            };
            let mut total = read_kib(&s.inner.writer.lock());
            for rc in &s.inner.readers {
                let per = read_kib(&rc.lock());
                assert!(per >= MIN_CACHE_KIB_PER_CONN, "each connection keeps a usable working set");
                total += per;
            }
            assert!(
                total <= TOTAL_CACHE_KIB,
                "pool cache {total} KiB exceeds the {TOTAL_CACHE_KIB} KiB budget ({} readers)",
                s.inner.readers.len()
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn quick_check_passes_on_fresh_db() {
        let s = Store::open(None).unwrap();
        s.quick_check().unwrap();
    }

    #[test]
    fn append_pull_advances_cursor() {
        let s = Store::open(None).unwrap();
        s.ensure_room("team", RoomKind::Channel, None, 1).unwrap();
        s.upsert_agent("U_A", "alice", None, 1).unwrap();
        s.upsert_agent("U_B", "bob", None, 1).unwrap();
        s.add_member("team", "U_A", 1).unwrap();
        s.add_member("team", "U_B", 1).unwrap();

        s.append_message("team", &eref("U_A", "alice"), &[Part::text("one")], None, None, None, 10).unwrap();
        s.append_message("team", &eref("U_A", "alice"), &[Part::text("two")], None, None, None, 11).unwrap();

        // Bob pulls: sees both, cursor now at 2.
        let (msgs, cursor) = s.pull("team", "U_B", None, None, None).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(cursor, 2);
        // A second pull (cursor-based) is empty — no re-reading history.
        let (msgs2, _) = s.pull("team", "U_B", None, None, None).unwrap();
        assert!(msgs2.is_empty());
        // An explicit `since` re-reads without moving the cursor.
        let (again, _) = s.pull("team", "U_B", Some(0), None, None).unwrap();
        assert_eq!(again.len(), 2);
    }

    #[test]
    fn ack_pull_defers_the_commit_and_closes_the_loss_window() {
        // #85: an ack-aware pull reads the batch but does NOT commit past it — so a batch whose reply
        // is lost on a drop is RE-READ on retry instead of silently skipped (the old advance-on-read
        // loss window). Write the failing assertion first: the ack-less pull below proves the skip.
        let s = Store::open(None).unwrap();
        s.ensure_room("team", RoomKind::Channel, None, 1).unwrap();
        s.upsert_agent("U_A", "alice", None, 1).unwrap();
        s.upsert_agent("U_B", "bob", None, 1).unwrap();
        s.add_member("team", "U_A", 1).unwrap();
        s.add_member("team", "U_B", 1).unwrap();
        for i in 1..=3 {
            s.append_message("team", &eref("U_A", "alice"), &[Part::text("m")], None, None, None, i).unwrap();
        }

        // Ack-aware pull (ack floor 0): returns all three, cursor tail is 3, but the commit is deferred.
        let (batch, cursor) = s.pull("team", "U_B", None, None, Some(0)).unwrap();
        assert_eq!(batch.len(), 3);
        assert_eq!(cursor, 3, "returned cursor is the batch tail");

        // The reply was "lost" (bob never acked). A retry with the SAME ack re-delivers the batch —
        // an advance-on-read pull would have skipped it. This is the loss-window fix.
        let (redelivered, _c) = s.pull("team", "U_B", None, None, Some(0)).unwrap();
        assert_eq!(redelivered.len(), 3, "un-acked batch is re-delivered, not skipped");

        // Once bob acks up to 3, those messages are committed and never re-read.
        let (after_ack, _c) = s.pull("team", "U_B", None, None, Some(3)).unwrap();
        assert!(after_ack.is_empty(), "acked messages are committed");
    }

    #[test]
    fn ack_less_pull_advances_on_read_exactly_as_before() {
        // Old-client compatibility (#85): a Pull without ack commits on read, so a second pull is
        // empty — byte-for-byte the prior behavior (and the loss window the ack path closes).
        let s = Store::open(None).unwrap();
        s.ensure_room("team", RoomKind::Channel, None, 1).unwrap();
        s.upsert_agent("U_B", "bob", None, 1).unwrap();
        s.add_member("team", "U_B", 1).unwrap();
        s.append_message("team", &eref("U_B", "bob"), &[Part::text("m")], None, None, None, 1).unwrap();
        s.append_message("team", &eref("U_B", "bob"), &[Part::text("m")], None, None, None, 2).unwrap();

        let (b1, cur) = s.pull("team", "U_B", None, None, None).unwrap();
        assert_eq!(b1.len(), 2);
        assert_eq!(cur, 2);
        let (b2, _c) = s.pull("team", "U_B", None, None, None).unwrap();
        assert!(b2.is_empty(), "ack-less pull advances on read, as before");
    }

    #[test]
    fn remember_recall_and_key_upsert() {
        let s = Store::open(None).unwrap();
        s.remember("U_A", &Fact { key: None, text: "deploy plan is blue-green".into(), room: None }, 1, None, None).unwrap();
        s.remember("U_A", &Fact { key: Some("db".into()), text: "uses postgres".into(), room: None }, 1, None, None).unwrap();
        s.remember("U_A", &Fact { key: Some("db".into()), text: "uses postgres 16".into(), room: None }, 2, None, None).unwrap();

        let hits = s.recall("U_A", "deploy", None, None, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].text.contains("blue-green"));

        // The keyed fact upserted (still one row), with the new text.
        let db = s.recall("U_A", "postgres", None, None, None).unwrap();
        assert_eq!(db.len(), 1);
        assert!(db[0].text.contains("16"));
    }

    #[test]
    fn recall_by_key_is_exact_and_beats_bm25_ranking() {
        // #91: a keyed fetch returns exactly the keyed fact, independent of FTS ranking — even in a
        // room where BM25 ranks a decoy above the real (longer) digest.
        let s = Store::open(None).unwrap();
        // The real digest under its key, with a long body (BM25 length-normalizes it downward).
        let digest = format!("SESSION DIGEST: {}", "auth done, next is billing. ".repeat(20));
        s.remember("U_A", &Fact { key: Some("session-digest".into()), text: digest, room: Some("team".into()) }, 1, None, None).unwrap();
        // Short, unkeyed decoys that match the sentinel query strongly.
        for i in 0..3 {
            s.remember("U_B", &Fact { key: None, text: "SESSION DIGEST".into(), room: Some("team".into()) }, 2 + i, None, None).unwrap();
        }

        // BM25 for the sentinel (limit 1) surfaces a decoy, not the keyed digest — the old heuristic's
        // fragility.
        let bm25 = s.recall("U_A", "SESSION DIGEST", Some("team"), Some(1), None).unwrap();
        assert_eq!(bm25.len(), 1);
        assert_ne!(bm25[0].key.as_deref(), Some("session-digest"), "BM25 top hit is a decoy");

        // Keyed fetch returns exactly the digest regardless of ranking.
        let keyed = s.recall_by_key("U_A", "session-digest", Some("team"), Some(1)).unwrap();
        assert_eq!(keyed.len(), 1);
        assert_eq!(keyed[0].key.as_deref(), Some("session-digest"));
        assert!(keyed[0].text.starts_with("SESSION DIGEST: auth done"));

        // An unknown key is empty (deterministic, not a fuzzy match), and the room scopes it.
        assert!(s.recall_by_key("U_A", "nonexistent", Some("team"), None).unwrap().is_empty());
        assert!(s.recall_by_key("U_A", "session-digest", Some("other-room"), None).unwrap().is_empty());
    }

    #[test]
    fn invite_redeem_joins_and_enforces_limits() {
        let s = Store::open(None).unwrap();
        s.ensure_room("dm.x", RoomKind::Dm, None, 1).unwrap();
        s.create_invite("CODE1", "dm.x", RoomKind::Dm, None, 1, 9_999_999_999_999, "U_A", false, 1).unwrap();

        let r = s.redeem_invite("CODE1", "U_B", 2).unwrap();
        assert_eq!(r.room, "dm.x");
        assert_eq!(r.kind, RoomKind::Dm);
        assert!(!r.pending, "an ordinary invite joins on the spot");
        assert!(s.is_member("dm.x", "U_B").unwrap());
        // Single-use invite is now spent.
        assert!(s.redeem_invite("CODE1", "U_C", 3).is_err());
    }

    #[test]
    fn approval_invite_gates_join_behind_owner_consent() {
        let s = Store::open(None).unwrap();
        // alice owns the room (as the hub does on invite creation); bob will ask to join.
        s.ensure_room("room.s", RoomKind::Channel, None, 1).unwrap();
        s.add_member("room.s", "U_ALICE", 1).unwrap();
        s.set_room_owner("room.s", "U_ALICE").unwrap();
        s.create_invite("KEY", "room.s", RoomKind::Channel, None, 50, 9_999_999_999_999, "U_ALICE", true, 1).unwrap();

        // Bob redeems → a *pending* request, NOT membership: he can't read the room yet.
        let r = s.redeem_invite("KEY", "U_BOB", 2).unwrap();
        assert!(r.pending);
        assert!(!s.is_member("room.s", "U_BOB").unwrap(), "a pending joiner is not a member");
        // Re-redeeming while pending is idempotent (still pending, no extra use / queue row).
        assert!(s.redeem_invite("KEY", "U_BOB", 3).unwrap().pending);

        // Only the owner can see the queue; a stranger (or non-owner) is refused.
        assert!(s.pending_join_requests("room.s", "U_EVE").is_err());
        let reqs = s.pending_join_requests("room.s", "U_ALICE").unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].agent, "U_BOB");

        // A non-owner cannot approve; the owner can.
        assert!(s.resolve_join("room.s", "U_EVE", "U_BOB", true, 4).is_err());
        assert!(s.resolve_join("room.s", "U_ALICE", "U_BOB", true, 4).unwrap());
        assert!(s.is_member("room.s", "U_BOB").unwrap(), "approval admits the requester");
        assert!(s.pending_join_requests("room.s", "U_ALICE").unwrap().is_empty());
        // A post-approval re-redeem is the idempotent member path (still not pending).
        assert!(!s.redeem_invite("KEY", "U_BOB", 5).unwrap().pending);
    }

    #[test]
    fn denied_join_request_is_terminal() {
        let s = Store::open(None).unwrap();
        s.ensure_room("room.s", RoomKind::Channel, None, 1).unwrap();
        s.set_room_owner("room.s", "U_ALICE").unwrap();
        s.create_invite("KEY", "room.s", RoomKind::Channel, None, 50, 9_999_999_999_999, "U_ALICE", true, 1).unwrap();

        s.redeem_invite("KEY", "U_EVE", 2).unwrap(); // pending
        assert!(!s.resolve_join("room.s", "U_ALICE", "U_EVE", false, 3).unwrap()); // deny
        assert!(!s.is_member("room.s", "U_EVE").unwrap());
        // Eve cannot re-request her way in past a denial.
        assert!(s.redeem_invite("KEY", "U_EVE", 4).is_err());
        assert!(s.pending_join_requests("room.s", "U_ALICE").unwrap().is_empty());
    }

    #[test]
    fn blob_meta_and_room_binding() {
        let s = Store::open(None).unwrap();
        s.ensure_room("dev", RoomKind::Channel, None, 1).unwrap();
        s.add_member("dev", "U_A", 1).unwrap();

        s.put_blob_meta("deadbeef", "dev", "U_A", Some("application/x-git-bundle"), 42, 1).unwrap();
        // Idempotent: same bytes + room re-recorded without error or duplication.
        s.put_blob_meta("deadbeef", "dev", "U_A", Some("application/x-git-bundle"), 42, 2).unwrap();

        let m = s.blob_meta("deadbeef").unwrap().unwrap();
        assert_eq!(m.size, 42);
        assert_eq!(m.media_type.as_deref(), Some("application/x-git-bundle"));
        assert!(s.blob_meta("nope").unwrap().is_none());

        // Members of a bound room can read; everyone else cannot.
        assert!(s.blob_readable_by("deadbeef", "U_A").unwrap());
        assert!(!s.blob_readable_by("deadbeef", "U_B").unwrap());

        // Binding the same blob to a second room a new member belongs to opens access for them.
        s.ensure_room("ops", RoomKind::Channel, None, 1).unwrap();
        s.add_member("ops", "U_B", 1).unwrap();
        s.put_blob_meta("deadbeef", "ops", "U_B", None, 42, 3).unwrap();
        assert!(s.blob_readable_by("deadbeef", "U_B").unwrap());
    }

    #[test]
    fn prune_messages_keeps_newest_and_respects_age() {
        let s = Store::open(None).unwrap();
        s.ensure_room("team", RoomKind::Channel, None, 1).unwrap();
        for i in 0..5 {
            s.append_message("team", &eref("U_A", "a"), &[Part::text("m")], None, None, None, 100 + i).unwrap();
        }
        // now=10_000, retain 1s ⇒ all five (ts 100..104) are "expired", but keep the newest 2.
        assert_eq!(s.prune_messages(1_000, 2, 10_000).unwrap(), 3);
        let (msgs, _) = s.pull("team", "U_A", Some(0), None, None).unwrap();
        assert_eq!(msgs.len(), 2);
        // Idempotent: a second pass removes nothing (only the keep floor remains).
        assert_eq!(s.prune_messages(1_000, 2, 20_000).unwrap(), 0);
        // A recent message is never pruned, even with keep=0.
        s.append_message("team", &eref("U_A", "a"), &[Part::text("fresh")], None, None, None, 19_900).unwrap();
        assert_eq!(s.prune_messages(1_000, 0, 20_000).unwrap(), 2); // the two old survivors go
        let (after, _) = s.pull("team", "U_A", Some(0), None, None).unwrap();
        assert_eq!(after.len(), 1);
    }

    #[test]
    fn prune_facts_keeps_newest_unkeyed_and_keeps_keyed() {
        let s = Store::open(None).unwrap();
        for i in 0..4 {
            s.remember("U_A", &Fact { key: None, text: format!("note {i}"), room: None }, 100 + i, None, None).unwrap();
        }
        s.remember("U_A", &Fact { key: Some("k".into()), text: "kept".into(), room: None }, 200, None, None).unwrap();
        // Keep the newest 1 unkeyed ⇒ remove 3; the keyed fact is untouched.
        assert_eq!(s.prune_facts(1).unwrap(), 3);
        assert_eq!(s.recall("U_A", "note", None, None, None).unwrap().len(), 1);
        assert_eq!(s.recall("U_A", "kept", None, None, None).unwrap().len(), 1);
    }

    #[test]
    fn gc_blobs_and_sweep_expired() {
        let s = Store::open(None).unwrap();
        s.ensure_room("dev", RoomKind::Channel, None, 1).unwrap();
        s.put_blob_meta("aa", "dev", "U_A", None, 10, 100).unwrap();
        // created at 100, ttl 1s, now 5_000 ⇒ stale; gc returns the id to unlink and drops the rows.
        assert_eq!(s.gc_blobs(1_000, 5_000).unwrap(), vec!["aa".to_string()]);
        assert!(s.blob_meta("aa").unwrap().is_none());
        assert!(!s.blob_readable_by("aa", "U_A").unwrap());
        // A fetch keeps a blob alive past the TTL.
        s.put_blob_meta("bb", "dev", "U_A", None, 10, 100).unwrap();
        s.touch_blob_fetched("bb", 4_900).unwrap();
        assert!(s.gc_blobs(1_000, 5_000).unwrap().is_empty());

        s.create_invite("C1", "dev", RoomKind::Channel, None, 1, 5_000, "U_A", false, 1).unwrap();
        assert_eq!(s.sweep_expired(10_000).unwrap(), 1); // invite expired at ts 5_000
    }

    #[test]
    fn find_dm_room_matches_exact_pair() {
        let s = Store::open(None).unwrap();
        s.ensure_room("dm.x", RoomKind::Dm, None, 1).unwrap();
        s.add_member("dm.x", "U_A", 1).unwrap();
        s.add_member("dm.x", "U_B", 1).unwrap();
        assert_eq!(s.find_dm_room("U_A", "U_B").unwrap().as_deref(), Some("dm.x"));
        assert_eq!(s.find_dm_room("U_A", "U_C").unwrap(), None);
    }

    fn card(id: &str, name: &str, tags: &[&str], skills: &[&str]) -> AgentCard {
        AgentCard {
            id: id.into(),
            name: name.into(),
            kind: EndpointKind::Agent,
            role: Some("planner".into()),
            description: None,
            tags: Some(tags.iter().map(|t| t.to_string()).collect()),
            skills: Some(
                skills
                    .iter()
                    .map(|k| parler_protocol::AgentSkill {
                        id: k.to_string(),
                        name: k.to_string(),
                        description: None,
                    })
                    .collect(),
            ),
            meta: None,
            protocol_version: None,
        }
    }

    #[test]
    fn register_and_discover_respects_scope_and_filters() {
        let s = Store::open(None).unwrap();
        s.upsert_agent("U_PUB", "alice", Some("planner"), 10).unwrap();
        s.upsert_agent("U_PRIV", "bob", None, 11).unwrap();
        s.touch_presence("U_PUB", "working", Some("planning"), 12).unwrap();

        s.register_card(&card("U_PUB", "alice", &["planning", "ops"], &["plan"]), Some("sig"), true, Visibility::Public, 12).unwrap();
        s.register_card(&card("U_PRIV", "bob", &["review"], &["audit"]), None, false, Visibility::Private, 13).unwrap();

        // `now` close to the presence ts so the working status is live (not decayed to offline).
        let now = 20;

        // Public scope sees only the public agent.
        let pubd = s.discover(DiscoverScope::Public, "hubz", None, None, None, None, None, now).unwrap();
        assert_eq!(pubd.len(), 1);
        assert_eq!(pubd[0].card.id, "U_PUB");
        assert!(pubd[0].verified);
        assert_eq!(pubd[0].status, "working");
        assert_eq!(pubd[0].hub, "hubz");

        // Hub scope sees both (same-hub view).
        assert_eq!(s.discover(DiscoverScope::Hub, "hubz", None, None, None, None, None, now).unwrap().len(), 2);

        // Tag/skill/text filters.
        let by_tag = s.discover(DiscoverScope::Hub, "hubz", None, Some("review"), None, None, None, now).unwrap();
        assert_eq!(by_tag.len(), 1);
        assert_eq!(by_tag[0].card.id, "U_PRIV");
        let by_skill = s.discover(DiscoverScope::Hub, "hubz", None, None, Some("plan"), None, None, now).unwrap();
        assert_eq!(by_skill.len(), 1);
        assert_eq!(by_skill[0].card.id, "U_PUB");
        let by_text = s.discover(DiscoverScope::Public, "hubz", Some("alice"), None, None, None, None, now).unwrap();
        assert_eq!(by_text.len(), 1);
        let by_status = s.discover(DiscoverScope::Hub, "hubz", None, None, None, Some("working"), None, now).unwrap();
        assert_eq!(by_status.len(), 1);
        assert_eq!(by_status[0].card.id, "U_PUB");

        // bob never reported presence, so he reads as offline (and the offline filter finds him).
        let offline = s.discover(DiscoverScope::Hub, "hubz", None, None, None, Some("offline"), None, now).unwrap();
        assert_eq!(offline.len(), 1);
        assert_eq!(offline[0].card.id, "U_PRIV");

        // Far in the future, even alice's working status has decayed to offline.
        let stale = s.discover(DiscoverScope::Public, "hubz", None, None, None, None, None, 12 + PRESENCE_STALE_MS + 1).unwrap();
        assert_eq!(stale[0].status, "offline");
    }

    #[test]
    fn lookup_respects_visibility_and_register_is_idempotent() {
        let s = Store::open(None).unwrap();
        s.upsert_agent("U_PRIV", "bob", None, 1).unwrap();
        s.register_card(&card("U_PRIV", "bob", &["x"], &[]), None, false, Visibility::Private, 1).unwrap();
        // Private card hidden from anonymous lookup, visible in hub scope.
        assert!(s.lookup_card("U_PRIV", "h", false, 1).unwrap().is_none());
        assert!(s.lookup_card("U_PRIV", "h", true, 1).unwrap().is_some());

        // Re-register flips visibility but keeps a single row + the original `registered` time.
        s.register_card(&card("U_PRIV", "bob", &["x"], &[]), None, false, Visibility::Public, 99).unwrap();
        assert!(s.lookup_card("U_PRIV", "h", false, 1).unwrap().is_some());
        assert_eq!(s.directory_counts().unwrap(), (1, 1));
    }

    #[test]
    fn directory_token_mint_validate_and_expiry() {
        let s = Store::open(None).unwrap();
        s.mint_directory_token("TKN", "hub", 1_000, "U_A", 1).unwrap();
        assert!(s.validate_directory_token("TKN", 500).unwrap());
        assert!(s.validate_directory_token("TKN", 1_000).unwrap());
        assert!(!s.validate_directory_token("TKN", 1_001).unwrap()); // expired
        assert!(!s.validate_directory_token("NOPE", 1).unwrap()); // unknown
    }

    #[test]
    fn watch_token_is_owner_only_room_scoped_and_distinct_from_directory_token() {
        let s = Store::open(None).unwrap();
        // A session room owned by alice; bob is an (approved) member but NOT the owner.
        s.ensure_room("room.s", RoomKind::Channel, None, 1).unwrap();
        s.add_member("room.s", "U_A", 1).unwrap();
        s.add_member("room.s", "U_B", 1).unwrap();
        s.set_room_owner("room.s", "U_A").unwrap();

        // Only the owner may mint a watch link; a non-owner member is refused.
        assert!(s.mint_watch_token("W_BOB", "room.s", "U_B", 10_000, 1).is_err());
        s.mint_watch_token("W_OK", "room.s", "U_A", 10_000, 1).unwrap();

        // It resolves to exactly its room, honors expiry, and an unknown token is None.
        assert_eq!(s.validate_watch_token("W_OK", 5_000).unwrap().as_deref(), Some("room.s"));
        assert_eq!(s.validate_watch_token("W_OK", 10_001).unwrap(), None); // expired
        assert_eq!(s.validate_watch_token("NOPE", 1).unwrap(), None);

        // Cross-scope must NOT hold: a watch token can't unlock the directory, and a directory token
        // can't be used as a watch token. (Both live in the same table — the scope is the wall.)
        assert!(!s.validate_directory_token("W_OK", 5_000).unwrap());
        s.mint_directory_token("D_OK", "hub", 10_000, "U_A", 1).unwrap();
        assert_eq!(s.validate_watch_token("D_OK", 5_000).unwrap(), None);
    }

    #[test]
    fn room_messages_reads_in_order_without_advancing_a_cursor() {
        let s = Store::open(None).unwrap();
        s.ensure_room("room.s", RoomKind::Channel, None, 1).unwrap();
        s.add_member("room.s", "U_A", 1).unwrap();
        s.append_message("room.s", &eref("U_A", "alice"), &[Part::text("one")], None, None, None, 10).unwrap();
        s.append_message("room.s", &eref("U_A", "alice"), &[Part::text("two")], None, None, None, 11).unwrap();

        let all = s.room_messages("room.s", 0, 100).unwrap();
        assert_eq!(all.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![1, 2]);
        // `since` filters to newer rows only.
        let tail = s.room_messages("room.s", 1, 100).unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].seq, 2);
        // The viewer is not a member, so its read must leave the member's pull cursor untouched.
        let (pulled, _cursor) = s.pull("room.s", "U_A", None, None, None).unwrap();
        assert_eq!(pulled.len(), 2, "member still sees the full backlog — room_messages didn't advance it");
    }

    #[test]
    fn room_stats_aggregates_tokens_messages_span_and_per_agent() {
        let s = Store::open(None).unwrap();
        // "hello world" = 11 chars → ceil(11/4) = 3 tokens; "hi" = 2 → 1; "bug report here" = 15 → 4.
        s.append_message("team", &eref("U_A", "alice"), &[Part::text("hello world")], None, None, None, 10).unwrap();
        s.append_message("team", &eref("U_A", "alice"), &[Part::text("hi")], None, None, None, 20).unwrap();
        s.append_message("team", &eref("U_B", "bob"), &[Part::text("bug report here")], None, None, None, 30).unwrap();

        let st = s.room_stats("team").unwrap();
        assert_eq!(st.messages, 3);
        assert_eq!(st.tokens, 3 + 1 + 4, "total = sum of the per-message estimates");
        assert_eq!(st.first_ts, Some(10));
        assert_eq!(st.last_ts, Some(30), "the activity span is MIN/MAX ts");

        // Per-agent, most tokens first; alice (4) ties bob (4) on tokens but wins the message-count
        // tiebreak (2 vs 1). Only display identity is exposed — the test never sees an agent id.
        assert_eq!(st.per_agent.len(), 2);
        assert_eq!((st.per_agent[0].name.as_str(), st.per_agent[0].messages, st.per_agent[0].tokens), ("alice", 2, 4));
        assert_eq!((st.per_agent[1].name.as_str(), st.per_agent[1].messages, st.per_agent[1].tokens), ("bob", 1, 4));

        // An unknown room is all-zeros with no span and no agents (not an error).
        let empty = s.room_stats("nope").unwrap();
        assert_eq!(empty, super::RoomStats::default());
    }

    #[test]
    fn append_message_dedupes_on_client_id_replay() {
        // #86: a retried send (same client_id) must return the ORIGINAL row, not insert a second.
        let s = Store::open(None).unwrap();
        let from = eref("U_A", "alice");
        let first = s
            .append_message("team", &from, &[Part::text("ship it")], None, None, Some("cid-1"), 10)
            .unwrap();
        assert!(!first.deduped, "the first send inserts");

        // Replay with the same key: same id + seq, flagged deduped, tokens 0 (no double count).
        let replay = s
            .append_message("team", &from, &[Part::text("ship it")], None, None, Some("cid-1"), 99)
            .unwrap();
        assert!(replay.deduped, "the replay is recognized as a duplicate");
        assert_eq!(replay.id, first.id, "same message id returned");
        assert_eq!(replay.seq, first.seq, "same seq returned");
        assert_eq!(replay.tokens, 0, "a dedup contributes no new tokens");

        // Exactly one row exists.
        let pulled = s.room_messages("team", 0, 1000).unwrap();
        assert_eq!(pulled.len(), 1, "the retry did NOT double-post");

        // A different key from the same author inserts a distinct row; and NULL keys never dedup.
        let other = s
            .append_message("team", &from, &[Part::text("again")], None, None, Some("cid-2"), 11)
            .unwrap();
        assert!(!other.deduped);
        let unkeyed_a = s.append_message("team", &from, &[Part::text("x")], None, None, None, 12).unwrap();
        let unkeyed_b = s.append_message("team", &from, &[Part::text("x")], None, None, None, 13).unwrap();
        assert!(!unkeyed_a.deduped && !unkeyed_b.deduped, "unkeyed sends always insert");
        assert_ne!(unkeyed_a.seq, unkeyed_b.seq);
    }

    // ---- vector / semantic recall ----

    fn make_embedding(dim: usize, seed: f32) -> Vec<f32> {
        (0..dim).map(|i| (seed + i as f32 * 0.01).sin()).collect()
    }

    #[test]
    fn vector_recall_returns_nearest_facts() {
        let s = Store::open(None).unwrap();
        let dim = s.vec_dimension();
        let emb_a = make_embedding(dim, 1.0);
        let emb_b = make_embedding(dim, 2.0);
        let emb_c = make_embedding(dim, 1.01); // very close to emb_a

        s.remember("U_A", &Fact { key: None, text: "alpha fact".into(), room: None }, 1, Some(&emb_a), Some("test-model")).unwrap();
        s.remember("U_A", &Fact { key: None, text: "beta fact".into(), room: None }, 2, Some(&emb_b), None).unwrap();
        s.remember("U_A", &Fact { key: None, text: "gamma fact".into(), room: None }, 3, Some(&emb_c), None).unwrap();

        // Vector-only recall (empty text query) with a vector close to emb_a.
        let query_emb = make_embedding(dim, 1.005);
        let hits = s.recall("U_A", "", None, None, Some(&query_emb)).unwrap();
        assert!(!hits.is_empty());
        // The closest should be alpha or gamma (both near seed 1.0), not beta (seed 2.0).
        assert!(hits[0].text.contains("alpha") || hits[0].text.contains("gamma"));
    }

    #[test]
    fn hybrid_recall_fuses_fts_and_vector() {
        let s = Store::open(None).unwrap();
        let dim = s.vec_dimension();
        let emb_a = make_embedding(dim, 1.0);
        let emb_b = make_embedding(dim, 5.0);

        // "deploy" is a keyword hit; emb_b is far from query vector.
        s.remember("U_A", &Fact { key: None, text: "deploy strategy is blue-green".into(), room: None }, 1, Some(&emb_b), None).unwrap();
        // "scaling plan" won't match "deploy" keyword; emb_a is close to query vector.
        s.remember("U_A", &Fact { key: None, text: "scaling plan for the cluster".into(), room: None }, 2, Some(&emb_a), None).unwrap();

        let query_emb = make_embedding(dim, 1.001);
        let hits = s.recall("U_A", "deploy", None, None, Some(&query_emb)).unwrap();
        // Both should appear: one from FTS (deploy), one from vector (scaling).
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn embedding_dimension_mismatch_errors() {
        let s = Store::open(None).unwrap();
        let wrong_dim = vec![0.1_f32; 100]; // not 768
        let result = s.remember("U_A", &Fact { key: None, text: "x".into(), room: None }, 1, Some(&wrong_dim), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("dimension mismatch"));

        let result = s.recall("U_A", "x", None, None, Some(&wrong_dim));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("dimension mismatch"));
    }

    #[test]
    fn keyed_upsert_replaces_embedding() {
        let s = Store::open(None).unwrap();
        let dim = s.vec_dimension();
        let emb_old = make_embedding(dim, 1.0);
        let emb_new = make_embedding(dim, 5.0);

        s.remember("U_A", &Fact { key: Some("k".into()), text: "v1".into(), room: None }, 1, Some(&emb_old), None).unwrap();
        // Upsert with new embedding.
        s.remember("U_A", &Fact { key: Some("k".into()), text: "v2".into(), room: None }, 2, Some(&emb_new), None).unwrap();

        // Query with a vector close to the NEW embedding — should find the updated fact.
        let q = make_embedding(dim, 5.001);
        let hits = s.recall("U_A", "", None, None, Some(&q)).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].text.contains("v2"));
    }

    #[test]
    fn vector_recall_respects_room_scope() {
        let s = Store::open(None).unwrap();
        let dim = s.vec_dimension();
        let emb = make_embedding(dim, 1.0);

        s.ensure_room("team", RoomKind::Channel, None, 1).unwrap();
        s.add_member("team", "U_A", 1).unwrap();
        s.ensure_room("secret", RoomKind::Channel, None, 1).unwrap();

        // Fact in "team" (U_A is a member).
        s.remember("U_A", &Fact { key: None, text: "visible".into(), room: Some("team".into()) }, 1, Some(&emb), None).unwrap();
        // Fact in "secret" (U_A is NOT a member).
        s.remember("U_B", &Fact { key: None, text: "hidden".into(), room: Some("secret".into()) }, 2, Some(&emb), None).unwrap();

        let q = make_embedding(dim, 1.001);
        // Unscoped recall for U_A should only see "team" facts + private facts.
        let hits = s.recall("U_A", "", None, None, Some(&q)).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].text.contains("visible"));

        // Room-scoped recall in "team".
        let hits = s.recall("U_A", "", Some("team"), None, Some(&q)).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn prune_facts_also_cleans_vec_facts() {
        let s = Store::open(None).unwrap();
        let dim = s.vec_dimension();
        let emb = make_embedding(dim, 1.0);

        for i in 0..4 {
            s.remember("U_A", &Fact { key: None, text: format!("note {i}"), room: None }, 100 + i, Some(&emb), None).unwrap();
        }
        // Keep the newest 1 unkeyed ⇒ remove 3 (and their vec entries).
        assert_eq!(s.prune_facts(1).unwrap(), 3);

        let q = make_embedding(dim, 1.001);
        let hits = s.recall("U_A", "", None, None, Some(&q)).unwrap();
        assert_eq!(hits.len(), 1, "vec_facts should be pruned in sync with facts");
    }

    #[test]
    fn recall_without_embedding_is_pure_bm25() {
        let s = Store::open(None).unwrap();
        let dim = s.vec_dimension();
        let emb = make_embedding(dim, 1.0);
        s.remember("U_A", &Fact { key: None, text: "deploy strategy".into(), room: None }, 1, Some(&emb), None).unwrap();

        // No embedding in recall → pure FTS, same behavior as before.
        let hits = s.recall("U_A", "deploy", None, None, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].text.contains("deploy"));
    }
}
