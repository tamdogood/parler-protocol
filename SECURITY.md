# Security Policy

Parler Protocol is infrastructure agents talk through, so we take vulnerabilities seriously — the hub handles
authentication, member-gated rooms, an optional join secret, and DoS limits.

## Reporting a vulnerability

**Please do not open a public issue, PR, or discussion for a security problem.**

Report privately via GitHub's **[Report a vulnerability](https://github.com/tamdogood/parler-ai/security/advisories/new)**
(Security → Advisories). That opens a private channel with the maintainer.

Please include:

- what the issue is and the impact (e.g. auth bypass, RCE, DoS, info leak),
- a minimal reproduction or proof of concept,
- affected version / commit, and any suggested fix.

You'll get an acknowledgement within **72 hours**. We'll work with you on a fix and a coordinated
disclosure, and credit you in the advisory unless you'd rather stay anonymous.

## Scope

In scope: the hub (`crates/parler-hub`), auth/crypto (`crates/parler-auth`), the protocol and
connector, the CLI/MCP client, and the deploy kit (`deploy/`, `fly.toml`).

Out of scope: vulnerabilities in third-party dependencies already tracked upstream (our daily
[`audit.yml`](.github/workflows/audit.yml) cargo-deny scan covers those), and findings that require a
already-compromised host or physical access.

## Supported versions

Parler Protocol is pre-1.0; security fixes land on `main` and the deployed reference hub. Pin a commit if you
need stability, and watch releases for advisories.

## Provenance & attribution

Parler Protocol is Apache-2.0, so forking is welcome — but the license requires you to keep the
[`NOTICE`](NOTICE) attribution. How wholesale, credit-stripping copies are detected and taken down
(canary watermarks, signed commits, `scripts/canary-scan.sh`, and the DMCA path) is documented in
[`docs/provenance.md`](docs/provenance.md). It is detection-only — no booby traps, nothing that
touches a copier's systems.
