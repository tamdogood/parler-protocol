import { useEffect } from "react";
import { X, Lock } from "lucide-react";
import type { DirectoryEntry } from "@/lib/types";
import { relativeTime } from "@/lib/utils";
import { StatusLabel } from "@/components/status-dot";
import { VerifiedBadge } from "@/components/verified-badge";
import { Badge } from "@/components/ui/badge";
import { CopyButton } from "@/components/copyable";

/** A right-side drawer with the full agent card. Esc / backdrop closes it. */
export function AgentDetail({ entry, onClose }: { entry: DirectoryEntry; onClose: () => void }) {
  const { card } = entry;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-40 flex justify-end" role="dialog" aria-modal>
      <div className="absolute inset-0 bg-black/60 animate-[scale-up-fade_.15s_ease]" onClick={onClose} />
      <aside className="relative z-10 flex h-full w-full max-w-[440px] flex-col overflow-y-auto border-l border-graphite-rail bg-void-black">
        <div className="flex items-start justify-between gap-3 border-b border-graphite-rail p-6">
          <div>
            <div className="flex items-center gap-2">
              <h3 className="text-[20px] font-semibold text-pure-white">{card.name}</h3>
              {entry.visibility === "private" && <Lock className="size-3.5 text-steel" />}
              <VerifiedBadge verified={entry.verified} />
            </div>
            {card.role && <p className="mt-0.5 text-[13px] text-fog">{card.role}</p>}
            <div className="mt-2">
              <StatusLabel status={entry.status} />
            </div>
          </div>
          <button
            onClick={onClose}
            className="no-drag rounded-[8px] border border-graphite-rail p-1.5 text-steel transition-colors hover:text-frost"
          >
            <X className="size-4" />
          </button>
        </div>

        <div className="flex flex-col gap-5 p-6">
          <Field label="Agent id">
            <div className="flex items-center gap-2">
              <code className="min-w-0 flex-1 truncate rounded-[8px] border border-graphite-rail bg-black/40 px-2.5 py-1.5 font-mono text-[12px] text-mist" data-selectable>
                {card.id}
              </code>
              <CopyButton value={card.id} label="" />
            </div>
          </Field>

          {card.description && (
            <Field label="About">
              <p className="text-[13px] leading-relaxed text-fog" data-selectable>
                {card.description}
              </p>
            </Field>
          )}

          {card.tags && card.tags.length > 0 && (
            <Field label="Tags">
              <div className="flex flex-wrap gap-1.5">
                {card.tags.map((t) => (
                  <Badge key={t}>{t}</Badge>
                ))}
              </div>
            </Field>
          )}

          {card.skills && card.skills.length > 0 && (
            <Field label="Skills">
              <ul className="flex flex-col gap-2">
                {card.skills.map((s) => (
                  <li key={s.id} className="rounded-[10px] border border-graphite-rail bg-black/30 p-3">
                    <p className="text-[13px] font-medium text-frost">{s.name}</p>
                    {s.description && <p className="mt-0.5 text-[12px] text-steel">{s.description}</p>}
                  </li>
                ))}
              </ul>
            </Field>
          )}

          <div className="grid grid-cols-2 gap-4">
            <Field label="Visibility">
              <span className="text-[13px] capitalize text-fog">{entry.visibility}</span>
            </Field>
            <Field label="Last seen">
              <span className="text-[13px] text-fog">{relativeTime(entry.lastSeen)}</span>
            </Field>
            {card.protocolVersion && (
              <Field label="Protocol">
                <span className="font-mono text-[13px] text-fog">v{card.protocolVersion}</span>
              </Field>
            )}
            <Field label="First seen">
              <span className="text-[13px] text-fog">{relativeTime(entry.firstSeen)}</span>
            </Field>
          </div>
        </div>
      </aside>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <p className="mb-1.5 text-[11px] uppercase tracking-wide text-steel">{label}</p>
      {children}
    </div>
  );
}
