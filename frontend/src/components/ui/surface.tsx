import { cn } from "@/lib/utils";

type SurfaceVariant = "default" | "quiet" | "interactive" | "inset";

const variants: Record<SurfaceVariant, string> = {
  default: "rounded-2xl border border-border bg-card text-card-foreground card-elev",
  quiet:
    "rounded-2xl border border-border/80 bg-[color-mix(in_srgb,var(--muted)_40%,var(--card))] text-card-foreground shadow-none",
  interactive:
    "rounded-2xl border border-border bg-card text-card-foreground card-elev transition-[border-color,background-color,box-shadow] duration-200 hover:border-primary/30 hover:bg-accent/20",
  inset:
    "rounded-xl border border-border/70 bg-muted/40 text-card-foreground shadow-none",
};

/**
 * Unified surface language for cards, list items, and panels.
 * Prefer this (or Card with matching variant) over one-off hover styles.
 */
export function Surface({
  variant = "default",
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement> & { variant?: SurfaceVariant }) {
  return <div className={cn(variants[variant], className)} {...props} />;
}

export { variants as surfaceVariants };
export type { SurfaceVariant };
