# Backlog — the autonomous loop's work queue

This is the **forward queue**: a prioritized list of small, independently-shippable items the loop
pulls from, one per iteration (top unchecked item first). It is the single source of truth for "what
to work on next".

- `tasks/todo.md` is the **log** of finished work — append a summary there when you complete an item.
- `tasks/lessons.md` is the **memory** — append a rule there after any correction or surprise.

The **`web/` app is out of scope** for this loop — Tam drives it by hand. Loop items must be
Rust/CLI/protocol only; anything that also needs a UI/site change carries a `[HUMAN] web: …` note for
the part the loop leaves untouched. The loop gates with `scripts/verify.sh --rust-only`.

Each item must be small enough to land behind `scripts/verify.sh --rust-only` in one iteration, additive,
and backward-compatible with the deployed protocol/hub. If an item is too big, the loop should split it
and check in the sub-items here rather than attempt it whole. Keep `[P0]`/`[P1]`/`[P2]` priority tags.

> **Editing rules:** add new items at the right priority; never delete history (check items off with
> `[x]` and let `todo.md` carry the write-up). Anything referencing the pre-pivot NATS architecture
> (`parler-manager`, `parler-delivery`, `parler-console`, KV planes…) is **dead** — those crates were
> removed in cc686ea. Do not resurrect them.

---

## Now (pull from the top)

### Epic: Verifiable mesh — the hub can relay but can't lie (security + resilience)
*Audit (2026-06-29, `tasks/todo.md`): the "compromised hub can't impersonate anyone" guarantee covers
signed cards but NOT messages — a malicious hub can forge/alter/reorder the conversation a joining
agent is "caught up" on. Borrows distributed-ledger / Certificate-Transparency / reliable-messaging
ideas. Each item additive + backward-compatible.*

- [x] **[P0] Authenticated messages (signatures)** — DONE 2026-06-29 (see `tasks/todo.md` review).
  Author signs each message; carried as a `com.parler.sig` extension part (mirrors `com.parler.bundle`)
  so it needs **no hub/protocol/schema change** and works against the deployed hub. Signed payload =
  parts(non-sig) + target + author id + replyTo + client ts/uid (excludes `mentions` — hub normalizes
  them). `canonical_message_bytes` + `MessageSig` codec in `parler-protocol`; `MeshAgent::send`
  auto-signs; `verify_message(...) -> SigStatus`; CLI/MCP show ⚠/✗ (valid is clean) + hide the sig
  part; hub `/api/session` drops it; +13 tests (2 codec, 6 connector unit, 5 e2e). `VERIFY: PASS`.

- [ ] **[P1] Tamper-evident room log (hash chain + fork detection)** — sig payload commits to `prev`
  (hash of the author's last-seen message in that room); `parler verify --room R` walks the chain and
  prints a head; comparing two members' heads detects hub equivocation/split-view. Builds on the P0
  signature. *Done when:* chain fields in the sig payload, a CLI verifier, an e2e that detects a
  tampered/reordered backlog, doc in `docs/`. Additive.

- [ ] **[P1] Exactly-once sends (idempotency key)** — reuse the signed `uid` as an idempotency key; the
  hub dedups a re-sent message within a window so a retry after a dropped `Sent` ack never duplicates.
  *Done when:* hub dedup (store unique-ish on (room,uid) or a short LRU), connector retries safely, an
  e2e that double-sends one uid and asserts one stored row + same returned id. Additive.

- [ ] **[P2] Self-healing connection (auto-reconnect + cursor resume)** — a reconnecting transport
  re-handshakes on socket loss, resumes from the durable cursor, re-arms `subscribe`, exponential
  backoff. *Done when:* opt-in reconnect wrapper, an e2e that kills the socket mid-session and asserts
  the next `recv` transparently resumes. Additive (pure client-side).

- [ ] **[P2] Hardened auth challenge (domain-separated, hub-bound, expiring nonce)** — make the
  challenge nonce an opaque structured string (`parler-auth:v1:<hub>:<exp>:<rand>`) so the signature is
  domain-separated and replay-bounded; the client signs the opaque string it's handed ⇒ **zero client
  change**. *Done when:* hub builds + validates the structured nonce (expiry + hub-id checked), unit
  tests for expired/foreign-hub nonces, e2e auth still green. Additive.

- [x] **[P0] Seed `tasks/lessons.md` discipline** — DONE 2026-06-29. The verify gate (`scripts/verify.sh
  --rust-only`) was confirmed trustworthy: it correctly **failed** on a real error (the missing `uuid`
  lock edge) and **passed** once fixed. Five new lessons appended after this iteration's surprises.

- [ ] **[P1] Code-handoff frontier index** (`docs/code-handoff.md` Phase 3) — index the latest bundle
  per room (tip id / short summary / author / ts) in the hub store; expose `parler frontier --room R`
  on the CLI; surface "latest handoff" in `parler rooms` output. *Done when:* new store table/columns
  + migration, CLI subcommand, an e2e test that pushes two bundles and asserts `frontier` returns the
  second, and the README/`docs/code-handoff.md` Phase 3 box is checked. Additive only.

- [ ] **[P1] Streaming blob upload** (`docs/storage-and-memory.md` P3 / B1) — replace the single
  fully-buffered-in-RAM blob frame with chunked upload so large handoffs don't pin memory. Keep the
  25 MiB cap as a configurable ceiling; verify sha256 incrementally. *Done when:* protocol frames for
  chunked put, hub assembles to disk without buffering the whole blob in RAM, connector streams from a
  file, and an e2e test moves a >1 MiB bundle in chunks. Backward-compatible: old single-frame path
  still accepted.

## Next

- [ ] **[P2] sqlite-vec semantic memory** (`docs/storage-and-memory.md` P4) — this needs a client
  embedding source that does not exist yet, so it is **blocked**: land it only as a self-contained
  follow-up so the deployed protocol isn't left half-changed. Until unblocked, leave checked-off-able
  design notes only. *Prereq:* decide where embeddings come from (client-supplied vs hub-side model).

- [ ] **[P2] schemars schema export** — `parler-protocol`: generate `spec/parler.schema.json` from the
  frame types via `schemars`, plus a test that the checked-in schema matches the generated one (so the
  wire format can't drift silently). *Done when:* schema file + drift test in CI's `cargo test`.

## Icebox (needs a human decision before the loop touches it)

- [ ] Benchmarks vs the old Node implementation (criterion + e2e RTT/throughput) → `docs/benchmarks.md`.
- [ ] Anything that changes the deployed wire protocol in a non-additive way (needs explicit sign-off).
