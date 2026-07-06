# Parler Protocol File Transfer — hand a peer a file, not a paste

**Status: BUILT, 2026-07-05.** Try it: `parler send-file` / `parler fetch` (and the
`parler_send_file` / `parler_fetch` MCP tools).

Two agents can already hand each other a **code bundle** ([`code-handoff.md`](code-handoff.md)). This
adds the general case: transfer **any file** — a PDF, an image, a log, a `.zip` — over the same
transport, so an agent stops pasting a base64 blob into the chat and instead moves the bytes
directly.

```
   alice ──send-file (bytes)──► parler-hub ──(blob on disk)        a transfer is an ordinary room
                                    │                              message carrying a file Part,
   bob   ──recv──► sees a com.parler.file Part  ──fetch──► bytes   so send/recv/cursor/wake all
         ──(save to disk)                                          work unchanged.
```

## It reuses the proven blob transport — nothing new on the hub

File transfer is **not a new channel**. It rides the exact content-addressed blob path that code
handoff already uses, so the hub needs **zero changes** and gains **zero new attack surface**:

1. **The blob** — the file's bytes, stored **content-addressed** (`id = sha256(bytes)`) on the hub's
   disk and bound to the room it was posted to. Deduped; tamper-evident; the hub never executes it.
2. **The reference** — an ordinary room message carrying a `Part::Extension` of kind
   `com.parler.file`:

   ```json
   { "kind": "com.parler.file", "blob": "<sha256>", "name": "report.pdf",
     "size": 20000, "mediaType": "application/pdf", "summary": "Q3 numbers" }
   ```

The only difference from a `com.parler.bundle` reference is a `name` (the original basename, so a
receiver can save it back) and the absence of VCS/commit fields. Everything downstream —
`send`/`recv`, the per-room cursor, durability, reconnect-resume, and the Stop-hook "wake" — works
with zero changes. An old client that doesn't know `com.parler.file` still sees a renderable
extension part.

## Why this is efficient, cheap, and fast (the battle-tested parts)

The design borrows the two patterns that industrial file movers converge on:

| Property | How | Prior art |
|---|---|---|
| **Fast** | Bytes ride the already-authenticated WebSocket as a **raw binary frame** — no base64 (which adds ~33% size + encode/decode CPU). One round-trip to upload, one frame to download. | RFC 6455 binary frames |
| **Cheap** | **Content-addressing**: the id *is* `sha256(bytes)`, so the same file sent to N agents (or re-sent) is stored **once** on the hub. Files never touch the 1 MiB JSON message path — they ride the binary blob path. | Git, Docker layers, IPFS, restic, borg |
| **Integrity** | The hub rejects any upload whose bytes don't hash to the declared id. | Content-addressed stores |
| **Bounded & safe** | Inherits the blob layer's defenses: room-membership authorization, `max_blob_bytes` (25 MiB default), per-agent rate limits, a total disk budget, and idle GC. | — |

## Transport (unchanged from code handoff)

```
client → PutBlob { target, sha256, size, mediaType }
hub    → BlobReady { id }             # member + size OK; expect the bytes next
client → <Binary frame: the file>     # whole file in one frame, ≤ max_blob_bytes
hub    → BlobStored { id }            # verified sha256(bytes)==id && len==size; written to disk
```

Download is `GetBlob { id }` → `BlobIncoming` → one binary frame. `parler fetch` writes the bytes to
`-o <path>`; the receiver chooses where they land (no path is ever taken from the sender's `name`).

## Using it

```bash
# CLI
parler send-file --room dev ./report.pdf --note "Q3 numbers"   # or --to <id> / --service <name>
parler recv --room dev                                         # shows: 📎 report.pdf (…) — parler fetch <blob> -o report.pdf
parler fetch <blob> -o report.pdf                              # download the exact bytes
```

```jsonc
// MCP
parler_send_file { "room": "dev", "path": "./report.pdf", "note": "Q3 numbers" }
parler_fetch     { "id": "<blob>", "out": "report.pdf" }       // the same downloader bundles use
```

## What changed, file by file

- `parler-protocol`: `FILE_KIND = "com.parler.file"` + `FileRef { blob, name, size, mediaType?,
  summary? }` with `to_part`/`from_part` (the plain-file sibling of `BundleRef`).
- `parler-connector`: `MeshAgent::send_file(target, name, bytes, media_type, note)`; the shared
  `put_blob` helper is factored out of `push` so both post the same content-addressed upload and
  differ only in the reference part. Download reuses `fetch_blob` (bytes are just a blob).
- `parler-cli`: `parler send-file`; `recv` renders `com.parler.file` as a 📎 line; `parler fetch`
  already downloads any blob. `guess_media_type` labels a handful of common extensions.
- `parler-cli/mcp`: `parler_send_file` tool; download reuses `parler_fetch`.
- **`parler-hub`: no changes.**

## Deferred frontier (not built)

Whole-file content-addressing already dedups whole files. The next tier — worth it only when real
usage demands it — is **content-defined chunking** (FastCDC, à la restic/borg) for *sub-file* dedup,
**resumable multi-frame streaming** for files larger than a single-frame RAM buffer (> 25 MiB), and
optional **zstd** compression. The `com.parler.file` reference format doesn't change when they land,
because the blob stays content-addressed.

## Verified

- Unit/e2e: protocol `file_ref_round_trips_through_a_part`; connector e2e
  `file_transfer_send_recv_fetch_round_trips` (send_file → recv sees the 📎 part with the basename →
  fetch matches bytes exactly → non-member denied); MCP `test_mcp_send_file_recv_fetch_e2e`.
- Live: two `parler` agents over a real hub — `send-file` a 20 KB random binary, peer `recv`s the 📎
  transfer, `fetch` writes byte-identical bytes (`sha256` matches), and the blob id equals
  `sha256(file)` (content-addressed); a directory prefix in the name is stripped to a bare basename.
