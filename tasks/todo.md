# Task: Per-IP HTTP rate limiting on the hub's public front door

## Why
Rate limiting today only covers *authenticated* WS ops (per-agent `RateLimits`: sends/blobs).
The unauthenticated HTTP surface — `/api/directory`, `/api/session`, `/api/agents/:id`,
`/a2a/*`, and the `/ws` upgrade (connection/registration floods) — has no limit. That is the
real abuse/DoS/cost vector on the public hub. Add a per-client-IP fixed-window HTTP limiter.

## Plan
- [x] Confirm no existing HTTP rate limiting (only per-agent WS limiter exists)
- [ ] `server.rs`: add `DEFAULT_MAX_HTTP_PER_MIN` (600) + `max_http_per_min` field + `http_rate` map
- [ ] `server.rs`: `http_rate_allows(ip, now)` mirroring `rate_allows`
- [ ] `server.rs`: `client_ip(headers, peer)` — Fly-Client-IP -> X-Forwarded-For -> socket peer
- [ ] `server.rs`: `rate_limit` middleware (429 + Retry-After; exempt `/health`); wire in `app()`
- [ ] `server.rs`: `serve()` -> `into_make_service_with_connect_info::<SocketAddr>()`
- [ ] `server.rs`: extend `prune_rate_windows` to bound the `http_rate` map
- [ ] `main.rs`: `--max-http-per-min` / `PARLER_HUB_MAX_HTTP_PER_MIN` flag
- [ ] Tests: unit (`http_rate_allows`, `client_ip` precedence) + integration (429 + `/health` exempt)
- [ ] Docs: SECURITY.md note
- [ ] `CI_SKIP_WEB=1 make ci` green

## Review
Shipped a per-IP fixed-window HTTP limiter (`rate_limit` middleware) covering the whole public
front door — REST/A2A routes + the `/ws` upgrade — keyed `Fly-Client-IP` -> `X-Forwarded-For`
-> socket peer, `429 + Retry-After` over budget. Default 600/min, `PARLER_HUB_MAX_HTTP_PER_MIN`
/ `--max-http-per-min`, `0` disables. `/health` is exempt (Fly probe). Built in-house to match
the existing per-agent fixed-window limiter (no `governor` dep). `serve()` now uses
`into_make_service_with_connect_info` so direct connections key by socket peer. Janitor prunes
the per-IP map alongside the per-agent one.

Verified: 4 new unit tests (budget/rollover, disabled, prune, IP precedence) + 1 integration
test (flood -> 429, `/health` exempt). `cargo test -p parler-hub -p parler-connector -p
parler-cli` green, `clippy -D warnings` clean, `CI_SKIP_WEB=1 make ci` all gates passed.
Resolves the rate-limit half of audit finding A1 (doc updated). Still open (separate): CORS
tightening on `/api/session`, dedicated `RateKind::Redeem` (A2).
