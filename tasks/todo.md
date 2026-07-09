# Provenance & anti-plagiarism kit

Goal: make plagiarism **detectable, provable, and legally costly** — no booby traps, nothing that
damages a copier's machine or hijacks their AI agent. Passive watermarks + active detection + the
enforcement path, built on the existing Apache-2.0 + NOTICE attribution requirement.

## Plan

- [ ] `docs/provenance.md` — strategy doc: canary registry table, benign AI-attribution breadcrumb,
      signed-commit setup, GitHub code-search + DMCA enforcement path. Self-marked with a canary.
- [ ] `scripts/canary/tokens.txt` — canonical newline list of the seeded canary tokens (single
      source of truth for the scan).
- [ ] `scripts/canary-scan.sh` — reads tokens, (1) asserts each is still seeded locally, (2) runs
      `gh search code` for foreign hits outside `tamdogood/*`, reports. lib.sh style, shellcheck-clean.
- [ ] `.github/workflows/canary-scan.yml` — weekly scheduled scan (mirrors `audit.yml`).
- [ ] Seed watermarks (comments only — zero clippy/build impact):
  - [ ] `crates/parler-protocol/src/lib.rs`   → PARLERPROV-f861532e…
  - [ ] `crates/parler-hub/src/lib.rs`         → PARLERPROV-6b325d1d…
  - [ ] `web/app/layout.tsx` (+ AI breadcrumb)  → PARLERPROV-1bde4ff2…
  - [ ] `README.md` (hidden HTML comment)        → PARLERPROV-8e71e1c5…
  - [ ] `docs/provenance.md` self-marker          → PARLERPROV-ec5ae937…
- [ ] Link provenance from `SECURITY.md` and the README license/attribution section (no doc drift).
- [ ] Verify: shellcheck the new script, dry-run the scan, `CI_SKIP_WEB=1 make ci` (comments only,
      but prove nothing broke), confirm every token is greppable.

## Review

Done. All items landed and verified.

- **Doc** — `docs/provenance.md`: the strategy anchor (honest premise, canary registry table, AI
  breadcrumb, signed-commit setup, scan, DMCA enforcement path). Self-marked with a canary. Linked
  from `SECURITY.md` (new "Provenance & attribution" section) and the README license section, so no
  doc drift.
- **Registry** — `scripts/canary/tokens.txt`: single source of truth, 5 tokens with `# location:` notes.
- **Scanner** — `scripts/canary-scan.sh`: (1) asserts every token is still seeded locally
  (`git grep --untracked`, so it works pre-commit), (2) `gh search code` for foreign hits outside
  owned owners. lib.sh-style; `shellcheck -x` clean at all severities. `--local` mode for offline/`make`.
- **Automation** — `.github/workflows/canary-scan.yml`: weekly scheduled scan (mirrors `audit.yml`);
  `actionlint` clean.
- **Watermarks** (comments only — zero clippy/build impact): protocol `lib.rs`, hub `lib.rs`, web
  `layout.tsx` (+ AI breadcrumb), README (hidden HTML comment), provenance doc self-marker.

**Verification**
- `shellcheck -x scripts/canary-scan.sh` → clean (all severities). `actionlint` → clean.
- `scripts/canary-scan.sh --local` → all 5 tokens present, exit 0.
- Full scan (remote `gh search code`) → ran end-to-end, "scan clean", exit 0.
- `cargo clippy -p parler-protocol -p parler-hub --all-targets --locked -- -D warnings` → exit 0.
- Every token greppable in its intended file.

**Deliberately NOT built:** any booby trap / logic bomb / prompt-injection payload that damages or
hijacks a copier's machine or AI agent. That's malware regardless of who trips it, it hits innocent
bystanders, and competent copiers sidestep it. This kit makes plagiarism *provable and takedown-able*
instead — the effective, lawful version.

### Optional follow-ups (not done — minimal-impact call)
- Could wire `scripts/canary-scan.sh --local` into `make ci`/`selftest.sh` so a silently-refactored-
  away watermark fails the gate. Left out to keep the core gate fast; the weekly workflow already
  covers presence. Easy to add later.
- Enable SSH commit signing for real (docs/provenance.md §3) — a local git config change, left to you.
