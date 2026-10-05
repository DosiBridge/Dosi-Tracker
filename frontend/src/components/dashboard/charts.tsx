"use client";

import { ChartLoading } from "@/components/charts/chart-loading";
import type { Recharts } from "@/components/charts/recharts-kit";
import { useRecharts } from "@/components/charts/use-recharts";
import { useIsMdUp, useMediaQuery } from "@/hooks/use-media-query";
import { cn, formatDuration } from "@/lib/utils";

/** Axis labels: the muted text colour (≥ 4.5:1 on cards in both themes). */
const axisColor = "var(--muted-foreground)";

/**
 * The honest "nothing to chart" placeholder. A live workspace with no rows must
 * say so, not draw an empty axis (or, worse, demo bars).
 */
export function ChartEmpty({ message, className }: { message: string; className?: string }) {
  return (
    <div
      className={cn(
        "flex h-[200px] w-full items-center justify-center rounded-xl border border-dashed border-border px-4 text-center text-sm text-muted-foreground",
        className,
      )}
    >
      {message}
    </div>
  );
}

const tooltipStyle = {
  borderRadius: 12,
  border: "1px solid var(--border)",
  background: "var(--card)",
  color: "var(--card-foreground)",
  fontSize: 12,
  boxShadow: "var(--elev-lg)",
} as const;

/** "2h", "45m": an axis tick for a minutes scale. */
function minutesTick(value: unknown): string {
  const n = Number(value);
  if (!Number.isFinite(n) || n <= 0) return "0";
  return n >= 60 ? `${Math.round((n / 60) * 10) / 10}h` : `${Math.round(n)}m`;
}

/** Round steps for a minutes axis: whole minutes under an hour, whole (and round) hours above. */
const MINUTE_STEPS = [5, 10, 15, 30, 60, 120, 180, 240, 300, 600, 1200, 1800, 3000, 6000, 12000, 30000, 60000];

/**
 * At most five round ticks from 0 that cover `maxMinutes` ("0, 10h, 20h, 30h,
 * 40h" — never "8.3h, 16.7h"), and the axis top they end at.
 */
export function minuteAxis(maxMinutes: number): { ticks: number[]; top: number } {
  const max = Number.isFinite(maxMinutes) ? Math.max(0, maxMinutes) : 0;
  const step = MINUTE_STEPS.find((s) => Math.ceil(max / s) <= 4) ?? Math.ceil(max / 4 / 60000) * 60000;
  const count = Math.max(1, Math.ceil(max / step));
  const ticks = Array.from({ length: count + 1 }, (_, i) => i * step);
  return { ticks, top: ticks[ticks.length - 1] };
}

/**
 * Charts animate only once the browser has said motion is welcome (false on
 * the server and until then), so reduced-motion users never see a chart draw.
 */
function useChartMotion(): boolean {
  return useMediaQuery("(prefers-reduced-motion: no-preference)");
}

function durationText(value: unknown): string {
  const n = Number(value);
  return Number.isFinite(n) ? formatDuration(Math.max(0, Math.round(n))) : "—";
}

/**
 * A chart's labelled box. `children` draws the chart with recharts' components
 * once they have loaded (use-recharts.ts); until then the box holds a
 * placeholder of the same size, under the same accessible name.
 */
function ChartFrame({
  children,
  className,
  label,
}: {
  children: (recharts: Recharts) => React.ReactElement;
  className?: string;
  label?: string;
}) {
  const recharts = useRecharts();
  return (
    <div className={cn("w-full min-w-0", className)} role={label ? "img" : undefined} aria-label={label}>
      {recharts ? (
        <recharts.ResponsiveContainer width="100%" height="100%">
          {children(recharts)}
        </recharts.ResponsiveContainer>
      ) : (
        <ChartLoading />
      )}
    </div>
  );
}

/* ----------------------------- Legend ----------------------------- */

export interface LegendItem {
  key: string;
  label: string;
  color: string;
  /** A value after the label (e.g. "4h 30m"). */
  value?: string;
}

/** Swatch + label rows, fed from the same config as the series they describe. */
export function ChartLegend({ items, className }: { items: readonly LegendItem[]; className?: string }) {
  return (
    <ul className={cn("flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground", className)}>
      {items.map((item) => (
        <li key={item.key} className="flex items-center gap-1.5">
          <span aria-hidden className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ background: item.color }} />
          <span>{item.label}</span>
          {item.value && <span className="font-medium text-foreground">{item.value}</span>}
        </li>
      ))}
    </ul>
  );
}

/* ----------------------------- Daily trend ----------------------------- */

export interface TrendPoint {
  day: string;
  /** Minutes tracked. */
  tracked: number;
  /** Minutes with keyboard or mouse input (tracked × activity %). */
  productive: number;
}

/** The trend's two series; the legend reads the same entries. */
export const TREND_SERIES = [
  { key: "tracked", label: "Tracked", color: "#0d9488" },
  { key: "productive", label: "Active (keyboard & mouse)", color: "#16a34a" },
] as const;

export function ActivityTrendChart({ data }: { data: TrendPoint[] }) {
  const md = useIsMdUp();
  const motion = useChartMotion();
  const total = data.reduce((s, d) => s + d.tracked, 0);
  const axis = minuteAxis(Math.max(0, ...data.map((d) => Math.max(d.tracked, d.productive))));
  return (
    <ChartFrame
      className="h-[200px] sm:h-[240px] lg:h-[260px]"
      label={`Tracked time per day: ${data.map((d) => `${d.day} ${durationText(d.tracked)}`).join(", ")}. ${durationText(total)} in all.`}
    >
      {({ AreaChart, Area, XAxis, YAxis, Tooltip }) => (
        <AreaChart data={data} margin={{ top: 10, right: 8, left: 0, bottom: 0 }}>
          <defs>
            {TREND_SERIES.map((s) => (
              <linearGradient key={s.key} id={`trend-${s.key}`} x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor={s.color} stopOpacity={0.35} />
                <stop offset="100%" stopColor={s.color} stopOpacity={0} />
              </linearGradient>
            ))}
          </defs>
          <XAxis dataKey="day" tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: md ? 12 : 10 }} />
          <YAxis
            tickLine={false}
            axisLine={false}
            tick={{ fill: axisColor, fontSize: md ? 12 : 10 }}
            tickFormatter={minutesTick}
            width={md ? 44 : 36}
            domain={[0, axis.top]}
            ticks={axis.ticks}
          />
          <Tooltip contentStyle={tooltipStyle} formatter={(v, name) => [durationText(v), name]} />
          {TREND_SERIES.map((s) => (
            <Area
              key={s.key}
              type="monotone"
              dataKey={s.key}
              stroke={s.color}
              strokeWidth={2.5}
              fill={`url(#trend-${s.key})`}
              name={s.label}
              isAnimationActive={motion}
            />
          ))}
        </AreaChart>
      )}
    </ChartFrame>
  );
}

/* ----------------------------- Donut ----------------------------- */

export interface DonutSlice {
  name: string;
  value: number;
  color: string;
}

export function ProjectDonut({
  data,
  format = durationText,
  centerLabel = "tracked",
}: {
  data: DonutSlice[];
  /** How a slice's value reads (default: minutes as "4h 30m"). */
  format?: (value: number) => string;
  /** The word under the total in the middle. */
  centerLabel?: string;
}) {
  const md = useIsMdUp();
  const motion = useChartMotion();
  const total = data.reduce((s, d) => s + d.value, 0);
  const inner = md ? 62 : 48;
  const outer = md ? 92 : 70;
  const pct = (v: number) => (total > 0 ? Math.round((v / total) * 100) : 0);
  return (
    <div className="relative mx-auto h-[180px] max-w-[280px] sm:h-[220px] sm:max-w-none">
      <ChartFrame
        className="h-full"
        label={`${format(total)} ${centerLabel}: ${data.map((d) => `${d.name} ${format(d.value)} (${pct(d.value)}%)`).join(", ")}`}
      >
        {({ PieChart, Pie, Cell, Tooltip }) => (
          <PieChart>
            <Pie data={data} dataKey="value" nameKey="name" innerRadius={inner} outerRadius={outer} paddingAngle={data.length > 1 ? 2 : 0} strokeWidth={0} isAnimationActive={motion}>
              {data.map((d) => (
                <Cell key={d.name} fill={d.color} />
              ))}
            </Pie>
            <Tooltip contentStyle={tooltipStyle} formatter={(v, name) => [`${format(Number(v))} · ${pct(Number(v))}%`, name]} />
          </PieChart>
        )}
      </ChartFrame>
      <div aria-hidden className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center">
        <span className="font-display text-lg font-bold tabular-nums text-ink">{format(total)}</span>
        <span className="text-[11px] text-muted-foreground">{centerLabel}</span>
      </div>
    </div>
  );
}

/**
 * Every slice of a donut with its value and share — the first `max`, then a
 * "+N more" line with the rest's total, so the legend always adds up.
 */
export function DonutLegend({
  data,
  max = 6,
  format = durationText,
}: {
  data: DonutSlice[];
  max?: number;
  format?: (value: number) => string;
}) {
  const total = data.reduce((s, d) => s + d.value, 0);
  const shown = data.length > max ? data.slice(0, max - 1) : data;
  const rest = data.slice(shown.length);
  const restValue = rest.reduce((s, d) => s + d.value, 0);
  const pct = (v: number) => (total > 0 ? Math.round((v / total) * 100) : 0);
  return (
    <ul className="mt-3 space-y-2">
      {shown.map((d) => (
        <li key={d.name} className="flex items-center justify-between gap-3 text-sm">
          <span className="flex min-w-0 items-center gap-2">
            <span aria-hidden className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ background: d.color }} />
            <span className="truncate text-muted-foreground" title={d.name}>
              {d.name}
            </span>
          </span>
          <span className="shrink-0 tabular-nums">
            <span className="font-medium">{format(d.value)}</span>
            <span className="ml-1.5 text-xs text-muted-foreground">{pct(d.value)}%</span>
          </span>
        </li>
      ))}
      {rest.length > 0 && (
        <li className="flex items-center justify-between gap-3 text-sm">
          <span className="flex items-center gap-2 text-muted-foreground">
            <span aria-hidden className="h-2.5 w-2.5 shrink-0 rounded-full bg-muted-foreground/40" />+{rest.length} more
          </span>
          <span className="shrink-0 tabular-nums">
            <span className="font-medium">{format(restValue)}</span>
            <span className="ml-1.5 text-xs text-muted-foreground">{pct(restValue)}%</span>
          </span>
        </li>
      )}
    </ul>
  );
}

/* ----------------------------- Apps ----------------------------- */

function shortLabel(name: string, max = 12) {
  return name.length > max ? `${name.slice(0, max - 1)}…` : name;
}

export function TopAppsChart({ data }: { data: { app: string; minutes: number; color: string }[] }) {
  const md = useIsMdUp();
  const motion = useChartMotion();
  return (
    <ChartFrame
      className="h-[200px] sm:h-[240px]"
      label={`Time per app: ${data.map((d) => `${d.app} ${durationText(d.minutes)}`).join(", ")}`}
    >
      {({ BarChart, Bar, Cell, LabelList, XAxis, YAxis, Tooltip }) => (
        <BarChart data={data} layout="vertical" margin={{ top: 0, right: md ? 56 : 48, left: 4, bottom: 0 }}>
          <XAxis type="number" hide />
          <YAxis
            type="category"
            dataKey="app"
            tickLine={false}
            axisLine={false}
            // Room for the common names on one line ("Visual Studio Code", "Google Chrome"); longer ones end in "…".
            width={md ? 144 : 100}
            tick={{ fill: axisColor, fontSize: md ? 12 : 10 }}
            tickFormatter={(v) => shortLabel(String(v), md ? 20 : 15)}
          />
          <Tooltip
            contentStyle={tooltipStyle}
            cursor={{ fill: "var(--muted)" }}
            formatter={(v) => [durationText(v), "Time"]}
            labelFormatter={(label) => String(label)}
          />
          <Bar dataKey="minutes" radius={[0, 6, 6, 0]} barSize={md ? 16 : 12} isAnimationActive={motion}>
            {data.map((d) => (
              <Cell key={d.app} fill={d.color} />
            ))}
            <LabelList dataKey="minutes" position="right" formatter={(v: unknown) => durationText(v)} fill="currentColor" fontSize={11} className="fill-muted-foreground" />
          </Bar>
        </BarChart>
      )}
    </ChartFrame>
  );
}

/* ----------------------------- Hours of the day ----------------------------- */

export function HourlyChart({
  data,
  label = "Time tracked per hour",
}: {
  data: { hour: string; minutes: number }[];
  /** What the bars show, for the chart's accessible name (the busiest hour is added). */
  label?: string;
}) {
  const md = useIsMdUp();
  const motion = useChartMotion();
  const busiest = data.reduce<{ hour: string; minutes: number } | null>((best, d) => (!best || d.minutes > best.minutes ? d : best), null);
  const axis = minuteAxis(busiest?.minutes ?? 0);
  return (
    <ChartFrame
      className="h-[160px] sm:h-[180px]"
      label={`${label}${busiest && busiest.minutes > 0 ? `; busiest hour ${busiest.hour} with ${durationText(busiest.minutes)}` : ""}.`}
    >
      {({ BarChart, Bar, XAxis, YAxis, Tooltip }) => (
        <BarChart data={data} margin={{ top: 4, right: 4, left: 0, bottom: 0 }}>
          <XAxis dataKey="hour" tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: 10 }} interval={md ? 1 : 2} />
          <YAxis
            tickLine={false}
            axisLine={false}
            tick={{ fill: axisColor, fontSize: 10 }}
            tickFormatter={minutesTick}
            width={md ? 36 : 30}
            domain={[0, axis.top]}
            ticks={axis.ticks}
          />
          <Tooltip contentStyle={tooltipStyle} cursor={{ fill: "var(--muted)" }} formatter={(v) => [durationText(v), "Tracked"]} />
          <Bar dataKey="minutes" radius={[4, 4, 0, 0]} fill="#0d9488" barSize={md ? 14 : 10} isAnimationActive={motion} />
        </BarChart>
      )}
    </ChartFrame>
  );
}
