import { cn } from "@/lib/utils";

/**
 * Presence → one of the Resend vivid status colors. These are the only saturated colors on the
 * canvas (reserved for data): green = working, blue = idle, yellow = waiting, muted steel = offline.
 */
const STATUS: Record<string, { color: string; label: string }> = {
  working: { color: "#3ad389", label: "Working" },
  idle: { color: "#70b8ff", label: "Idle" },
  waiting: { color: "#ffca16", label: "Waiting" },
  offline: { color: "#6c6c6c", label: "Offline" },
};

export function statusMeta(status: string) {
  return (
    STATUS[status] ?? {
      color: "#6c6c6c",
      label: status.charAt(0).toUpperCase() + status.slice(1),
    }
  );
}

export function StatusDot({ status, className }: { status: string; className?: string }) {
  const { color } = statusMeta(status);
  const live = status !== "offline";
  return (
    <span className={cn("relative inline-flex size-2 shrink-0 items-center justify-center", className)}>
      {live && (
        <span
          className="absolute inline-flex size-full animate-ping rounded-full opacity-60"
          style={{ backgroundColor: color }}
        />
      )}
      <span
        className="relative inline-flex size-2 rounded-full"
        style={{ backgroundColor: color, opacity: live ? 1 : 0.55 }}
      />
    </span>
  );
}

export function StatusLabel({ status }: { status: string }) {
  const { color, label } = statusMeta(status);
  return (
    <span className="inline-flex items-center gap-1.5 text-[13px]" style={{ color }}>
      <StatusDot status={status} />
      {label}
    </span>
  );
}
