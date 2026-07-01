import * as React from "react";
import { cn } from "@/lib/utils";

export const Input = React.forwardRef<HTMLInputElement, React.InputHTMLAttributes<HTMLInputElement>>(
  ({ className, ...props }, ref) => (
    <input
      ref={ref}
      className={cn(
        "no-drag h-10 w-full rounded-[10px] border border-graphite-rail bg-transparent px-3 text-[14px] text-frost placeholder:text-steel outline-none transition-colors focus:border-electric-blue/70 focus:ring-1 focus:ring-electric-blue/40",
        className,
      )}
      {...props}
    />
  ),
);
Input.displayName = "Input";
