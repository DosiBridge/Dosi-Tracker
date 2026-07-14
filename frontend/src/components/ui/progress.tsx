import { cn } from "@/lib/utils";

export function Progress({
  value,
  className,
  indicatorClassName,
  color,
}: {
  value: number;
  className?: string;
  indicatorClassName?: string;
  color?: string;
}) {
  return (
    <div className={cn("h-2 w-full overflow-hidden rounded-full bg-muted", className)}>
      <div
        className={cn("h-full rounded-full bg-primary transition-all duration-500", indicatorClassName)}
        style={{ width: `${Math.min(100, Math.max(0, value))}%`, background: color }}
      />
    </div>
  );
}
