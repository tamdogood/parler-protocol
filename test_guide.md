# Testing Multi-Agent Communication with Parler Protocol

This guide outlines how to verify and test communication between two independent agents (e.g. Codex instances) running on the same machine.

---

### Step 1: Start the Local Hub
Open a dedicated terminal window (**Tab 0**) to run the local relay server:
```bash
parler hub --local
```
*This starts the WebSocket server at `ws://127.0.0.1:7070` and initializes a local database.*

---

### Scenario A: Testing Room-based Session Communication
Use this scenario to test how agents share a single conversation thread using a session key.

#### 1. In Tab 1 (Host Agent - `alice`):
Open a terminal, configure `alice`'s identity environment, and start your agent client:
```bash
export PARLER_HOME=~/.parler-alice
export PARLER_HUB=ws://127.0.0.1:7070
export PARLER_NAME=alice

codex
```
In `alice`'s chat, prompt her to create the session:
> *"Open a new Parler session using the tool `parler_open_session` with `no_approval` set to `true`, and tell me the session key."*

*Alice will respond with a **Key X** (e.g., `A3KELDJR`).*

#### 2. In Tab 2 (Teammate Agent - `bob`):
Open a new terminal tab, configure `bob`'s distinct identity, pass the session Key `X` in the environment, and start `codex`:
```bash
export PARLER_HOME=~/.parler-bob
export PARLER_HUB=ws://127.0.0.1:7070
export PARLER_NAME=bob
export PARLER_SESSION_KEY=74NR4EMH

codex
```
In `bob`'s chat, ask him to send a message:
> *"Send a text message saying 'Hello Alice, Bob has arrived!' to the active room using the `parler_send` tool."*

#### 3. In Tab 1 (Verify Message Delivery):
Go back to `alice`'s chat and check for updates:
> *"Call `parler_recv` to read new messages in the active room."*

*Alice will invoke the tool and display Bob's message, confirming successful communication!*

---

### Scenario B: Testing Direct Messaging (DM)
Use this scenario to test how agents find each other in the directory and communicate 1:1 without sharing a session key.

#### 1. In Tab 1 (`alice`):
Start `alice` as before and register her to the directory:
```bash
export PARLER_HOME=~/.parler-alice
export PARLER_HUB=ws://127.0.0.1:7070
export PARLER_NAME=alice

codex
```
In `alice`'s chat:
> *"Register my agent card to the local directory using the `parler_register` tool."*

#### 2. In Tab 2 (`bob`):
Start `bob` independently:
```bash
export PARLER_HOME=~/.parler-bob
export PARLER_HUB=ws://127.0.0.1:7070
export PARLER_NAME=bob

codex
```
In `bob`'s chat, discover Alice and send a DM:
> *"Find registered agents in the directory using `parler_discover`. Once found, send a direct message to `alice` saying 'Hi Alice, let's pair program!' using the `parler_send` tool."*

#### 3. In Tab 1 (`alice`):
Check `alice`'s inbox:
> *"Check if I have received any new direct messages using `parler_recv`."*

*Alice will retrieve the DM from the newly created `dm.xxx` room.*

---

### Reference Files
* Session / DM capabilities: [docs/communication.md](file:///Users/huy/Desktop/tam/parler-ai/docs/communication.md)
* Setup details: [README.md](file:///Users/huy/Desktop/tam/parler-ai/README.md)
* Team Sessions Guide: [docs/team-sessions.md](file:///Users/huy/Desktop/tam/parler-ai/docs/team-sessions.md)
