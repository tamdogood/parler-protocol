import { useEffect, useState } from "react";
import { MessagesSquare, Plus, KeyRound, Eye, Loader2, Sparkles, ShieldCheck } from "lucide-react";
import type { HubStatus, OpenedSession } from "@shared/types";
import { parler } from "@/lib/ipc";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { CopyButton } from "@/components/copyable";
import { SessionViewer } from "@/components/session-viewer";

/**
 * Sessions = open a live session (mint a KEY + watch code, seeded with a recap) and watch any session
 * you hold a watch code for. Opening runs the bundled `parler` against the app's operative hub (the
 * local hub when it's running, else the public hub).
 */
export function SessionsScreen({
  base,
  localUrl,
  publicUrl,
  status,
}: {
  base: string;
  localUrl: string | null;
  publicUrl: string;
  status: HubStatus | null;
}) {
  const openOnLocal = status?.phase === "running";
  const openBase = openOnLocal ? localUrl ?? publicUrl : publicUrl;

  const [watchBase, setWatchBase] = useState(base);
  const [watchToken, setWatchToken] = useState<string | undefined>(undefined);
  useEffect(() => {
    setWatchBase(base);
    setWatchToken(undefined);
  }, [base]);

  return (
    <div className="mx-auto max-w-[900px] px-8 py-8">
      <div className="flex items-center gap-3">
        <span className="flex size-11 items-center justify-center rounded-[12px] border border-graphite-rail surface-lift">
          <MessagesSquare className="size-5 text-electric-blue" />
        </span>
        <div>
          <h1 className="text-[22px] font-semibold tracking-tight text-pure-white">Sessions</h1>
          <p className="text-[13px] text-fog">Hand off a live conversation, then watch it play out — no copy-paste.</p>
        </div>
      </div>

      <OpenSessionPanel
        openOnLocal={openOnLocal}
        onWatchHere={(token) => {
          setWatchBase(openBase);
          setWatchToken(token);
        }}
      />

      <div className="mt-8 border-t border-graphite-rail pt-8">
        <SessionViewer base={watchBase} initialToken={watchToken} />
      </div>
    </div>
  );
}

function OpenSessionPanel({
  openOnLocal,
  onWatchHere,
}: {
  openOnLocal: boolean;
  onWatchHere: (token: string) => void;
}) {
  const [context, setContext] = useState("");
  const [topic, setTopic] = useState("");
  const [noApproval, setNoApproval] = useState(false);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<OpenedSession | null>(null);
  const [error, setError] = useState<string | null>(null);

  const open = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await parler.session.open({ context: context.trim() || undefined, topic: topic.trim() || undefined, noApproval });
      setResult(r);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to open the session.");
    } finally {
      setBusy(false);
    }
  };

  if (result) {
    return (
      <div className="mt-6 rounded-[16px] border border-electric-blue/40 bg-electric-blue/[0.04] p-6">
        <div className="flex items-center gap-2 text-[14px] font-semibold text-pure-white">
          <Sparkles className="size-4 text-electric-blue" /> Session open
        </div>
        <p className="mt-1 text-[13px] text-fog">
          Hand the <b className="text-frost">key</b> to another agent to join. The <b className="text-frost">watch code</b> is
          read-only — paste it below or into the website viewer.
        </p>

        <Field label="Session key (to join)">
          <TokenRow value={result.key} />
        </Field>
        {result.watch && (
          <Field label="Watch code (read-only)">
            <TokenRow value={result.watch} />
          </Field>
        )}
        <p className="mt-2 font-mono text-[11px] text-steel">room: {result.room}</p>

        <div className="mt-4 flex flex-wrap gap-2">
          {result.watch && (
            <Button variant="primary" size="sm" onClick={() => onWatchHere(result.watch as string)}>
              <Eye className="size-3.5" /> Watch here
            </Button>
          )}
          <Button variant="outline" size="sm" onClick={() => setResult(null)}>
            Open another
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="mt-6 rounded-[16px] border border-graphite-rail bg-void-black p-6">
      <div className="flex items-center gap-2 text-[14px] font-semibold text-frost">
        <Plus className="size-4 text-electric-blue" /> Open a session
      </div>
      <p className="mt-1 text-[13px] text-fog">
        Opens on your <b className="text-frost">{openOnLocal ? "local hub" : "the public hub"}</b>
        {openOnLocal ? "" : " (start your local hub to host it privately)"}.
      </p>

      <div className="mt-4 flex flex-col gap-3">
        <label className="block">
          <span className="mb-1.5 block text-[11px] uppercase tracking-wide text-steel">Context recap (seeds the room)</span>
          <textarea
            value={context}
            onChange={(e) => setContext(e.target.value)}
            rows={4}
            placeholder="Where we are, what's decided, what's next…"
            className="no-drag w-full resize-y rounded-[10px] border border-graphite-rail bg-transparent px-3 py-2.5 text-[14px] text-frost placeholder:text-steel outline-none transition-colors focus:border-electric-blue/70 focus:ring-1 focus:ring-electric-blue/40"
          />
        </label>
        <div className="flex flex-col gap-3 sm:flex-row sm:items-end">
          <label className="block flex-1">
            <span className="mb-1.5 block text-[11px] uppercase tracking-wide text-steel">Topic (optional)</span>
            <Input value={topic} onChange={(e) => setTopic(e.target.value)} placeholder="e.g. payments-refactor" />
          </label>
          <label className="no-drag flex cursor-pointer items-center gap-2 pb-2.5 text-[13px] text-fog">
            <input type="checkbox" checked={noApproval} onChange={(e) => setNoApproval(e.target.checked)} className="accent-electric-blue" />
            Skip join approval
          </label>
        </div>
      </div>

      {error && <p className="mt-3 text-[13px] text-bounced-red">{error}</p>}

      <div className="mt-4 flex items-center gap-3">
        <Button variant="primary" onClick={open} disabled={busy}>
          {busy ? <Loader2 className="size-4 animate-spin" /> : <KeyRound className="size-4" />} Open session
        </Button>
        <span className="flex items-center gap-1.5 text-[12px] text-steel">
          <ShieldCheck className="size-3.5 text-delivered-green" />
          Joiners need your approval unless you skip it.
        </span>
      </div>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="mt-4">
      <p className="mb-1.5 text-[11px] uppercase tracking-wide text-steel">{label}</p>
      {children}
    </div>
  );
}

function TokenRow({ value }: { value: string }) {
  return (
    <div className="flex items-center gap-2">
      <code className="min-w-0 flex-1 truncate rounded-[8px] border border-graphite-rail bg-black/40 px-2.5 py-1.5 font-mono text-[12.5px] text-mist" data-selectable>
        {value}
      </code>
      <CopyButton value={value} label="" />
    </div>
  );
}
