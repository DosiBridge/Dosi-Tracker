import { cn } from "@/lib/utils";

/*
 * Button looks for links. A link must not wrap a <button> (nested interactive
 * controls: one Tab stop announced as two things), so navigation that should
 * look like a button is a styled <Link> using these classes — they match the
 * Button component's sizes and variants.
 */

const base = cn(
  "inline-flex items-center justify-center rounded-xl font-medium transition-all",
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
);

const variants = {
  primary: "bg-primary text-primary-foreground hover:brightness-[1.04] active:brightness-95",
  outline: "border border-border bg-transparent hover:bg-muted/60",
} as const;

const sizes = {
  sm: "h-8 gap-1.5 px-3 text-xs",
  md: "h-10 gap-2 px-4 text-sm",
} as const;

/** Classes for a link that looks like a Button of the given variant and size. */
export function linkButtonClass(variant: keyof typeof variants = "outline", size: keyof typeof sizes = "sm"): string {
  return cn(base, variants[variant], sizes[size]);
}
