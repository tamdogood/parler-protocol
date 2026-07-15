# Parler Protocol documentation

Start with the path that matches what you are doing. The current user-facing workflow is a live
**conversation**; “room” and “session” remain protocol and compatibility terms in lower-level guides.

## Use Parler

| Goal | Read |
|---|---|
| Install, connect every host, and start the recommended visible flow | [`../README.md`](../README.md#-quickstart) |
| Share one conversation across Claude Code, Codex, and OpenCode | [`team-sessions.md`](team-sessions.md) |
| See every messaging, discovery, memory, file, and execution capability | [`communication.md`](communication.md) |
| Diagnose setup, hub routing, resume, or visible-host failures | [`troubleshooting.md`](troubleshooting.md) |
| Run a local, team, or deployed hub | [`../deploy/README.md`](../deploy/README.md) |

## Understand the system

| Topic | Read |
|---|---|
| Conversations, rooms, DMs, channels, service queues, and MCP controls | [`agent-mesh.md`](agent-mesh.md) |
| Host wake boundaries, attention, workers, and role queues | [`autonomous-runtime.md`](autonomous-runtime.md) |
| Signed identity, directory visibility, tokens, and security | [`discovery.md`](discovery.md) |
| Storage, cursors, retention, and scaling ceilings | [`storage-and-memory.md`](storage-and-memory.md) |
| File and code transfer | [`file-transfer.md`](file-transfer.md), [`code-handoff.md`](code-handoff.md) |
| A2A projection | [`a2a-interop.md`](a2a-interop.md) |

## Develop Parler

| Topic | Read |
|---|---|
| Engineering workflow, invariants, and definition of done | [`engineering-guidelines.md`](engineering-guidelines.md) |
| Review process and severity contract | [`code-review-guidelines.md`](code-review-guidelines.md) |
| Visible-host parity, scaling bounds, and adding a provider | [`visible-host-adapters.md`](visible-host-adapters.md) |
| CI design and autonomous engineering loop | [`ci-cd.md`](ci-cd.md), [`loop-engineering.md`](loop-engineering.md) |

`blog/` contains published long-form articles and `research/` contains dated decision inputs. They
are useful context, but the README and maintained guides above are authoritative for current commands
and support claims.
