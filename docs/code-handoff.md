# Parler Protocol Code Handoff — passing work, not just words

**Status: BUILT (Phase 1 + Phase 2), 2026-06-27.** Borrowed from
[ottogin/agenthub](https://github.com/ottogin/agenthub). Try it: `parler push` / `parler fetch` /
`parler apply` (and the `parler_push` / `parler_fetch` / `parler_apply` MCP tools). Phase 3
(frontier) is deferred.

This artifact-handoff primitive lets two Parler Protocol agents exchange the **change itself**, not
just talk about it or write text **facts**. One agent pushes a **git bundle** into a room; a peer can
fetch it and import the exact commits into an isolated ref.

```
   alice ──push (git bundle)──► parler-hub ──(blob on disk)        a handoff is an ordinary room
                                    │                              message carrying a bundle Part,
   bob   ──recv──► sees a com.parler.bundle Part  ──fetch──► bytes so send/recv/cursor/wake all
         ──apply──► git bundle unbundle into refs/parler/*         work unchanged.
```

## The one design decision: split *bytes* from *reference*

agenthub keeps a whole GitHub-replacement (a commit DAG with `children`/`lineage`/`diff`, browsable
in a UI). **We borrow the transport, not the metaphor.** The bundle is an **opaque artifact** the hub
moves; all git semantics stay on the agents' machines.

A handoff is two things:

1. **The blob** — the bundle bytes, stored **content-addressed** (`id = sha256(bytes)`) on the hub's
   disk and bound to the room it was posted to. Dedups; tamper-evident; the hub never executes it.
2. **The reference** — an ordinary room message carrying a `Part::Extension`:

   ```json
   { "kind": "com.parler.bundle", "blob": "<sha256>", "vcs": "git",
     "tip": "<commit>", "base": "<base commit|null>", "summary": "feat: add X",
     "size": 12345, "mediaType": "application/x-git-bundle" }
   ```

This rides the machinery Parler Protocol already has — `Part::Extension` is first-class on the wire (see the
`com.acme.snapshot` example in `types.rs`), so **`send`/`recv`, the per-room cursor, durability,
reconnect-resume, and the Stop-hook "wake" all work with zero changes**. An old client that doesn't
know `com.parler.bundle` just sees an extension part it can render as `[bundle: feat: add X]`.

> **Why git bundles** (the borrow): one self-contained file carrying commits + ancestry (or a thin
> `base..HEAD` slice); no live git server needed; the receiver applies locally. The `vcs`/`mediaType`
> fields keep it open to patches or tarballs later, but git bundle is first-class.

## Transport: WS-binary, not a second HTTP channel

agenthub ships bytes over HTTP (`POST /api/git/push`, `GET /api/git/fetch/{hash}`). We **don't**, for
MVP — the WebSocket already carries an *authenticated* connection (nkey challenge-response) and
already supports binary frames (`handle_socket` just ignores them today). Reusing it means:

- **no new dependency** (no `reqwest`/HTTP client in the connector),
- **no second auth path** — authorization is room membership on the already-proven socket, so **no
  capability-token table** is needed (unlike the directory tokens),
- **one code path**.

Upload (one round-trip + one binary frame):

```
client → PutBlob { room, sha256, size, mediaType }
hub    → BlobReady { id }            # member + size OK; expect the bytes next
client → <Binary frame: the bundle>  # MVP: whole blob in one frame, ≤ max_blob_bytes
hub    → BlobStored { id }           # verified sha256(bytes)==id && len==size; written to disk
```

Download:

```
client → GetBlob { id }              # hub checks caller is a member of the blob's room
hub    → BlobIncoming { id, size }
hub    → <Binary frame: the bundle>
```

The handoff message itself is still posted with the **existing** `Send` (a bundle `Part`), and read
with the **existing** `Pull`. Only blob movement is new.

> **Partially shipped:** a session watch token can download only blobs referenced by that exact room
> through the scoped viewer endpoint. General agent HTTP upload/download, resumable transfers, and
> chunked or streamed uploads remain deferred. Agent transport continues to use authenticated
> WebSocket binary frames.

## What changes, file by file

### `crates/parler-protocol/src/hub.rs`
- `ClientFrame`: `PutBlob { room, sha256, size, media_type }`, `GetBlob { id }`.
- `ServerFrame`: `BlobReady { id }`, `BlobStored { id }`, `BlobIncoming { id, size }`.
- A `BUNDLE_KIND = "com.parler.bundle"` const + a `BundleRef` struct with `to_part() -> Part` and
  `from_part(&Part) -> Option<BundleRef>` (build/parse the `Part::Extension`). Round-trip test.
- (Extensions are forward-compatible; a `PROTOCOL_VERSION` bump is optional.)

### `crates/parler-hub/src/store.rs`
- New table — metadata only; **bytes live on disk**, keeping the DB small (mirrors agenthub keeping
  git data out of SQLite):
  ```sql
  CREATE TABLE IF NOT EXISTS blobs (
    id         TEXT PRIMARY KEY,   -- sha256 hex (content address)
    room       TEXT NOT NULL,      -- authorization scope (membership of this room)
    author     TEXT NOT NULL,
    media_type TEXT,
    size       INTEGER NOT NULL,
    created    INTEGER NOT NULL
  );
  ```
- `BlobMeta`, `put_blob_meta(...)`, `blob_meta(id) -> Option<BlobMeta>`. Download auth reuses
  `is_member(blob.room, caller)`.

### `crates/parler-hub/src/server.rs`
- A tiny per-connection state addition: after a `PutBlob` ack, the connection is "awaiting one blob
  upload" (carry `Option<PendingUpload{ id, room, size, media_type }>` in `ConnState`); the next
  `WsMessage::Binary` is consumed as that blob (verify `sha256`+len, reject oversize, write
  `blob_dir/<id>`, `put_blob_meta`, reply `BlobStored`). Any other frame while pending = error/reset.
- `GetBlob`: membership check → reply `BlobIncoming` then `socket.send(Binary(bytes))`.
- This is the **only** place the socket loop grows beyond pure request/reply; keep it bounded
  (single-frame, size-capped).

### `crates/parler-hub/src/lib.rs` + `main.rs`
- `HubState` gains `blob_dir: PathBuf` and `max_blob_bytes: u64`. Defaults: `blob_dir` next to the
  sqlite file (`<db>.blobs/`, or a temp dir for in-memory); `max_blob_bytes = 25 * 1024 * 1024`.
- New `parler-hub` flags/env: `--blob-dir` / `PARLER_HUB_BLOB_DIR`, `--max-blob-bytes`. Create the dir
  on boot.

### `crates/parler-connector/src/client.rs`
- `HubClient::send_binary(&[u8])` and `recv_binary() -> Vec<u8>` (the stream is already owned here).

### `crates/parler-connector/src/agent.rs`
- `BundleMeta { vcs, tip, base, summary, media_type }`.
- `push(target, bundle: &[u8], meta) -> Result<(msg_id, blob_id)>`: `PutBlob` → `send_binary` →
  await `BlobStored` → `send` a message whose parts include `BundleRef::to_part()`.
- `fetch_blob(id) -> Result<Vec<u8>>`: `GetBlob` → `recv_binary`. **Bytes only — all git lives in the
  CLI.**

### `crates/parler-cli/src/lib.rs`
- `parler push (--room R | --to ID | --service S) [--base <ref>] [--summary <s>] [<gitref>]`
  - builds a bundle locally via `std::process::Command`: `git bundle create <tmp> <range>`, where
    `range` is `<base>..HEAD` when `--base` is given (a patch series), else `HEAD` (full history to
    the tip). Reads the temp file, fills `BundleMeta` (`tip = git rev-parse HEAD`), calls `push`.
- `parler fetch <blobId> [-o file.bundle]` — retrieve bytes only.
- `parler apply <blobId>` — `git bundle verify`, then `git fetch <tmp> '*:refs/parler/<author>/*'`,
  and print what landed + the tip. **Never auto-merges into the working tree** (a hard-to-reverse
  action stays an explicit, separate `git merge`/`cherry-pick` the human runs).
- `parler recv` learns to render a `com.parler.bundle` part as
  `📦 bundle "<summary>" (<tip>, <size>) — parler fetch <blob> / parler apply <blob>`.

### `crates/parler-cli/src/mcp.rs`
- `parler_push`, `parler_fetch`, and `parler_apply` tools. `parler_apply` verifies the bundle and
  imports it under `refs/parler/<blob-prefix>` in an explicit repository path. It never merges,
  checks out, or modifies the working tree.

## Borrow #2 — defense (agenthub has it; Parler Protocol doesn't)

agenthub enforces max bundle size (50MB) + pushes/hr + posts/hr per agent. Parler Protocol has none. Add:
- `max_blob_bytes` (above), enforced at `PutBlob` and on the received frame.
- Per-agent in-memory **token buckets** on `HubState` (`Mutex<HashMap<id, RateState>>`): sends/min and
  blobs/hour. Reset on restart — same simple posture as agenthub. Cheap, and the first thing you want
  the moment a hub is public. Flags: `--max-sends-per-min`, `--max-blobs-per-hour`.

## Borrow #3 — frontier (optional, phase 3)

Not a git DAG — a lightweight "what's the current tip in this room": because bundle parts carry
`tip`/`summary`, index the latest per room and answer `parler frontier --room R` →
`latest: "<summary>" @ <tip> by <author>`. Helps agents avoid clobbering each other's work. Could
surface in `rooms` output and on the website. Cheap; defer until handoff is in use.

## Explicitly NOT borrowed (keep the chat-protocol focus)

- No server-side bare repo / commit DAG / `children`/`lineage`/`diff` REST — the GitHub-clone path.
- No in-browser code UI (commit tree / diff / blame): the read-only **session viewer** lets a watcher
  *download* an exchanged bundle, but never renders or diffs it.
- No auto-merge into a working tree.

## Security model

| Property | How |
|---|---|
| **Integrity** | `id = sha256(bytes)`; the hub rejects a blob whose bytes don't hash to the declared id. Content-addressing dedups and is tamper-evident. |
| **Authorization** | Pure room membership (existing model). A blob is bound to its room; only members (`is_member`) can `GetBlob` it. No new ACL concept. |
| **Narrow viewer auth** | Agent bytes ride the authenticated WebSocket. A session watch token can download only blobs referenced by its exact room. |
| **Bounded** | `max_blob_bytes` + per-agent rate limits. |
| **Hub never executes** | The bundle is opaque bytes; no `git` on the server in MVP ⇒ no server-side git RCE surface. |
| **Apply is isolated** | CLI and MCP apply fetch into `refs/parler/*`; neither merges, checks out, or changes the working tree. |

## Compatibility & phasing

Purely **additive**: new frames, one new table, new CLI verbs, a new *extension kind*. No breaking
changes; old clients degrade gracefully on the unknown part.

- **Phase 1 — blob handoff (MVP):** the frames, blob store, WS-binary transport, `agent.push/fetch`,
  CLI `push`/`fetch`/`apply`, the `com.parler.bundle` convention, end-to-end test.
- **Phase 2 — defense:** size cap + per-agent rate limits.
- **Phase 3 — frontier (optional):** latest-tip-per-room index + `parler frontier`.

## Decisions made (as built)

1. **Transport: WS-binary** — agent bytes ride the already-authenticated WebSocket as a single
   binary frame. The later session-viewer endpoint is read-only, watch-token-scoped, and limited to
   blobs referenced by the viewer's exact room.
2. **Single-frame blob** — a `PutBlob` declares `size`+`sha256`; the next binary frame must be exactly
   those bytes. Chunked upload is deferred.
3. **`max_blob_bytes` = 25 MiB** default (`DEFAULT_MAX_BLOB_BYTES`), overridable via
   `--max-blob-bytes` / `PARLER_HUB_MAX_BLOB_BYTES`.
4. **Phase 3 (frontier) deferred** — kept out to stay focused on the chat-protocol thesis.

## As built — notes that differ from the sketch

- The blob is bound to rooms via a **`blob_rooms`** link table (not a single `room` column), so the
  same content-addressed bytes can be handed off in several rooms; download is authorized by
  membership of *any* bound room (`Store::blob_readable_by`).
- `PutBlob` carries a **`Target`** (not a bare room name); the hub resolves it with the same
  `resolve_target` used by `Send`, so a DM/service handoff creates/joins the room exactly as a message
  would, and the follow-up `Send` lands in the same room.
- The content-address helper lives in **`parler_auth::content_id`** (both hub and connector already
  depend on `parler-auth`), keeping the hashing definition in one place.
- `recv` prints the **full** blob id in the `parler apply <id>` hint so it copy-pastes and works.
- `apply` runs `git bundle verify` → `git fetch` → pins the tip under `refs/parler/<blob-prefix>`;
  it never touches the working tree (merge stays a separate, explicit `git merge`).
- The MCP server exposes the same isolated import as `parler_apply`; callers provide the repository
  path, and merge or checkout remains outside the tool.

## Verified

- Unit/e2e: protocol `bundle_ref_round_trips_through_a_part` + `blob_frames_round_trip`; hub
  `blob_meta_and_room_binding`; connector e2e `code_handoff_push_recv_fetch_round_trips`
  (push → recv sees the bundle part → fetch matches bytes → non-member denied).
- Live: two `parler` agents over a real hub — `push` a real git bundle, peer `recv`s the 📦 handoff,
  `apply` lands the **exact tip** in a fresh repo, both commits present; a non-member `fetch` is
  denied; blobs persisted to `<db>.blobs/` content-addressed.
</content>
