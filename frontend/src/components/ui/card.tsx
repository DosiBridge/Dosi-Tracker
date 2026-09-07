import * as React from "react";
import { cn } from "@/lib/utils";
import { surfaceVariants, type SurfaceVariant } from "@/components/ui/surface";

export function Card({
  className,
  variant = "default",
  ...props
}: React.HTMLAttributes<HTMLDivElement> & { variant?: SurfaceVariant }) {
  return (
    <div
      className={cn(surfaceVariants[variant], className)}
      {...props}
    />
  );
}

export function CardHeader({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("flex flex-wrap items-center justify-between gap-3 p-5 pb-0", className)}
      {...props}
    />
  );
}

// h2, not h3: cards are the top-level sections beneath a page's single h1, so
// an h3 here skips a level and trips axe's heading-order rule. Visual size is
// carried by the classes, not the tag.
export function CardTitle({ className, ...props }: React.HTMLAttributes<HTMLHeadingElement>) {
  return (
    <h2
      className={cn("font-display text-sm font-semibold tracking-tight", className)}
      {...props}
    />
  );
}

export function CardContent({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("p-5", className)} {...props} />;
}
