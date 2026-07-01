import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { parler } from "@/lib/ipc";
import { cn } from "@/lib/utils";

/** A small copy-to-clipboard button that flips to a check for a beat. */
export function CopyButton({
  value,
  className,
  label,
}: {
  value: string;
  className?: string;
  label?: string;
}) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    await parler.clipboard.write(value);
    setCopied(true);
    setTimeout(() => setCopied(false), 1400);
  };
  return (
    <button
      onClick={copy}
      className={cn(
        "no-drag inline-flex items-center gap-1.5 rounded-[6px] border border-graphite-rail px-2 py-1 text-[12px] text-fog transition-colors hover:border-smoke hover:text-frost",
        className,
      )}
      title="Copy"
    >
      {copied ? <Check className="size-3.5 text-delivered-green" /> : <Copy className="size-3.5" />}
      {label ?? (copied ? "Copied" : "Copy")}
    </button>
  );
}

/** A monospace code block with a copy affordance in the corner. */
export function CodeBlock({ code, className }: { code: string; className?: string }) {
  return (
    <div className={cn("group relative rounded-[12px] border border-graphite-rail bg-black/40", className)}>
      <pre
        data-selectable
        className="overflow-x-auto whitespace-pre-wrap break-all px-4 py-3.5 font-mono text-[12.5px] leading-relaxed text-mist"
      >
        {code}
      </pre>
      <div className="absolute right-2 top-2 opacity-0 transition-opacity group-hover:opacity-100">
        <CopyButton value={code} className="bg-void-black" />
      </div>
    </div>
  );
}
