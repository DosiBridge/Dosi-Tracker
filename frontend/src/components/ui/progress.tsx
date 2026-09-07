import { cn } from "@/lib/utils";

export function Progress({
  value,
  className,
  indicatorClassName,
  color,
  "aria-label": ariaLabel = "Progress",
}: {
  value: number;
  className?: string;
  indicatorClassName?: string;
  color?: string;
  "aria-label"?: string;
}) {
  const clamped = Math.min(100, Math.max(0, value));
  return (
    <div
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={clamped}
      aria-label={ariaLabel}
      className={cn("h-2 w-full overflow-hidden rounded-full bg-muted", className)}
    >
      <div
        className={cn("h-full rounded-full bg-primary transition-all duration-500", indicatorClassName)}
        style={{ width: `${clamped}%`, background: color }}
      />
    </div>
  );
}
