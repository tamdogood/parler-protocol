# Visible host adapter contract

`parler conversation` keeps a provider's normal interactive UI open while Parler turns signed room
messages into native turns. Codex, Claude Code, and OpenCode implement the same product contract over
different host interfaces. This document is the extension checklist for a fourth provider.

## Boundary

The entrypoint in `crates/parler-cli/src/conversation.rs` performs host-independent setup once, then
passes one `AdapterContext` to the selected provider module:

```text
CLI options + terminal identity + workspace + exact hub + MeshAgent
                              |
                              v
                       AdapterContext
                 /            |             \
              Codex       Claude Code      OpenCode
```

Shared code owns:

- terminal-scoped identity and the managed child/MCP environment;
- create/join, portable hub routing, signed backlog validation, bounded catch-up paging, and files;
- arrival and presence conventions;
- actionable-message filtering, prompt construction, and loop prevention;
- signed terminal task receipts, replies, and explicit addressed handoffs.

Provider modules own:

- native session creation, resume, and visible UI attachment;
- the host's wake or turn-injection mechanism;
- native busy/idle and completion observation;
- bounded extraction of visible local transcript and final output;
- preserving the host's normal permission channel.

The modules expose one uniform `run(AdapterContext)` boundary rather than a fine-grained async trait.
Their internal state machines are materially different: Codex multiplexes app-server RPC and
notifications, Claude Code coordinates process hooks, and OpenCode consumes an SSE stream plus
canonical HTTP state. Forcing those operations into identical methods would move provider branching
into shared code and weaken the lifecycle invariants. The single dispatch match is the intended
registration point.

## Required parity

Every visible adapter must satisfy all of these behaviors:

| Contract | Requirement |
|---|---|
| Visible host | Attach the provider's normal interactive UI. Do not substitute a headless runner. |
| Identity | Apply `managed_host_environment` to the UI, native bridge, hooks, and invocation-local MCP process. |
| Resume | Validate provider session IDs and read only bounded visible transcript state. |
| Catch-up | Use `prepare_backlog`; never duplicate signature, file, or history handling in the adapter. |
| Durable ack | Call `commit_reads_through` only after the native host accepts the prepared context or completed peer turn. |
| Inbound work | Execute only `is_actionable` signed messages and serialize native turns. |
| Permissions | Leave approval and elicitation with the visible host. Never synthesize approval. |
| Local turns | Mirror the visible user's prompt and final agent answer, excluding internal bridge instructions. |
| Results | Publish through `publish_turn` or `send_peer_result` so every provider emits the same signed `TaskRef`. |
| Continuation | Continue automatically only for a valid addressed `PARLER_HANDOFF`; ordinary results do not wake peers again. |
| Presence | Publish `working` and `waiting`, including the shared heartbeat. |
| Failure | Do not acknowledge durable peer work when injection, execution, output retrieval, or publication fails. |

## Scaling invariants

All history and deduplication state must have an explicit bound:

| Path | Current bound and synchronization source |
|---|---|
| Shared room catch-up | 1,000-message pages, at most 10,000 messages per join, and a rolling 24,000-character trusted context tail. Prompt construction keeps the newest context inside each native host's envelope. An oversized history fails explicitly instead of replaying old messages as fresh work. |
| Codex | Status-driven synchronization; newest 64 full turns per canonical read; 256 recent terminal IDs retained. Idle threads do not issue history reads. |
| Claude Code | 9,000-character rewake prompt, 4 MiB transcript tail, 32 local prompts, one waiter per session, 24-hour hook lifetime, and ended state removed after the waiter releases it. |
| OpenCode | Native `/event` SSE status, newest 256 messages on terminal reconciliation, 1,024 assistant IDs, and an 8 MiB event/API buffer ceiling. No timer-driven transcript reads. |
| Parler delivery | One-message receive channel plus the durable room cursor provides backpressure. A provider failure leaves the cursor retryable. |

An event is a source-of-change signal, not durable truth. On a terminal event, read the provider's
bounded canonical state before publishing. Subscribe before the startup snapshot where the provider
allows it, so completion cannot fall between observation and subscription.

Do not add an unbounded `HashSet`, transcript vector, directory sort, response body, event buffer, or
fixed-interval full-history poll. If a provider offers pagination or events, use them. If it offers
neither, define an explicit bounded reconciliation strategy and document its worst-case cost.

## Adding a provider

1. Add one `Host` value and one dispatch arm in `conversation.rs`.
2. Add `conversation/<provider>.rs` with `pub(super) async fn run(AdapterContext) -> Result<()>`.
3. Probe the native executable and required interface at startup. Fail with an actionable update
   message when the installed version lacks the interface.
4. Validate every session identifier before placing it in a path, URL, or command argument. Preserve
   native local-server authentication and bound all response bodies and stream frames.
5. Configure both the visible process and its Parler MCP entry from `managed_host_environment`.
6. Call `enter_conversation`, `prepare_backlog`, `announce_arrival`, and `print_connected` instead of
   reimplementing the shared lifecycle.
7. Mark catch-up accepted at the provider's real durability point, then commit its exact cursor.
8. Recheck canonical busy state immediately before injecting queued peer work. A rejected concurrent
   start must remain queued, not be acknowledged or dropped.
9. Convert final native state into `TurnCapture` and use the shared result path.
10. Add tests for CLI selection, configuration overlay, resume-ID validation, bounded state, event or
    page parsing, local/injected turn separation, failed-turn retry, and the shared parity receipt.

A provider without a supported way to wake or inject into an existing visible session is not a
visible adapter. Keep `parler work` or `parler supervise` as the explicit managed fallback rather
than presenting a headless subprocess as parity.

## Verification

Run focused adapter tests first, then the repository gates:

```bash
CARGO_INCREMENTAL=0 cargo test -p parler-cli conversation --lib
CARGO_INCREMENTAL=0 cargo clippy --workspace -- -D warnings
CARGO_INCREMENTAL=0 make ci
```

Review the union against `docs/code-review-guidelines.md`, including failure paths where a provider
dies after receiving a peer message but before publishing and acknowledging its result.
