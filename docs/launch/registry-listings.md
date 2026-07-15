# Track C: registry listings checklist

Evergreen passive distribution. List Parler Protocol once in every place a self-selected MCP buyer
already shops, and get discovered forever. This is the "few hours, once, week 1" task from
[`docs/research/parler-launch-plan.md`](../research/parler-launch-plan.md) section 5. Work top to
bottom; each box is meant to be finished in one sitting.

**One thing to do first (it unblocks the biggest listing):** the official MCP Registry hosts
*metadata, not artifacts*. It will not accept `cargo install --git` or a raw source build. Every
supported package type (`npm`, `pypi`, `nuget`, `oci`, `mcpb`) points at an artifact published to a
public registry, and each has an ownership-verification hook. So before the official registry accepts
Parler, a human must publish one artifact. See the "What a human must publish" box below. Every other
registry here (mcp.so, Smithery, Glama, LobeHub, the awesome-lists) accepts a plain GitHub repo URL
today, so do those regardless.

Repo: `https://github.com/tamdogood/parler-protocol` · Site: `https://www.parlerprotocol.com` ·
Registry name: `io.github.tamdogood/parler` · Install: one command (below).

---

## The ready-to-paste copy (used by every listing)

**Name:** `Parler Protocol`

**One-liner (the wedge):**
> Share one live coding-agent conversation across Claude Code, Codex, and OpenCode in about 10
> seconds. No copy-paste, no re-briefing; other MCP hosts get the same messaging and memory tools.

**Short description (2-3 sentences):**
> Parler Protocol (no relation to the social app) is one small Rust binary that lets independent AI
> coding agents share a live conversation without copy-paste. One visible Claude Code, Codex, or
> OpenCode agent starts `parler conversation` and shares a portable key; the next visible agent joins
> the same chat already caught up, across tools and machines. It also gives MCP hosts verifiable
> identity, messaging, a searchable directory, file/code transfer, and shared memory over a tiny
> WebSocket hub.

**Category / tags:** `agents`, `agent-coordination`, `multi-agent`, `session-handoff`, `memory`,
`directory`, `mcp`, `rust`, `websocket`, `cli`. Primary category: agent coordination / aggregators.

**Transport:** `stdio` (run as `parler mcp`).

**Install command (one line, prebuilt binary):**
```bash
curl -fsSL https://raw.githubusercontent.com/tamdogood/parler-protocol/main/scripts/install.sh | sh && parler connect
```

**From source:**
```bash
cargo install --git https://github.com/tamdogood/parler-protocol parler-bin && parler connect
```

**MCP config snippet (for registries that render one inline):**
```json
{
  "mcpServers": {
    "parler": {
      "command": "parler",
      "args": ["mcp"]
    }
  }
}
```

> **Defusal clause — paste once, high in any description that shows a name.** "Parler Protocol (no
> relation to the social app)…". State it once, then never mention it again. It costs one clause and
> saves you the "like the right-wing app?" reaction on every listing.

---

## What a human must publish (unblocks the official registry)

The official registry needs one verifiable artifact. Cheapest correct option first:

- **Option A — MCPB bundle (recommended, no new package registry account).** Build a `.mcpb` bundle
  that wraps the `parler` binary invoked as `parler mcp`, attach it to a GitHub Release on
  `tamdogood/parler-protocol`, then fill the two placeholders in [`/server.json`](../../server.json):
  - `packages[0].identifier` → the release download URL (it MUST contain the string `mcp`; the
    `.mcpb` extension already satisfies that).
  - `packages[0].fileSha256` → `openssl dgst -sha256 parler-mcp.mcpb`.
  - `version` may be dropped for mcpb (it is not applicable to direct downloads).
- **Option B — npm wrapper.** Publish a thin npm package (e.g. `@tamdogood/parler-mcp`) whose bin
  shells out to the installed `parler mcp`, add `"mcpName": "io.github.tamdogood/parler"` to its
  `package.json`, and change `server.json` `registryType` to `npm`. Most familiar path, but adds an
  npm package to maintain.
- **Option C — OCI image.** There is already `ghcr.io/tamdogood/parler-hub`, but that image is the
  *hub*, not the `parler mcp` stdio client, so it does not satisfy this listing as-is. A dedicated
  `ghcr.io/tamdogood/parler-mcp` image with the label
  `io.modelcontextprotocol.server.name=io.github.tamdogood/parler` would.

`server.json` is committed at the repo root with the mcpb path pre-filled and the two
publish-time fields marked `FILL_ME`. Nothing else in it needs editing.

---

## 1. Official MCP Registry  🎖️  *(verified 2026-07-05)*

- **URL:** https://registry.modelcontextprotocol.io/ · quickstart:
  https://modelcontextprotocol.io/registry/quickstart
- **Why it matters:** the verified backbone (Anthropic/GitHub/Microsoft-backed); feeds many clients
  programmatically. Worth the extra publish step.
- **Prereq:** publish one artifact per the box above, and the GitHub account `tamdogood` (the
  `io.github.tamdogood/` namespace authenticates via GitHub device login).
- **Steps:**
  1. Install the publisher CLI:
     ```bash
     brew install mcp-publisher
     # or: curl -L "https://github.com/modelcontextprotocol/registry/releases/latest/download/mcp-publisher_$(uname -s | tr '[:upper:]' '[:lower:]')_$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/').tar.gz" | tar xz mcp-publisher && sudo mv mcp-publisher /usr/local/bin/
     ```
  2. From the repo root (where `server.json` already lives), authenticate:
     ```bash
     mcp-publisher login github
     ```
     Namespace rule: with GitHub auth the server name MUST start with `io.github.tamdogood/` — it
     already does.
  3. Validate then publish:
     ```bash
     mcp-publisher publish --dry-run
     mcp-publisher publish
     ```
  4. Confirm:
     ```bash
     curl "https://registry.modelcontextprotocol.io/v0.1/servers?search=io.github.tamdogood/parler"
     ```
- **Copy:** use the name, one-liner, and description above. `server.json` already carries them.

---

## 2. mcp.so  *(verified 2026-07-05)*

- **URL:** https://mcp.so · submit: https://mcp.so/submit
- **Why it matters:** the largest third-party marketplace (20k+ servers), where people browse for
  tools by problem.
- **Steps:** open the submit page, sign in with GitHub, and fill the form. It asks for: server name,
  a one-sentence capability description, tool count, transport type, GitHub repository URL, homepage
  URL, and an optional icon. It renders your config snippet inline, so paste the MCP config snippet
  above.
- **Fields to paste:**
  - Name: `Parler Protocol`
  - Description: the one-liner.
  - Tool count: **29** (`parler_open_session`, `parler_join_session`, `parler_close_session`,
    `parler_delete_room`, `parler_join_requests`, `parler_approve_join`, `parler_deny_join`,
    `parler_watch_session`, `parler_register`, `parler_discover`, `parler_card`, `parler_send`,
    `parler_recv`, `parler_handoff`, `parler_task`, `parler_bring`, `parler_push`,
    `parler_send_file`, `parler_fetch`, `parler_apply`, `parler_invite`, `parler_join`,
    `parler_serve`, `parler_remember`, `parler_recall`, `parler_rooms`, `parler_roster`,
    `parler_presence`, `parler_attention`). Re-count against the README
    "full MCP tool surface" list before pasting; keep it honest.
  - Transport: `stdio`
  - Repo: `https://github.com/tamdogood/parler-protocol`
  - Homepage: `https://www.parlerprotocol.com`
  - Config snippet: the JSON block above.

---

## 3. Smithery.ai  *(verified 2026-07-05)*

- **URL:** https://smithery.ai · new server: https://smithery.ai/new
- **Why it matters:** major registry with real client traffic; many IDE integrations pull from it.
- **Steps:** create a publisher account (GitHub sign-in), then either connect the GitHub repo through
  the "new server" flow or publish from the CLI:
  ```bash
  npm install -g @smithery/cli
  smithery mcp publish https://github.com/tamdogood/parler-protocol -n tamdogood/parler
  ```
  Provide a manifest describing name, description, tools, and auth method (`none` — Parler has no API
  key; identity is self-minted on connect).
- **Note (unverified specifics):** Smithery historically favored hosted/remote servers and may want a
  `smithery.yaml` in the repo. If the CLI flags a missing manifest, follow its prompt or the docs at
  https://smithery.ai/docs — the exact manifest schema was not re-verified in this pass. Auth method
  to select: **none**.
- **Copy:** name + one-liner + description above; tags list above.

---

## 4. Glama.ai / mcp  *(verified 2026-07-05)*

- **URL:** https://glama.ai/mcp/servers
- **Why it matters:** major registry; tends to feature production-quality servers with real docs, so
  Parler's README + docs give it a good shot at being featured, not just listed.
- **Steps:** Glama auto-indexes public GitHub repos tagged/related to MCP, but you can submit and then
  **claim** the server to control its listing. Sign in with GitHub, find or add
  `tamdogood/parler-protocol`, and claim it. Provide: name, description, repository URL, installation
  snippet, transport, tool count, and a one-line capability summary.
- **Tip:** link the real README with the one-command install (Glama rewards a working install guide
  over a raw git URL). Point the install field at the `curl … | sh && parler connect` line.
- **Copy:** name + one-liner + description + install command above.

---

## 5. LobeHub MCP store  *(steps unverified — confirm on-site)*

- **URL:** https://lobehub.com/mcp · submit: https://lobehub.com/mcp/publish (confirm the exact path
  on the site; not re-verified in this pass).
- **Why it matters:** LobeHub's MCP marketplace surfaces servers to its large chat-UI user base.
- **Steps (expected):** sign in with GitHub, choose "submit/publish an MCP server," and provide the
  repo URL, name, description, transport, and the MCP config snippet. Some LobeHub submissions go
  through a PR to a manifest repo rather than a web form — follow whichever the publish page presents.
- **Copy:** name + one-liner + description + config snippet above.
- **⚠️ Verify before submitting:** the exact submit URL and whether it is a web form or a GitHub PR.

---

## 6. punkpeye/awesome-mcp-servers (GitHub PR)  *(verified 2026-07-05)*

- **URL:** https://github.com/punkpeye/awesome-mcp-servers
- **Why it matters:** the canonical community awesome-list; high traffic, and being on it seeds the
  smaller lists that copy from it.
- **Steps:** fork → add one line to `README.md` in the right category, alphabetized → PR titled
  `Add Parler Protocol`. One server per line; match existing formatting exactly.
- **Category:** best fit is **🔗 Aggregators** (multi-agent orchestration / agent-to-agent), with
  **💬 Communication** as the fallback if Aggregators has tightened its scope.
- **Legend emojis for this entry:** 🦀 (Rust) 🏠 (local service) ☁️ (also runs as a shared hub) 🍎🐧
  (macOS + Linux prebuilt).
- **Ready-to-paste line** (place alphabetically under the chosen heading):
  ```markdown
  - [tamdogood/parler-protocol](https://github.com/tamdogood/parler-protocol) 🦀 🏠 ☁️ 🍎 🐧 - Share one live coding-agent conversation across Claude Code, Codex, and OpenCode in ~10s — no copy-paste, no re-briefing. Verifiable agent identity, directory, and shared memory over a tiny WebSocket hub. CLI + MCP server.
  ```
  > House rule note: the repo forbids em dashes in *its own* copy, but this line lives in an external
  > repo whose format uses them. Match punkpeye's house style there; the em dash above is theirs, not
  > ours.

---

## 7. Adjacent awesome-lists (one PR each)  *(lists verified; per-list format not re-verified)*

Same PR pattern as #6: fork, add one alphabetized line matching the list's format, PR titled
`Add Parler Protocol`. Confirm each list still accepts entries and read its CONTRIBUTING before you
open the PR.

- [ ] **awesome-ai-agents** — https://github.com/e2b-dev/awesome-ai-agents (agent frameworks/tools;
      strong fit for agent-coordination). *Verify the exact section + line format on the day.*
- [ ] **awesome-mcp-servers (appcypher)** — https://github.com/appcypher/awesome-mcp-servers (a
      second large MCP list independent of punkpeye).
- [ ] **awesome-claude** — search `github.com awesome-claude` and pick the actively maintained one;
      *the canonical repo owner was not verified in this pass — confirm before PRing.*
- [ ] **awesome-devtools** — https://github.com/... *(no single canonical repo verified; confirm the
      most-starred active one, e.g. an "awesome developer tools" list, before submitting.)*

**Ready-to-paste line for these lists** (adjust emoji/format to each list's convention):
```markdown
- [Parler Protocol](https://github.com/tamdogood/parler-protocol) - Share one live coding-agent conversation across Claude Code, Codex, and OpenCode in about 10 seconds, no copy-paste and no re-briefing. Verifiable agent identity, a searchable directory, and shared memory over a tiny WebSocket hub. CLI + MCP server, Rust.
```

---

## Done-when

- [ ] Official MCP Registry: artifact published + `mcp-publisher publish` succeeded + search returns it.
- [ ] mcp.so: form submitted, listing live.
- [ ] Smithery.ai: server published/claimed.
- [ ] Glama.ai: server claimed with install guide.
- [ ] LobeHub: submitted (after confirming the submit path).
- [ ] punkpeye/awesome-mcp-servers: PR open.
- [ ] 2+ adjacent awesome-lists: PRs open.

Set and forget. Re-check the official-registry listing after any version bump (re-run
`mcp-publisher publish` with the new `version`).

---

### Links that could not be fully verified in this pass
- **LobeHub** exact submit URL / form-vs-PR (section 5).
- **Smithery** current manifest schema / whether a `smithery.yaml` is required (section 3).
- **awesome-claude** and **awesome-devtools** canonical repo owners (section 7) — multiple candidates
  exist; confirm the actively maintained one before opening a PR.
- **mcp.so tool count (22)** — derived from the README's MCP tool surface; re-count before pasting.
