import { cn } from "@/lib/utils";

/**
 * A faithful, static mock of two Claude Code sessions doing a Parler Protocol mid-chat handoff:
 * the lead agent (atlas) publishes the conversation and gets a key; a brand-new agent asks to join
 * in a single `claude mcp add … -e PARLER_SESSION_KEY=…` line, the host approves it, and only then
 * is it caught up. The approval step is the security gate — a leaked key can't read the context
 * without the host's explicit OK.
 *
 * Purely presentational — no live terminal, just styled text that reads like the real TUI.
 */

/** Claude's clay accent, used for the assistant bullet + the welcome star. */
const CLAY = "#d97757";

type Row =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string }
  | { kind: "tool"; name: string }
  | { kind: "result"; text: string; tone?: "green" | "steel" }
  | { kind: "cmd"; text: string }
  | { kind: "note"; text: string }
  | { kind: "gap" };

/** Render one logical row, splitting multi-line text and indenting continuations. */
function RowView({ row }: { row: Row }) {
  if (row.kind === "gap") return <div className="h-3" aria-hidden />;

  const lines = "text" in row ? row.text.split("\n") : [""];

  if (row.kind === "user") {
    // The human's turn — deliberately the loudest thing in the transcript: a lifted box with an
    // electric-blue accent rail and bright, semibold text, so the eye lands on it before Claude's reply.
    return (
      <div className="flex gap-2.5 rounded-[7px] border border-graphite-rail border-l-2 border-l-electric-blue bg-surface-lift px-3 py-2">
        <span className="select-none font-semibold text-electric-blue">&gt;</span>
        <span className="font-semibold text-pure-white">
          {lines.map((l, i) => (
            <span key={i} className="block">
              {l}
            </span>
          ))}
        </span>
      </div>
    );
  }

  if (row.kind === "assistant") {
    // Claude's turn — readable but deliberately a step quieter than the user's bright box above.
    return (
      <div className="flex gap-2">
        <span className="select-none" style={{ color: CLAY }}>
          ⏺
        </span>
        <span className="text-mist">
          {lines.map((l, i) => (
            <span key={i} className="block">
              {l}
            </span>
          ))}
        </span>
      </div>
    );
  }

  if (row.kind === "tool") {
    return (
      <div className="flex gap-2">
        <span className="select-none" style={{ color: CLAY }}>
          ⏺
        </span>
        <span>
          <span className="text-resend-violet">parler</span>
          <span className="text-mist"> - {row.name}</span>
          <span className="text-steel"> (MCP)</span>
        </span>
      </div>
    );
  }

  if (row.kind === "result") {
    const tone = row.tone === "green" ? "text-delivered-green" : "text-steel";
    return (
      <div className="flex gap-2 pl-2">
        <span className="select-none text-steel">⎿</span>
        <span className={tone}>
          {lines.map((l, i) => (
            <span key={i} className="block">
              {l}
            </span>
          ))}
        </span>
      </div>
    );
  }

  if (row.kind === "cmd") {
    return (
      <div className="flex gap-2">
        <span className="select-none text-steel">$</span>
        <span className="text-frost">
          {lines.map((l, i) => (
            <span key={i} className="block">
              {i === 0 ? <span className="text-resend-violet">claude</span> : null}
              {i === 0 ? l.replace(/^claude/, "") : l}
            </span>
          ))}
        </span>
      </div>
    );
  }

  // note
  return <div className="text-steel">{row.text}</div>;
}

function Terminal({
  title,
  subtitle,
  rows,
  hint,
  className,
}: {
  title: string;
  subtitle: string;
  rows: Row[];
  hint: string;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex flex-col overflow-hidden rounded-[12px] border border-graphite-rail bg-void-black",
        className,
      )}
    >
      {/* window chrome */}
      <div className="flex items-center gap-2 border-b border-graphite-rail px-4 py-2.5">
        <span className="size-2.5 rounded-full bg-graphite-rail" />
        <span className="size-2.5 rounded-full bg-graphite-rail" />
        <span className="size-2.5 rounded-full bg-graphite-rail" />
        <span className="ml-2 font-mono text-[12px] text-mist">{title}</span>
      </div>

      {/* welcome line */}
      <div className="flex items-center gap-2 px-5 pt-4 font-mono text-[12px]">
        <span style={{ color: CLAY }}>✻</span>
        <span className="text-frost">Welcome to Claude Code</span>
        <span className="text-steel">· {subtitle}</span>
      </div>

      {/* transcript */}
      <div className="flex-1 space-y-2 px-5 py-4 font-mono text-[12.5px] leading-[1.6]">
        {rows.map((r, i) => (
          <RowView key={i} row={r} />
        ))}
      </div>

      {/* prompt input box */}
      <div className="px-4 pb-4">
        <div className="flex items-center gap-2 rounded-[8px] border border-graphite-rail px-3 py-2 font-mono text-[12.5px]">
          <span className="text-steel">&gt;</span>
          <span className="inline-block h-[14px] w-[7px] animate-pulse bg-frost align-middle" />
        </div>
        <div className="mt-1.5 flex items-center justify-between px-1 font-mono text-[11px] text-steel">
          <span>? for shortcuts</span>
          <span>{hint}</span>
        </div>
      </div>
    </div>
  );
}

const ATLAS_ROWS: Row[] = [
  {
    kind: "user",
    text: "Open a Parler Protocol session — summarize what we've been\nworking on, and give me the key.",
  },
  { kind: "gap" },
  {
    kind: "assistant",
    text: "Publishing our context so another agent can pick it up\nwhere we are.",
  },
  { kind: "gap" },
  { kind: "tool", name: "open_session" },
  {
    kind: "result",
    tone: "green",
    text: "KEY: A3KELDJR\nroom auth-redesign · context posted (5 msgs)\napproval required — you admit each joiner",
  },
  { kind: "gap" },
  {
    kind: "assistant",
    text: "Done — share this key:  A3KELDJR\n\nI'll vet whoever tries to join before they can\nread our context.",
  },
  { kind: "gap" },
  { kind: "tool", name: "recv" },
  {
    kind: "result",
    text: "⏳ codex is asking to JOIN auth-redesign\n   approve? [Uo3F…2K]",
  },
  { kind: "gap" },
  { kind: "user", text: "Yes — let codex in." },
  { kind: "gap" },
  { kind: "tool", name: "approve_join" },
  {
    kind: "result",
    tone: "green",
    text: "✓ approved codex — it can now read the\nconversation and reply",
  },
];

const JOINER_ROWS: Row[] = [
  { kind: "note", text: "# the whole mid-chat setup — one line:" },
  {
    kind: "cmd",
    text: "claude mcp add parler \\\n    -e PARLER_SESSION_KEY=A3KELDJR -- parler mcp",
  },
  {
    kind: "result",
    text: 'Added MCP server "parler" ✔\nparler: join request sent — waiting for the\nhost to approve',
  },
  { kind: "gap" },
  {
    kind: "result",
    tone: "green",
    text: "parler: ✓ approved by host — joined\nauth-redesign, caught up on the full context",
  },
  { kind: "gap" },
  { kind: "user", text: "Where did we land on auth?" },
  { kind: "gap" },
  {
    kind: "assistant",
    text: "I'm in the session now — full context loaded:\n\n  • Designing auth in src/auth.rs\n  • Chose PKCE + refresh tokens\n  • TODO: token rotation",
  },
  { kind: "gap" },
  { kind: "assistant", text: "I'll take token rotation and post back to the room." },
  { kind: "tool", name: "send" },
  { kind: "result", text: "→ auth-redesign: \"on it — implementing rotation\"" },
];

export function ClaudeSim({ className }: { className?: string }) {
  return (
    <div className={cn("grid grid-cols-1 gap-4 lg:grid-cols-2 lg:items-stretch", className)}>
      <div className="flex flex-col">
        <p className="mb-2 flex items-center gap-2 px-1 text-[13px] text-fog">
          <span className="font-mono text-electric-blue">agent A</span>
          opens the session &amp; admits joiners
        </p>
        <Terminal
          title="atlas — claude code"
          subtitle="~/proj/auth"
          rows={ATLAS_ROWS}
          hint="auth-redesign"
          className="flex-1"
        />
      </div>

      <div className="flex flex-col">
        <p className="mb-2 flex items-center gap-2 px-1 text-[13px] text-fog">
          <span className="font-mono text-delivered-green">agent B</span>
          asks to join — host approves
        </p>
        <Terminal
          title="new agent — claude code"
          subtitle="~/proj/auth"
          rows={JOINER_ROWS}
          hint="auth-redesign"
          className="flex-1"
        />
      </div>
    </div>
  );
}
