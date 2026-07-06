# File transfer between agents (`com.parler.file`)

## Goal
Let agents transfer arbitrary files efficiently, cheaply, fast — riding the existing
content-addressed blob transport (the same one code-handoff uses), not a new channel.

## Why this design (battle-tested foundations)
- **Content-addressed storage + SHA-256** (Git, Docker, IPFS, restic, borg): dedup + integrity.
  The hub already keys blobs by `sha256(bytes)` → same file to N agents / re-sends store once.
- **Raw WebSocket binary frames** (no base64 → −33% size, no encode/decode CPU): already the
  transport for blobs.
- Reuses the **audited** blob path: member-gated, size-capped (25 MiB), rate-limited, disk-budgeted,
  idle-GC'd. **Zero hub changes, zero new attack surface.**

## Plan (purely additive)
- [ ] `parler-protocol`: `FILE_KIND = "com.parler.file"` + `FileRef { blob, name, size, media_type?, summary? }`
      with `to_part`/`from_part` (mirror `BundleRef`). Round-trip unit test.
- [ ] `parler-connector`: extract private `put_blob()` (dedup upload logic from `push`); add
      `MeshAgent::send_file(target, name, bytes, media_type, note)`. Download reuses `fetch_blob`.
- [ ] `parler-cli`: `parler send-file <selectors> <path> [--note]`; `render_parts` renders 📎;
      `parler fetch` already downloads bytes (generalize help). `guess_media_type` helper.
- [ ] `parler-cli/mcp`: `parler_send_file` tool (+ spec); reuse `parler_fetch` for download; recv
      renders the file part via shared `render_parts`.
- [ ] Tests: protocol round-trip, connector e2e (send_file → recv sees 📎 → fetch matches → non-member
      denied), mcp send_file unit test. Keep `tool_specs_stay_lean` under budget.
- [ ] Docs: `docs/file-transfer.md` + AGENTS.md index line.
- [ ] `CI_SKIP_WEB=1 make ci` green.

## Deferred frontier (documented, not built)
Content-defined chunking (FastCDC, à la restic/borg) for sub-file dedup + resumable multi-frame
streaming for files > 25 MiB + optional zstd. Whole-file content-addressing already dedups whole files.

## Review — DONE ✅
All boxes above complete. Purely additive; **zero hub changes**.

**Verified**
- Protocol unit: `file_ref_round_trips_through_a_part` (round-trips + rejects plain/sibling parts).
- Connector e2e: `file_transfer_send_recv_fetch_round_trips` (send_file → recv sees 📎 with a bare
  basename → fetch matches bytes exactly → non-member denied). Existing `code_handoff_*` test still
  passes, proving the `push`→`put_blob` refactor is behavior-preserving.
- MCP e2e: `test_mcp_send_file_recv_fetch_e2e`.
- **Live binary**: two `parler` agents on a real hub — send-file a 20 KB random `.png`, peer recv
  shows the 📎 line, fetch writes byte-identical bytes (sha256 matches), blob id == sha256(file).
- Gate: `CI_SKIP_WEB=1 make ci` → all gates passed (clippy -D warnings, test --locked, doc, deny).

**Notes**
- `TOOL_SPECS_BUDGET` 12,000 → 12,400 (the new `parler_send_file` tool adds ~170 B; descriptions
  still under their own 4,700 B budget).
- CLI `parler fetch` default output `.bundle` → `.bin` (it now downloads a bundle *or* a file).
- Docs: `docs/file-transfer.md`, AGENTS.md index, communication.md capability map (row 7·b).
