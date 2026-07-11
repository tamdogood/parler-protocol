# Using Parler Protocol with Codex

This guide explains how to configure Codex to use Parler Protocol for multi-agent collaboration, timeline capture, and memory retrieval.

---

## 1. Build and Install the Latest Parler Protocol Version

Before configuring your environment, compile and install the latest Parler Protocol binary from the root of the repository:

```bash
cargo install --path crates/parler-bin --force
```

Ensure that your cargo bin directory (usually `~/.cargo/bin`) is added to your system's `PATH` so the `parler` command runs anywhere.

---

## 2. Start the Hub Relay

If you are running Parler Protocol locally, boot the Hub WebSocket relay:

* **Start the Hub Relay**:
  ```bash
  parler hub --addr 127.0.0.1:7070
  ```

---

## 3. Configure the MCP Server

Add the Parler Protocol MCP server to your global Codex configuration file (`~/.codex/config.toml`):

```toml
[mcp_servers.parler]
command = "parler"
args = ["mcp"]
env = { PARLER_HOME = "~/.parler-codex", PARLER_HUB = "parler://127.0.0.1:7070", PARLER_NAME = "codex" }
```

---

## 4. Configure Global Timeline Capture Hooks

To automatically stream your prompts and tool executions (e.g. file edits, commands) to your active Parler Protocol room, create or update your global hooks file at `~/.codex/hooks.json`:

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "parler hook session-start"
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "parler hook user-prompt-submit"
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "parler hook post-tool-use"
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "parler hook session-end"
          }
        ]
      }
    ]
  }
}
```

---

## 5. Working in a Collaborative Session

### How to Open/Create a Session Room:
Before other agents can join, the host must initialize a collaborative session room:

1. **Start the Hub Relay** (if hosting locally):
   ```bash
   parler hub --addr 127.0.0.1:7070
   ```
2. **Open the Session Room**:
   Run the CLI open command from the host terminal:
   ```bash
   parler session open --topic "debugging-session" --no-approval
   ```
   This prints out the collaborative **Room ID** and a multi-use **Join Key** (`KEY`).

### How to Join a Session:
You can onboard Codex into an active collaborative session in two ways:

* **Prompt Codex in Chat**:
  Simply tell Codex:
  > "Join the Parler Protocol session using key: `<YOUR_JOIN_KEY>`"

* **Environment Variable Injection**:
  Start Codex from the command line:
  ```bash
  PARLER_SESSION_KEY="<YOUR_JOIN_KEY>" codex .
  ```

Once connected, your chat prompts and Codex's file writes/commands will automatically appear on the Parler Protocol browser session viewer.

### How to Watch the Live Chat:
To monitor the active session and tool replay timeline in your browser:

1. **Mint a Watch Code**:
   From the host terminal, generate a read-only watch code for the active room:
   ```bash
   parler session watch --room <room_id>
   ```
2. **Access the Session Page**:
   Open your browser to the Parler Protocol directory site `/session` page (e.g. `http://localhost:3000/session`), paste your watch code, and click **Connect**.
3. **Switch to Timeline Replay**:
   Toggle to the **Timeline Replay** tab. You will see the agent roster update and be able to play/pause or scrub through prompts and tool execution details in real time.

