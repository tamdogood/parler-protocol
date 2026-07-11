# Session viewer: monitor file exchanges + easy file access

## Interpretation
"monitor the **file** exchanges" (dictation rendered "file" as "five"). The ask: the session viewer —
on **both** the website and the desktop app — should let a watcher see the *file/bundle handoffs* in
the timeline (name, size, type) alongside the conversation, and **download/open** those files easily.

## The gap (verified in code)
- `viewer_message` (crates/parler-hub/src/server.rs:1064) reduces every `com.parler.bundle` /
  `com.parler.file` part to just `{ kind }` — no name/size/type/blob-id reaches the viewer.
- Blob bytes move **only over WS** (`GetBlob`, authorized by room *membership*). There is **no REST
  route** to fetch a blob, and a read-only watcher has no membership. So today: files are invisible
  in the viewer and unreachable.

## Security decision (needs the owner's nod)
The watch token currently grants read of **text only** — bytes were deliberately hidden. Satisfying
"give them access to the files" broadens it to **text + that room's exchanged files**. Kept tight:
room-scoped (`blob_rooms`), read-only, valid non-expired token, `Content-Disposition: attachment` +
`X-Content-Type-Options: nosniff` (no inline render → no XSS in hub origin), 25 MiB cap, GC'd.

## A. Backend — this repo (parler-hub / -protocol)
- [ ] `store.rs`: add `blob_in_room(id, room) -> Result<bool>` (query `blob_rooms`); +1 unit test.
- [ ] `server.rs` `viewer_message`: for bundle/file parts emit a whitelisted `file` object
      `{ blob, name?, size, mediaType?, summary?, vcs?, tip? }` via `BundleRef`/`FileRef::from_part`.
      No bytes. Update the doc comment.
- [ ] `server.rs`: new route `GET /api/session/blob/:id` (Bearer watch token or `?token=`):
      validate watch token → room → `blob_in_room` → stream bytes off the runtime (spawn_blocking),
      headers Content-Type/Length/Disposition(attachment; sanitized `?name`)/nosniff/no-store;
      `touch_blob_fetched`. Mirror `handle_get_blob`.
- [ ] Extend the `parler_protocol` import with `BundleRef, FileRef, BUNDLE_KIND, FILE_KIND`.
- [ ] `make ci` green.

## B. Desktop app — this repo (desktop/)
- [ ] `lib/types.ts`: `SessionPart.file?` optional metadata.
- [ ] `lib/api.ts`: `fetchSessionBlob(base, token, blob) -> Blob` (Bearer header).
- [ ] `session-viewer.tsx`: richer file card (type icon, name/derived, human size, summary) + a
      **Download** button; `SessionViewer` passes a `downloadFile(part)` callback (holds base+token)
      → fetch blob → objectURL → anchor `download`. No new IPC.

## C. Website — tamdogood/parler-web (separate PR)
- [ ] Mirror B: `lib/types.ts`, `lib/api.ts` (`fetchSessionBlob`), `SessionViewer` + `/session`.
      Same browser objectURL download. Branch + PR vs parler-web `main`.

## D. Docs (this repo)
- [ ] Update docs/agent-mesh.md + docs/discovery.md (watch/session-viewer surface) + grep README.md
      + AGENTS.md for the viewer/watch description; document `GET /api/session/blob/:id` and the
      widened watch scope.

## E. Verify
- [ ] `make ci` green. Live: open session → send-file + push bundle → mint watch code → `/api/session`
      shows `file` metadata; `/api/session/blob/:id` bytes match; wrong-room blob → 403; no token →
      401. Build desktop renderer. Web: `npm run build` + local run.

## Deploy note
Website download works once the backend lands on the public hub (parler-hub.fly.dev). Desktop uses its
bundled hub → works after rebuild.

## Review

**Shipped.** All four sections done and verified.

- **A — Backend (this repo):** `Store::blob_in_room` (+ test assertions in `blob_meta_and_room_binding`);
  `viewer_message` now emits whitelisted `file` metadata via `BundleRef`/`FileRef::to_value` (no bytes);
  new `GET /api/session/blob/:id` watch-gated download — room-scoped, `attachment` + `nosniff` +
  `no-store`, read off the blocking pool, `touch_blob_fetched` for GC LRU; `download_filename`
  sanitizer. No wire/protocol change. New e2e `web_session_viewer_downloads_an_exchanged_file` asserts
  metadata + byte-fidelity + nosniff/attachment headers + 403 (wrong-room) + 401 (join key / none).
  `make ci` green (selftest + rust clippy -D warnings/test/doc + audit).
- **B — Desktop app (this repo):** `SessionFile` type + `SessionPart.file`; `fetchSessionBlob`;
  `FilePart` card (icon/name/size/type + Download) wired through a `downloadFile` callback (objectURL);
  `downloadError` surfaced. `typecheck:web` + `electron-vite build` green.
- **C — Website (tamdogood/parler-web):** mirrored — `SessionFile`/`SessionPart.file`,
  `fetchSessionBlob`, `FilePart` in both the chat + timeline-replay render paths, `downloadFile` in
  `ConnectedView` (already had `token`), subtitle copy. `npm run build` green. → its own PR.
- **D — Docs:** team-sessions.md (viewer + both endpoints + 401/403), discovery.md (REST table rows),
  agent-mesh.md (watch section), code-handoff.md ("no web UI for code" → downloadable, not rendered).

**Security posture (as approved):** the watch token now also authorizes downloading *that room's*
exchanged files — room-scoped (`blob_rooms`), read-only, non-expired token only, no-sniff attachment
(no inline render → no XSS in hub origin), 25 MiB cap, GC'd. Metadata surfacing leaks only whitelisted
ref fields; `data` parts still hide their payload.

**Deploy dependency:** website downloads/metadata go live once the backend deploys to
parler-hub.fly.dev. Pre-deploy it degrades gracefully (older hub omits `file` ⇒ card shows name/type,
no Download button). Desktop uses its bundled hub ⇒ works after rebuild.
