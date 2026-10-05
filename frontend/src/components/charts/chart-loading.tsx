import { cn } from "@/lib/utils";

/**
 * A chart's box while its drawing code downloads (see use-recharts.ts): the
 * same size as the chart, quietly pulsing — still when motion is unwelcome.
 * Hidden from assistive technology: the chart's own label stays in place.
 */
export function ChartLoading({ className }: { className?: string }) {
  return <div aria-hidden className={cn("h-full w-full rounded-xl bg-muted/50 motion-safe:animate-pulse", className)} />;
}
