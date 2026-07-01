import * as React from "react";
import { cn } from "@/lib/utils";

/** A pill/tag. 10px radius for tags, per the Resend tag shape. */
export function Badge({ className, ...props }: React.HTMLAttributes<HTMLSpanElement>) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-[10px] border border-graphite-rail px-2 py-0.5 text-[12px] leading-none text-fog transition-colors",
        className,
      )}
      {...props}
    />
  );
}
