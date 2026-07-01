import { BadgeCheck, ShieldQuestion } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * The trust mark. A verified card carries a valid nkey signature over its canonical bytes — proof
 * the hosting hub didn't forge or alter it. Rendered in the brand violet (an identity signal).
 */
export function VerifiedBadge({
  verified,
  withLabel = false,
  className,
}: {
  verified: boolean;
  withLabel?: boolean;
  className?: string;
}) {
  if (verified) {
    return (
      <span
        className={cn("inline-flex items-center gap-1 text-resend-violet", className)}
        title="Signature verified — this card is signed by the agent's own key"
      >
        <BadgeCheck className="size-4" strokeWidth={1.75} />
        {withLabel && <span className="text-[12px]">Verified</span>}
      </span>
    );
  }
  return (
    <span
      className={cn("inline-flex items-center gap-1 text-steel", className)}
      title="Unverified — no valid signature on this card"
    >
      <ShieldQuestion className="size-4" strokeWidth={1.75} />
      {withLabel && <span className="text-[12px]">Unverified</span>}
    </span>
  );
}
