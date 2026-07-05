# Contributing to Parler Protocol

Thanks for helping build Parler Protocol! This guide gets you from a fresh clone to a green pull request. The
golden rule: **run `make ci` before you push** — it runs the *exact* gates the cloud runs, so if it's
green locally it'll be green on GitHub.

## Setup

You need **Rust** (stable) and **Node 20+**.

- Rust: nothing to pin by hand — [`rust-toolchain.toml`](rust-toolchain.toml) makes `rustup` install
  the right toolchain (stable + clippy) the first time you run `cargo` in the repo.
- Node: any 20+ works; the website lives in [`web/`](web/).

```bash
make build        # compile the Rust workspace + install web deps
make ci           # run the whole pipeline locally (see below)
```

## The pipeline is the contract

CI is not a pile of YAML you can't reproduce. Every gate is a small script in
[`scripts/ci/`](scripts/ci/), and the GitHub workflows just call them. So:

| Run locally            | What it checks                                                            |
| ---------------------- | ------------------------------------------------------------------------- |
| `make ci`              | **Everything below** — this is what the cloud sees                        |
| `make selftest`        | The pipeline scripts themselves (syntax, the `lib.sh` runner, config)     |
| `scripts/ci/rust.sh`   | `cargo build` · `clippy -D warnings` · `cargo test` · `cargo doc`         |
| `scripts/ci/web.sh`    | `npm ci` + `next build` (type-checks every route)                         |
| `make audit`           | `cargo-deny` — vulnerabilities, licenses, dependency sources              |
| `make smoke`           | Boots the real hub binary and probes its HTTP surface                     |

`make ci` runs **every** gate even if an earlier one fails, then reports the full list — so you fix
everything in one pass. To skip the (slow, network-heavy) website build while iterating on Rust:
`CI_SKIP_WEB=1 make ci`.

## House rules

- **Keep `cargo test --workspace` green.** New behavior comes with a test. Tests live in each crate's
  `src/*.rs` (`#[cfg(test)]` units) and `crates/*/tests/` (integration / e2e). The HTTP contract is
  pinned by [`crates/parler-hub/tests/smoke.rs`](crates/parler-hub/tests/smoke.rs).
- **Clippy is a hard gate** (`-D warnings`). No `#[allow(...)]` to silence a real lint without a
  one-line reason in a comment.
- **Do _not_ run `cargo fmt`.** This repository is deliberately **hand-formatted**; a repo-wide
  `cargo fmt` would reformat every file and blow up your diff. Match the style of the code around you.
  There is intentionally no rustfmt gate in CI.
- **Conventional commits.** Follow the existing style — `feat:`, `fix:`, `docs:`, `refactor:`,
  `test:`, `ci:`, `deps:` — optionally scoped, e.g. `feat(hub): …`. PR titles should match.
- **Small, focused PRs.** One logical change. Update docs in the same PR when behavior changes.

## Submitting a PR

1. Branch off `main`.
2. Make your change **with tests**.
3. `make ci` → green.
4. Open the PR. Fill in the template (what, why, how you tested). CI must pass and a
   [code owner](.github/CODEOWNERS) must approve before merge.

CI runs the same gates on your PR. The single required check is **"CI passed"** — it's green only
when build, clippy, test, doc, web, supply-chain, and the smoke test all pass.

## Adding to the pipeline itself

Because the pipeline is just scripts, you can extend it like any other code:

- Add or change a gate in `scripts/ci/`, wire it into `scripts/ci/all.sh` and the workflow.
- Run `make selftest` — it syntax-checks every script, unit-tests the `lib.sh` step runner on its
  success **and** failure paths, and sanity-checks the workflows and `deny.toml`. New scripts are
  picked up by adding them to the `scripts` list in `selftest.sh`.

## Reporting bugs & security issues

- Bugs / features: open an issue (there are templates).
- **Security vulnerabilities: do _not_ open a public issue** — see [`SECURITY.md`](SECURITY.md).

By contributing you agree your work is licensed under the project's [Apache-2.0](LICENSE) license.
