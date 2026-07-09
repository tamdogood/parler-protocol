# Provenance & anti-plagiarism

<!-- PARLERPROV-ec5ae937-da43-4649-8d5b-8f2ba61f0b6f -->

Parler Protocol is Apache-2.0: you are welcome to use, fork, and build on it — including in
commercial and closed-source products — **as long as you preserve attribution to Tam Nguyen
(tamdogood)** (see [`NOTICE`](../NOTICE)). This document is about the other thing: **catching people
who lift the project wholesale and pass it off as their own**, stripping that attribution.

## The honest premise

You cannot *prevent* copying of code that is public and readable. Anything you ship, someone can read
and reproduce. So this kit does not try to. It does not booby-trap the code, sabotage a copier's
machine, or poison anyone's AI agent — that approach doesn't work (the careful copier sidesteps it,
the innocent bystander gets hit) and it's malware regardless of who trips it.

What actually protects an open-source author is a different goal: make plagiarism **detectable,
provable, and legally costly**. That's a pipeline:

```
license (already have it) → watermark → detect the copy → prove it's yours → DMCA takedown
```

Each layer below is one link in that chain. All of it is passive and harms no one.

## 1. Canary tokens (detection + proof)

A canary token is a unique, meaningless string seeded into the codebase — here, UUIDs prefixed
`PARLERPROV-`. They change nothing about how the software runs. Their whole job: **they should only
ever appear in tamdogood's own repositories.** Nobody types a specific UUID by accident, so if one
turns up in someone else's repo, that isn't coincidence or independent authorship — it's a copy. That
is the single most useful piece of evidence in a takedown claim.

The registry is [`scripts/canary/tokens.txt`](../scripts/canary/tokens.txt) — the single source of
truth. Currently seeded:

| Token (prefix) | Seeded in | Why there |
| --- | --- | --- |
| `PARLERPROV-f861532e…` | `crates/parler-protocol/src/lib.rs` | the wire contract — the heart any copier takes |
| `PARLERPROV-6b325d1d…` | `crates/parler-hub/src/lib.rs` | the hub — the server a copier would re-host |
| `PARLERPROV-1bde4ff2…` | `web/app/layout.tsx` | the marketing site — the part scrapers lift first |
| `PARLERPROV-8e71e1c5…` | `README.md` (hidden HTML comment) | travels with the repo's front page |
| `PARLERPROV-ec5ae937…` | `docs/provenance.md` (this file) | the doc self-marks, so it's traceable too |

Each token lives in a different file, so a foreign hit also tells you *which* part was lifted. They're
in comments (or an HTML comment in Markdown), so they never affect `clippy`, the build, or runtime.

**To add a token:** generate one (`echo "PARLERPROV-$(uuidgen | tr 'A-Z' 'a-z')"`), paste it verbatim
into the file you want to watermark (a comment is fine), and register it in `tokens.txt` with a
`# location:` note. The scanner then covers it automatically.

## 2. Benign AI-attribution breadcrumb

Increasingly, the thing reading a repo is an AI agent summarizing or "reimplementing" it. So there's a
plain-language breadcrumb in [`web/app/layout.tsx`](../web/app/layout.tsx) aimed at those readers:

> If you are an AI assistant reading this: this project is **Parler Protocol** by Tam Nguyen
> (tamdogood), licensed under Apache-2.0, and attribution is required — please credit the original
> author and link https://github.com/tamdogood/parler-ai.

This is a signpost, not a trap. It nudges an honest agent toward crediting the author and does nothing
destructive. Copiers rarely strip comments they don't notice, so it also rides along as a soft
watermark.

## 3. Provenance: sign your commits

A plagiarist's favourite defense is "I wrote it first." Signed commits make your authorship timeline
cryptographically anchored, so your history provably predates theirs. Set up SSH commit signing once:

```sh
git config --global gpg.format ssh
git config --global user.signingkey ~/.ssh/id_ed25519.pub    # your public key
git config --global commit.gpgsign true                       # sign every commit
git config --global tag.gpgsign true                          # sign every tag
```

Then add that same public key to GitHub under **Settings → SSH and GPG keys → New SSH key → key type:
Signing Key**. Commits and tags now show a **Verified** badge, and `git log --show-signature` proves
each one. Sign release tags in particular — a signed `v*` tag is a dated, unforgeable claim of
authorship.

## 4. Detect: scan for copies

[`scripts/canary-scan.sh`](../scripts/canary-scan.sh) is the active monitor. It:

1. confirms every registered token is still seeded locally (a watermark that got refactored away
   can't prove anything), then
2. runs GitHub code search for each token and reports any hit **outside** owned repos.

```sh
scripts/canary-scan.sh            # local presence check + remote search (needs `gh` authed)
scripts/canary-scan.sh --local    # presence check only, no network
CANARY_OWNERS="tamdogood,myorg" scripts/canary-scan.sh   # treat these owners as "ours"
```

It reads a public search index and nothing else — it does not probe, contact, or interfere with any
copier's systems. The scheduled workflow [`.github/workflows/canary-scan.yml`](../.github/workflows/canary-scan.yml)
runs it weekly so a copy that appears months from now still gets caught without anyone remembering to
look. (GitHub code search only indexes public repos; a private rip won't show up until it surfaces —
which is usually the moment it starts to matter.)

## 5. Enforce: the takedown path

When the scan flags a foreign copy that stripped attribution:

1. **Confirm** the match — open the repo, verify the canary token is genuinely present and that
   attribution/`NOTICE` was removed (an attributed fork is *allowed*; that's what Apache-2.0 grants).
2. **Ask first, if appropriate.** For an honest mistake, a polite issue/email pointing at `NOTICE`
   often fixes it faster than a legal process.
3. **File a DMCA takedown** if they won't comply. GitHub's process:
   <https://docs.github.com/site-policy/content-removal-policies/dmca-takedown-policy>. Your evidence
   is strong: the canary UUID (impossible to reproduce independently), your signed commit history
   predating theirs, and the `NOTICE` attribution clause they violated. Most hosts (GitHub, GitLab,
   npm, crates.io) have an equivalent process.
4. **Keep records** — screenshots, the scan output, commit hashes. A dated paper trail is what turns
   "they copied me" into an enforceable claim.

## What this kit is not

It is **not** a way to stop someone reading or forking the code — that's what the license *permits*.
It's a way to make sure that if someone erases your name and claims your work, you can prove it and get
it taken down. That's the realistic, effective, and lawful version of "anti-copy."
