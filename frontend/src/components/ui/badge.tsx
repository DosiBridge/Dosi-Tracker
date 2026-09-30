import * as React from "react";
import { cn } from "@/lib/utils";

type Tone = "default" | "primary" | "success" | "warning" | "danger" | "info" | "muted";

// Badge text is 12px, so it needs the full 4.5:1 contrast ratio. The vivid
// status hues fail that on their own tints (see the --*-on-tint note in
// globals.css); these variants are gated by e2e/a11y.spec.ts.
const tones: Record<Tone, string> = {
  default: "bg-muted text-foreground",
  primary: "bg-accent text-accent-foreground",
  success: "bg-success/12 text-success-on-tint",
  warning: "bg-warning/15 text-warning-on-tint",
  danger: "bg-danger/12 text-danger-on-tint",
  info: "bg-info/12 text-info-on-tint",
  muted: "bg-muted text-muted-on-tint",
};

export function Badge({
  tone = "default",
  className,
  ...props
}: React.HTMLAttributes<HTMLSpanElement> & { tone?: Tone }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-full px-2.5 py-0.5 text-xs font-medium",
        tones[tone],
        className
      )}
      {...props}
    />
  );
}
