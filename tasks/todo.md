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
- [x] Verify: shellcheck the new script, dry-run the scan, `CI_SKIP_WEB=1 make ci` (comments only,
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

## 2026-07-08 — Landing page simplification (mosaic.inc-style)

**Ask (Tam):** the landing page is too busy; a first-time visitor can't grab the idea. Simplify and
reorganize like https://mosaic.inc — minimal hero, one clear message; move critical-but-secondary
elements to standalone pages or the footer. Use the provided demo video if possible.

### Plan
- [x] Assets: copy attached demo video → `web/public/demo.mp4` (H.264, 3.4 MB, 42 s, captions
      burned in); extract a poster frame → `web/public/demo-poster.jpg`.
- [x] `hero.tsx`: rewrite — short serif H1 ("Hand off the conversation, not the clipboard."),
      one-sentence sub with the "chat protocol for AI agents" keyword, Get started + GitHub CTAs,
      and the demo video as the centerpiece (autoplay/muted/loop). Drop badge pill, install block,
      ParticleField.
- [x] `page.tsx`: cut WhoItsFor / SessionsFeature / Directory / HowItWorks / Examples / Security /
      Hardening / Faq. New tight sections: 3-step "how it works" (mirrors video captions), install
      one-liner terminal, one-line security strip → link to /docs/security. Keep metadata (money
      keywords) unchanged.
- [x] New `/faq` page reusing the Faq component (FAQPage JSON-LD moves with it); add to sitemap.
- [x] NavBar: Hub / Docs / Blog (+GitHub); CTA → Get started (/docs/quickstart). Kill dead `/#how`,
      `/#security` anchors.
- [x] Footer: expand to columns — Product (Hub, Session viewer, Docs, Quickstart), Resources (Blog,
      FAQ, Security model, RSS), GitHub/License/Issues.
- [x] Delete now-dead components: directory.tsx, examples.tsx, particle-field.tsx (agent-card/detail
      stay — used by agents-console). SessionsFeature stays (hub Sessions tab uses it).
- [x] Docs drift: update web/README.md description; grep for old anchors.
- [x] Verify: scripts/ci/web.sh (npm ci + next build), then headless-Chrome screenshots of / and
      /faq (desktop + mobile) and iterate.

### Review (done 2026-07-08)
- Landing shrank from 9 sections (~8k px) to 4 (hero+video · 3 steps · install · security line);
  everything relocated, nothing orphaned: FAQ → `/faq` (FAQPage JSON-LD moved with the component,
  h2→h1), directory/viewer stayed at `/hub`, security depth → `/docs/security` via strip + footer.
- Demo video: `web/public/demo.mp4` (H.264 `avc1`, 42 s, 3.4 MB, captions burned in) + poster
  extracted at t=2 s via a throwaway Swift/AVFoundation script (no ffmpeg on the machine). Video is
  autoplay/muted/loop/playsInline; poster = first scene so the pre-play frame doesn't jump. Easy to
  swap when Tam replaces the temporary cut — one `<video>` in `hero.tsx`.
- Security copy on the strip was tightened after the first screenshot: the *id* is the public key,
  the *private seed* is what never leaves the device (don't conflate them on the money page).
- Deleted dead components after the rewrite: `directory.tsx`, `examples.tsx`, `particle-field.tsx`
  (agent-card/detail stay — agents-console uses them; SessionsFeature stays — hub Sessions tab).
- Verified: `next build` green (81 pages, `/faq` present); headless-Chrome screenshots at 1440/520
  (the 390 px "overflow" was Chrome's min-window clamp — viewport meta present, wraps fine);
  stale-anchor grep clean (`/#how`, `/#security`, `/#faq` all gone).
- Headline rewrite (same session): H1 → "Your agents just became a team." (sells the collaboration
  outcome; transformation-headline pattern); sub-line now carries the mechanism ("one key pulls any
  agent — yours or a teammate's — into the same live conversation"). Rebuilt + screenshot-verified.
