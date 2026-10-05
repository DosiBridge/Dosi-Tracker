import { Ring } from "@/components/ui/ring";
import { cn } from "@/lib/utils";

/**
 * An activity % ring that says what it is: the percentage is drawn with its
 * "%" sign and the ring is announced as "Activity 78%". Activity % is the
 * share of tracked minutes with keyboard or mouse input (contract §1); `null`
 * means nothing was measured (input capture off or unavailable) and reads "—".
 */
export function ActivityRing({
  value,
  size = 44,
  stroke = 5,
  label = "Activity",
  caption,
  className,
}: {
  value: number | null;
  size?: number;
  stroke?: number;
  /** What the percentage measures, for the accessible name. */
  label?: string;
  /** A visible line under the ring (e.g. "activity"). */
  caption?: string;
  className?: string;
}) {
  const measured = typeof value === "number" && Number.isFinite(value);
  const pct = measured ? Math.max(0, Math.min(100, Math.round(value))) : 0;
  const name = measured ? `${label} ${pct}%` : `${label} not measured`;
  return (
    <div className={cn("inline-flex shrink-0 flex-col items-center", className)}>
      <div role="img" aria-label={name} title={name} className="relative" style={{ width: size, height: size }}>
        <Ring value={pct} size={size} stroke={stroke} showLabel={false} color={measured ? undefined : "var(--border)"} />
        <span
          aria-hidden
          className={cn(
            "absolute inset-0 flex items-center justify-center font-semibold tabular-nums",
            measured ? "text-foreground" : "text-muted-foreground",
          )}
          style={{ fontSize: Math.max(10, Math.round(size * 0.24)) }}
        >
          {measured ? `${pct}%` : "—"}
        </span>
      </div>
      {caption && <span className="mt-1 text-[11px] text-muted-foreground">{caption}</span>}
    </div>
  );
}
