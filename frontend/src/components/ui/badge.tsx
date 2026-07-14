import * as React from "react";
import { cn } from "@/lib/utils";

type Tone = "default" | "primary" | "success" | "warning" | "danger" | "info" | "muted";

const tones: Record<Tone, string> = {
  default: "bg-muted text-foreground",
  primary: "bg-accent text-accent-foreground",
  success: "bg-success/12 text-success",
  warning: "bg-warning/15 text-warning",
  danger: "bg-danger/12 text-danger",
  info: "bg-info/12 text-info",
  muted: "bg-muted text-muted-foreground",
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
