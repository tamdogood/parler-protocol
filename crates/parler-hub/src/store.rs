//! The hub's durable store — embedded SQLite.
//!
//! Holds everything the bus needs to survive a restart: agents, rooms + membership, the per-room
//! message log (keyed by a monotonic `seq` that is also the cursor unit), the full-text `facts`
//! memory, and outstanding invites. Access is serialized through one connection behind a `Mutex`;
//! every method here is synchronous and never held across an `.await`, so the async server can call
//! it directly.

use anyhow::{anyhow, bail, Result};
use parler_protocol::{EndpointRef, Fact, Part, RecallHit, RoomInfo, RoomKind, RosterEntry, StoredMessage};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const MIGRATION: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 3000;

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
  ts          INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_room_seq ON messages(room, seq);

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
  created    INTEGER NOT NULL
);
"#;

/// The durable store. Cheaply cloneable (shares one connection behind an `Arc<Mutex<…>>`).
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    /// Open the store at `path`, or in-memory (lost on exit) when `path` is `None`. Runs migrations.
    pub fn open(path: Option<&Path>) -> Result<Store> {
        let conn = match path {
            Some(p) => Connection::open(p)?,
            None => Connection::open_in_memory()?,
        };
        conn.execute_batch(MIGRATION)?;
        Ok(Store {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    // ---- agents / presence ----

    pub fn upsert_agent(&self, id: &str, name: &str, role: Option<&str>, now: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO agents (id, name, role, first_seen, last_seen) VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, role = excluded.role, last_seen = excluded.last_seen",
            params![id, name, role, now],
        )?;
        Ok(())
    }

    pub fn touch_presence(&self, agent: &str, status: &str, activity: Option<&str>, now: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO presence (agent, status, activity, ts) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent) DO UPDATE SET status = excluded.status, activity = excluded.activity, ts = excluded.ts",
            params![agent, status, activity, now],
        )?;
        Ok(())
    }

    // ---- rooms / membership ----

    pub fn ensure_room(&self, name: &str, kind: RoomKind, description: Option<&str>, now: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO rooms (name, kind, description, created) VALUES (?1, ?2, ?3, ?4)",
            params![name, kind.as_str(), description, now],
        )?;
        Ok(())
    }

    pub fn add_member(&self, room: &str, agent: &str, now: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO members (room, agent, joined, cursor) VALUES (?1, ?2, ?3, 0)",
            params![room, agent, now],
        )?;
        Ok(())
    }

    pub fn is_member(&self, room: &str, agent: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM members WHERE room = ?1 AND agent = ?2",
            params![room, agent],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn room_kind(&self, name: &str) -> Result<Option<RoomKind>> {
        let conn = self.conn.lock().unwrap();
        let k: Option<String> = conn
            .query_row("SELECT kind FROM rooms WHERE name = ?1", params![name], |r| r.get(0))
            .optional()?;
        Ok(k.and_then(|s| RoomKind::parse(&s)))
    }

    /// The one DM room shared by exactly `a` and `b` (i.e. a 2-member `dm` room), if any.
    pub fn find_dm_room(&self, a: &str, b: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
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

    pub fn roster(&self, room: &str) -> Result<Vec<RosterEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT a.id, a.name, a.role, p.status, p.activity, p.ts, a.last_seen
               FROM members mb JOIN agents a ON a.id = mb.agent
               LEFT JOIN presence p ON p.agent = a.id
              WHERE mb.room = ?1
              ORDER BY a.name",
        )?;
        let rows = stmt
            .query_map(params![room], |r| {
                let status: Option<String> = r.get(3)?;
                let p_ts: Option<i64> = r.get(5)?;
                let last_seen: i64 = r.get(6)?;
                Ok(RosterEntry {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    role: r.get(2)?,
                    status: status.unwrap_or_else(|| "offline".into()),
                    activity: r.get(4)?,
                    last_seen: p_ts.unwrap_or(last_seen),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ---- messages ----

    pub fn append_message(
        &self,
        room: &str,
        from: &EndpointRef,
        parts: &[Part],
        mentions: Option<&[String]>,
        reply_to: Option<&str>,
        ts: i64,
    ) -> Result<(String, i64)> {
        let id = Uuid::now_v7().to_string();
        let parts_json = serde_json::to_string(parts)?;
        let mentions_json = match mentions {
            Some(m) => Some(serde_json::to_string(m)?),
            None => None,
        };
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO messages (id, room, author, author_name, author_role, parts, mentions, reply_to, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![id, room, from.id, from.name, from.role, parts_json, mentions_json, reply_to, ts],
        )?;
        Ok((id, conn.last_insert_rowid()))
    }

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
    ) -> Result<(Vec<StoredMessage>, i64)> {
        let conn = self.conn.lock().unwrap();
        let cur = match since {
            Some(s) => s,
            None => Self::get_cursor(&conn, room, agent)?,
        };
        let lim = limit.unwrap_or(200).min(1000) as i64;
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
        let new_cursor = raws.last().map(|r| r.seq).unwrap_or(cur);
        if since.is_none() && new_cursor > cur {
            conn.execute(
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
        now: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO invites (code, room, kind, role, max_uses, uses, expires, created_by, created)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8)",
            params![code, room, kind.as_str(), role, max_uses, expires, created_by, now],
        )?;
        Ok(())
    }

    /// Redeem `code` for `agent`: validate expiry + remaining uses, increment uses, join the room.
    pub fn redeem_invite(&self, code: &str, agent: &str, now: i64) -> Result<(String, RoomKind)> {
        let conn = self.conn.lock().unwrap();
        let row: Option<(String, String, i64, i64, i64)> = conn
            .query_row(
                "SELECT room, kind, max_uses, uses, expires FROM invites WHERE code = ?1",
                params![code],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let (room, kind_s, max_uses, uses, expires) =
            row.ok_or_else(|| anyhow!("invalid or unknown invite code"))?;
        if now > expires {
            bail!("invite has expired");
        }
        if uses >= max_uses {
            bail!("invite has already been used up");
        }
        conn.execute("UPDATE invites SET uses = uses + 1 WHERE code = ?1", params![code])?;
        conn.execute(
            "INSERT OR IGNORE INTO members (room, agent, joined, cursor) VALUES (?1, ?2, ?3, 0)",
            params![room, agent, now],
        )?;
        let kind = RoomKind::parse(&kind_s).unwrap_or(RoomKind::Channel);
        Ok((room, kind))
    }

    // ---- memory (facts) ----

    /// Write a fact. With a `key`, this upserts within (author, room, key) — idempotent updates.
    pub fn remember(&self, author: &str, fact: &Fact, ts: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        match &fact.key {
            Some(k) => {
                let updated = conn.execute(
                    "UPDATE facts SET text = ?1, ts = ?2
                       WHERE author = ?3 AND IFNULL(room, '') = IFNULL(?4, '') AND fkey = ?5",
                    params![fact.text, ts, author, fact.room, k],
                )?;
                if updated == 0 {
                    conn.execute(
                        "INSERT INTO facts (fkey, room, author, text, ts) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![k, fact.room, author, fact.text, ts],
                    )?;
                }
            }
            None => {
                conn.execute(
                    "INSERT INTO facts (fkey, room, author, text, ts) VALUES (NULL, ?1, ?2, ?3, ?4)",
                    params![fact.room, author, fact.text, ts],
                )?;
            }
        }
        Ok(())
    }

    /// Full-text recall. Scoped to `room` when given, else the agent's reachable memory (its own
    /// private facts plus every room it belongs to). Ordered by relevance (BM25; lower is better).
    pub fn recall(&self, agent: &str, query: &str, room: Option<&str>, limit: Option<u32>) -> Result<Vec<RecallHit>> {
        let match_q = build_fts_query(query);
        if match_q.is_empty() {
            return Ok(vec![]);
        }
        let lim = limit.unwrap_or(8).min(50) as i64;
        let conn = self.conn.lock().unwrap();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn eref(id: &str, name: &str) -> EndpointRef {
        EndpointRef { id: id.into(), name: name.into(), role: None }
    }

    #[test]
    fn append_pull_advances_cursor() {
        let s = Store::open(None).unwrap();
        s.ensure_room("team", RoomKind::Channel, None, 1).unwrap();
        s.upsert_agent("U_A", "alice", None, 1).unwrap();
        s.upsert_agent("U_B", "bob", None, 1).unwrap();
        s.add_member("team", "U_A", 1).unwrap();
        s.add_member("team", "U_B", 1).unwrap();

        s.append_message("team", &eref("U_A", "alice"), &[Part::text("one")], None, None, 10).unwrap();
        s.append_message("team", &eref("U_A", "alice"), &[Part::text("two")], None, None, 11).unwrap();

        // Bob pulls: sees both, cursor now at 2.
        let (msgs, cursor) = s.pull("team", "U_B", None, None).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(cursor, 2);
        // A second pull (cursor-based) is empty — no re-reading history.
        let (msgs2, _) = s.pull("team", "U_B", None, None).unwrap();
        assert!(msgs2.is_empty());
        // An explicit `since` re-reads without moving the cursor.
        let (again, _) = s.pull("team", "U_B", Some(0), None).unwrap();
        assert_eq!(again.len(), 2);
    }

    #[test]
    fn remember_recall_and_key_upsert() {
        let s = Store::open(None).unwrap();
        s.remember("U_A", &Fact { key: None, text: "deploy plan is blue-green".into(), room: None }, 1).unwrap();
        s.remember("U_A", &Fact { key: Some("db".into()), text: "uses postgres".into(), room: None }, 1).unwrap();
        s.remember("U_A", &Fact { key: Some("db".into()), text: "uses postgres 16".into(), room: None }, 2).unwrap();

        let hits = s.recall("U_A", "deploy", None, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].text.contains("blue-green"));

        // The keyed fact upserted (still one row), with the new text.
        let db = s.recall("U_A", "postgres", None, None).unwrap();
        assert_eq!(db.len(), 1);
        assert!(db[0].text.contains("16"));
    }

    #[test]
    fn invite_redeem_joins_and_enforces_limits() {
        let s = Store::open(None).unwrap();
        s.ensure_room("dm.x", RoomKind::Dm, None, 1).unwrap();
        s.create_invite("CODE1", "dm.x", RoomKind::Dm, None, 1, 9_999_999_999_999, "U_A", 1).unwrap();

        let (room, kind) = s.redeem_invite("CODE1", "U_B", 2).unwrap();
        assert_eq!(room, "dm.x");
        assert_eq!(kind, RoomKind::Dm);
        assert!(s.is_member("dm.x", "U_B").unwrap());
        // Single-use invite is now spent.
        assert!(s.redeem_invite("CODE1", "U_C", 3).is_err());
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
}
