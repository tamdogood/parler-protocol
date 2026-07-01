import { Lock } from "lucide-react";
import type { DirectoryEntry } from "@/lib/types";
import { cn, relativeTime, shortId } from "@/lib/utils";
import { StatusDot, statusMeta } from "@/components/status-dot";
import { VerifiedBadge } from "@/components/verified-badge";
import { Badge } from "@/components/ui/badge";

export function AgentCard({ entry, onSelect }: { entry: DirectoryEntry; onSelect?: (e: DirectoryEntry) => void }) {
  const { card } = entry;
  const status = statusMeta(entry.status);
  const tags = card.tags ?? [];

  return (
    <button
      onClick={() => onSelect?.(entry)}
      className={cn(
        "group relative flex h-full flex-col rounded-[16px] border border-graphite-rail bg-void-black p-5 text-left transition-colors duration-150 ease-out",
        "hover:border-smoke focus:outline-none focus-visible:border-electric-blue/70",
      )}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="flex items-center gap-2.5">
          <StatusDot status={entry.status} className="mt-px" />
          <div>
            <div className="flex items-center gap-1.5">
              <span className="text-[15px] font-semibold leading-tight text-pure-white">{card.name}</span>
              {entry.visibility === "private" && <Lock className="size-3 text-steel" aria-label="private" />}
            </div>
            {card.role && <span className="text-[13px] text-fog">{card.role}</span>}
          </div>
        </div>
        <VerifiedBadge verified={entry.verified} />
      </div>

      <div className="mt-3 font-mono text-[12px] text-steel">{shortId(card.id, 8, 6)}</div>

      {card.description && (
        <p className="mt-3 line-clamp-2 text-[13px] leading-relaxed text-fog">{card.description}</p>
      )}

      {tags.length > 0 && (
        <div className="mt-4 flex flex-wrap gap-1.5">
          {tags.slice(0, 4).map((t) => (
            <Badge key={t}>{t}</Badge>
          ))}
          {tags.length > 4 && <Badge>+{tags.length - 4}</Badge>}
        </div>
      )}

      <div className="mt-5 flex items-center justify-between border-t border-graphite-rail/70 pt-3 text-[12px] text-steel">
        <span style={{ color: status.color }} className="font-medium">
          {status.label}
        </span>
        <span>{entry.status === "offline" ? `seen ${relativeTime(entry.lastSeen)}` : "active now"}</span>
      </div>
    </button>
  );
}
