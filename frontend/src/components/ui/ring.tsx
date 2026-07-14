import { scoreTone } from "@/lib/utils";

const toneColor = {
  success: "#22c55e",
  warning: "#f59e0b",
  danger: "#ef4444",
};

export function Ring({
  value,
  size = 44,
  stroke = 5,
  showLabel = true,
  color,
}: {
  value: number;
  size?: number;
  stroke?: number;
  showLabel?: boolean;
  color?: string;
}) {
  const r = (size - stroke) / 2;
  const c = 2 * Math.PI * r;
  const offset = c - (value / 100) * c;
  const col = color ?? toneColor[scoreTone(value)];

  return (
    <div className="relative inline-flex items-center justify-center" style={{ width: size, height: size }}>
      <svg width={size} height={size} className="-rotate-90">
        <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="var(--muted)" strokeWidth={stroke} />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          stroke={col}
          strokeWidth={stroke}
          strokeDasharray={c}
          strokeDashoffset={offset}
          strokeLinecap="round"
          style={{ transition: "stroke-dashoffset 0.6s ease" }}
        />
      </svg>
      {showLabel && (
        <span className="absolute text-[11px] font-semibold" style={{ color: col }}>
          {value}
        </span>
      )}
    </div>
  );
}
