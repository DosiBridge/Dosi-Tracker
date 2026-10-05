"use client";

import { Fragment, useId, useState } from "react";
import type { LucideIcon } from "lucide-react";
import { ArrowDownRight, ArrowRight, ArrowUpRight, FlaskConical } from "lucide-react";
import { ChartLoading } from "@/components/charts/chart-loading";
import type { Recharts } from "@/components/charts/recharts-kit";
import { useRecharts } from "@/components/charts/use-recharts";
import { useCountUp, usePrefersReducedMotion } from "@/hooks/use-count-up";
import { useIsMdUp } from "@/hooks/use-media-query";
import { useElementWidth } from "@/hooks/use-element-width";
import {
  WEEKDAYS,
  clockLabel,
  gapKind,
  hourRangeLabel,
  layoutFlow,
  peakHour,
  placeLabels,
  squarify,
  type AppStat,
  type DayTimeline,
  type FlowData,
  type GapKind,
  type HourStat,
  type PlatformShare,
  type WorkWindow,
} from "@/lib/analytics";
import { formatActivity } from "@/lib/work-report";
import { cn } from "@/lib/utils";

/* ============================================================================
 * Charts, diagrams and shared widgets of Work Analytics and the Work Report.
 * Each takes ready-made figures (lib/analytics.ts and lib/work-report.ts do
 * the maths) and a formatter for durations.
 * Motion: recharts animates its own marks; the custom SVG/CSS diagrams use
 * the .chart-* classes in globals.css, which only move when the viewer has
 * not asked for reduced motion — the resting state is always the full chart.
 * Axis text uses the muted text colour (≥ 4.5:1 on cards in both themes).
 * ========================================================================== */

export type Fmt = (minutes: number) => string;

/** Member colours (projects keep their own colours, apps theirs from lib/apps). */
export const SERIES_COLORS = ["#006bff", "#00a389", "#7c3aed", "#db2777", "#ea580c", "#0891b2", "#65a30d", "#ca8a04"];
export const OTHER_COLOR = "#94a3b8";

const axis = { fill: "var(--muted-foreground)", fontSize: 11 };
const grid = { stroke: "var(--border)", strokeDasharray: "3 4" };

function gradientId(raw: string) {
  return raw.replace(/[^a-zA-Z0-9_-]/g, "");
}

/**
 * A recharts chart's box: `children` draws the chart with recharts'
 * components once they have loaded (components/charts/use-recharts.ts); until
 * then the box holds a placeholder of the same size.
 */
function ChartBox({
  className,
  style,
  children,
}: {
  className: string;
  style?: React.CSSProperties;
  children: (recharts: Recharts) => React.ReactElement;
}) {
  const recharts = useRecharts();
  return (
    <div className={className} style={style}>
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

/* ----------------------------- Building blocks ----------------------------- */

/** A number that counts up on first show; the final value is what assistive tech reads. */
export function AnimatedNumber({ value, format }: { value: number; format: (v: number) => string }) {
  const shown = useCountUp(value);
  return (
    <>
      <span aria-hidden className="tabular-nums">
        {format(shown)}
      </span>
      <span className="sr-only">{format(value)}</span>
    </>
  );
}

/** A tiny trend line that draws itself in. */
export function Sparkline({ values, color = "var(--primary)", className }: { values: number[]; color?: string; className?: string }) {
  const id = gradientId(useId());
  const w = 120;
  const h = 36;
  if (values.length < 2 || values.every((v) => v === 0)) return <div aria-hidden className={cn("h-9", className)} />;
  const max = Math.max(...values, 1e-9);
  const points = values.map((v, i) => [(i / (values.length - 1)) * w, h - 3 - (Math.max(0, v) / max) * (h - 8)]);
  const line = points.map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`).join(" ");
  return (
    <svg viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" aria-hidden className={cn("h-9 w-full overflow-visible", className)}>
      <defs>
        <linearGradient id={`${id}-fill`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" style={{ stopColor: color, stopOpacity: 0.28 }} />
          <stop offset="100%" style={{ stopColor: color, stopOpacity: 0 }} />
        </linearGradient>
        <clipPath id={`${id}-clip`}>
          <rect x={-2} y={-4} width={w + 4} height={h + 8} className="chart-reveal" />
        </clipPath>
      </defs>
      <g clipPath={`url(#${id}-clip)`}>
        <path d={`${line} L${w},${h} L0,${h} Z`} fill={`url(#${id}-fill)`} />
        <path d={line} fill="none" stroke={color} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
      </g>
    </svg>
  );
}

export interface Delta {
  percent: number;
  /** Shown after the percentage, e.g. "vs the week before". */
  label: string;
}

/** ▲ 12% / ▼ 8% / → 0% with a quiet comparison label. */
export function DeltaPill({ delta }: { delta: Delta }) {
  const up = delta.percent > 0;
  const down = delta.percent < 0;
  const Icon = up ? ArrowUpRight : down ? ArrowDownRight : ArrowRight;
  return (
    <span
      title={delta.label}
      className={cn(
        "print-exact inline-flex items-center gap-0.5 rounded-full px-2 py-0.5 text-[11px] font-semibold",
        up ? "bg-success/12 text-success-on-tint" : down ? "bg-danger/12 text-danger-on-tint" : "bg-muted text-muted-on-tint",
      )}
    >
      <Icon aria-hidden className="h-3 w-3" />
      {up ? "+" : ""}
      {delta.percent}%<span className="sr-only"> {delta.label}</span>
    </span>
  );
}

/** "Sample": the chart is drawn from the newest blocks of a period that has more than were loaded. */
export function SampleBadge({ detail }: { detail: string }) {
  return (
    <span
      title={detail}
      className="print-exact inline-flex items-center gap-1 rounded-full bg-warning/15 px-2 py-0.5 text-[11px] font-semibold text-warning-on-tint"
    >
      <FlaskConical aria-hidden className="h-3 w-3" />
      Sample
      <span className="sr-only">: {detail}</span>
    </span>
  );
}

export const accentClass = {
  blue: "bg-primary/10 text-primary",
  green: "bg-success/12 text-success-on-tint",
  violet: "bg-violet-500/12 text-violet-700 dark:text-violet-300",
  amber: "bg-warning/15 text-warning-on-tint",
  pink: "bg-pink-500/12 text-pink-700 dark:text-pink-300",
  cyan: "bg-cyan-500/12 text-cyan-700 dark:text-cyan-300",
} as const;

export const accentColor: Record<keyof typeof accentClass, string> = {
  blue: "var(--primary)",
  green: "var(--success)",
  violet: "#7c3aed",
  amber: "var(--warning)",
  pink: "#db2777",
  cyan: "#0891b2",
};

/** A headline figure: icon, label, counting number, optional change, note and sparkline. */
export function KpiCard({
  label,
  icon: Icon,
  accent,
  value,
  format,
  sub,
  spark,
  delta,
  badge,
}: {
  label: string;
  icon: LucideIcon;
  accent: keyof typeof accentClass;
  value: number;
  format: (v: number) => string;
  sub?: string;
  spark?: number[];
  delta?: Delta | null;
  /** A mark after the value (e.g. the sample badge). */
  badge?: React.ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col rounded-2xl border border-border bg-card p-4 card-elev">
      <dt className="flex items-center justify-between gap-2 text-xs font-medium text-muted-foreground">
        <span className="flex min-w-0 items-center gap-2">
          <span aria-hidden className={cn("print-exact flex h-7 w-7 shrink-0 items-center justify-center rounded-lg", accentClass[accent])}>
            <Icon className="h-3.5 w-3.5" />
          </span>
          <span className="truncate" title={label}>
            {label}
          </span>
        </span>
        {delta && <DeltaPill delta={delta} />}
      </dt>
      <dd className="mt-3 flex flex-wrap items-center gap-2 font-display text-2xl font-bold tracking-tight text-ink sm:text-[1.65rem]">
        <span>
          <AnimatedNumber value={value} format={format} />
        </span>
        {badge}
      </dd>
      {sub && <dd className="mt-0.5 text-xs text-muted-foreground">{sub}</dd>}
      {spark && (
        <dd className="mt-2">
          <Sparkline values={spark} color={accentColor[accent]} />
        </dd>
      )}
    </div>
  );
}

export function LegendItem({ color, label, line = false, dashed = false }: { color: string; label: string; line?: boolean; dashed?: boolean }) {
  return (
    <span className="flex items-center gap-1.5">
      {line ? (
        <span className="print-exact w-3.5 border-t-2" style={{ borderColor: color, borderStyle: dashed ? "dashed" : "solid" }} />
      ) : (
        <span className="print-exact h-2.5 w-2.5 rounded-sm" style={{ background: color }} />
      )}
      {label}
    </span>
  );
}

export function ChartLegend({ children }: { children: React.ReactNode }) {
  return <div aria-hidden className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">{children}</div>;
}

function TooltipBox({ title, rows }: { title: string; rows: [string, string, string?][] }) {
  return (
    <div className="min-w-[170px] rounded-xl border border-border bg-card px-3 py-2 text-xs text-card-foreground card-elev-lg">
      <div className="font-semibold">{title}</div>
      {rows.map(([label, value, color]) => (
        <div key={label} className="mt-1 flex items-center justify-between gap-4">
          <span className="flex items-center gap-1.5 text-muted-foreground">
            {color && <span className="h-2 w-2 rounded-full" style={{ background: color }} />}
            {label}
          </span>
          <span className="font-semibold tabular-nums">{value}</span>
        </div>
      ))}
    </div>
  );
}

type TipProps = { active?: boolean; payload?: ReadonlyArray<{ payload?: unknown }> };
const first = <T,>(p: TipProps): T | undefined => (p.active && p.payload?.length ? (p.payload[0]?.payload as T) : undefined);

/* ----------------------------- Overview ----------------------------- */

export interface TrendRow {
  label: string;
  longLabel: string;
  hours: number;
  minutes: number;
  /** The trailing 7-day average, hours (seeded with the days before the period); null when not drawn. */
  average: number | null;
  /** null on days without time or without a measurement (a gap, not a dip to 0%). */
  activity: number | null;
}

/** Hours per day (area), the 7-day moving average, and activity % on a second axis. */
export function TrendChart({ data, fmt, showAverage }: { data: TrendRow[]; fmt: Fmt; showAverage: boolean }) {
  const reduced = usePrefersReducedMotion();
  const id = gradientId(useId());
  const md = useIsMdUp();
  return (
    <ChartBox className="analytics-chart h-[260px] w-full min-w-0 sm:h-[300px]">
      {({ Area, CartesianGrid, ComposedChart, Line, Tooltip, XAxis, YAxis }) => (
        <ComposedChart data={data} margin={{ top: 10, right: md ? 4 : 2, left: md ? -6 : -2, bottom: 0 }}>
          <defs>
            <linearGradient id={`${id}-area`} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" style={{ stopColor: "var(--primary)", stopOpacity: 0.35 }} />
              <stop offset="100%" style={{ stopColor: "var(--primary)", stopOpacity: 0.02 }} />
            </linearGradient>
          </defs>
          <CartesianGrid vertical={false} {...grid} />
          <XAxis dataKey="label" tickLine={false} axisLine={false} tick={axis} interval="preserveStartEnd" minTickGap={14} />
          <YAxis yAxisId="h" tickLine={false} axisLine={false} tick={axis} width={40} tickFormatter={(v) => `${v}h`} allowDecimals={false} />
          <YAxis yAxisId="a" orientation="right" tickLine={false} axisLine={false} tick={axis} width={md ? 38 : 36} domain={[0, 100]} tickFormatter={(v) => `${v}%`} />
          <Tooltip
            cursor={{ stroke: "var(--border)", strokeWidth: 1 }}
            content={(p) => {
              const row = first<TrendRow>(p as TipProps);
              if (!row) return null;
              const rows: [string, string, string?][] = [["Hours", fmt(row.minutes), "var(--primary)"]];
              if (showAverage && row.average !== null) rows.push(["7-day average", fmt(row.average * 60), "#7c3aed"]);
              rows.push(["Activity", formatActivity(row.activity), "var(--success)"]);
              return <TooltipBox title={row.longLabel} rows={rows} />;
            }}
          />
          <Area
            yAxisId="h"
            type="monotone"
            dataKey="hours"
            stroke="var(--primary)"
            strokeWidth={2.5}
            fill={`url(#${id}-area)`}
            isAnimationActive={!reduced} animationDuration={1100}
            animationEasing="ease-out"
            activeDot={{ r: 5, strokeWidth: 2, stroke: "var(--card)" }}
          />
          {showAverage && (
            <Line yAxisId="h" type="monotone" dataKey="average" stroke="#7c3aed" strokeWidth={2} strokeDasharray="6 4" dot={false} isAnimationActive={!reduced} animationDuration={1400} />
          )}
          <Line
            yAxisId="a"
            type="monotone"
            dataKey="activity"
            stroke="var(--success)"
            strokeWidth={2}
            dot={{ r: 2.5, strokeWidth: 0, fill: "var(--success)" }}
            connectNulls={false}
            isAnimationActive={!reduced} animationDuration={1700}
          />
        </ComposedChart>
      )}
    </ChartBox>
  );
}

export interface PaceRow {
  label: string;
  longLabel: string;
  actual: number;
  target: number;
}

/** Running total of hours against the pace of a full day on every weekday. */
export function PaceChart({ data, fmt }: { data: PaceRow[]; fmt: Fmt }) {
  const reduced = usePrefersReducedMotion();
  const id = gradientId(useId());
  return (
    <ChartBox className="analytics-chart h-[220px] w-full min-w-0 sm:h-[260px]">
      {({ Area, AreaChart, CartesianGrid, Tooltip, XAxis, YAxis }) => (
        <AreaChart data={data} margin={{ top: 10, right: 6, left: -6, bottom: 0 }}>
          <defs>
            <linearGradient id={`${id}-pace`} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" style={{ stopColor: "var(--success)", stopOpacity: 0.32 }} />
              <stop offset="100%" style={{ stopColor: "var(--success)", stopOpacity: 0.02 }} />
            </linearGradient>
          </defs>
          <CartesianGrid vertical={false} {...grid} />
          <XAxis dataKey="label" tickLine={false} axisLine={false} tick={axis} interval="preserveStartEnd" minTickGap={14} />
          <YAxis tickLine={false} axisLine={false} tick={axis} width={44} tickFormatter={(v) => `${v}h`} allowDecimals={false} />
          <Tooltip
            content={(p) => {
              const row = first<PaceRow>(p as TipProps);
              return row ? (
                <TooltipBox
                  title={row.longLabel}
                  rows={[
                    ["Tracked so far", fmt(row.actual * 60), "var(--success)"],
                    ["Pace", fmt(row.target * 60), "var(--muted-foreground)"],
                  ]}
                />
              ) : null;
            }}
          />
          <Area type="monotone" dataKey="target" stroke="var(--muted-foreground)" strokeWidth={1.5} strokeDasharray="5 5" fill="none" isAnimationActive={!reduced} animationDuration={900} />
          <Area type="monotone" dataKey="actual" stroke="var(--success)" strokeWidth={2.5} fill={`url(#${id}-pace)`} isAnimationActive={!reduced} animationDuration={1300} />
        </AreaChart>
      )}
    </ChartBox>
  );
}

export interface IdleRow {
  label: string;
  longLabel: string;
  minutes: number;
  idle: number;
  /** Idle share of the day's tracked time, whole percent. */
  share: number;
}

/** Idle minutes per day: tracked time with no keyboard or mouse input. */
export function IdleChart({ data, fmt }: { data: IdleRow[]; fmt: Fmt }) {
  const reduced = usePrefersReducedMotion();
  const rows = data.map((d) => ({ ...d, hours: Math.round((d.idle / 60) * 100) / 100 }));
  return (
    <ChartBox className="analytics-chart h-[220px] w-full min-w-0">
      {({ Bar, BarChart, CartesianGrid, Tooltip, XAxis, YAxis }) => (
        <BarChart data={rows} margin={{ top: 10, right: 6, left: -6, bottom: 0 }}>
          <CartesianGrid vertical={false} {...grid} />
          <XAxis dataKey="label" tickLine={false} axisLine={false} tick={axis} interval="preserveStartEnd" minTickGap={14} />
          <YAxis tickLine={false} axisLine={false} tick={axis} width={44} tickFormatter={(v) => `${v}h`} />
          <Tooltip
            cursor={{ fill: "var(--muted)", radius: 6 }}
            content={(p) => {
              const row = first<(typeof rows)[number]>(p as TipProps);
              return row ? (
                <TooltipBox
                  title={row.longLabel}
                  rows={[
                    ["Idle", fmt(row.idle), "var(--warning)"],
                    ["Tracked", fmt(row.minutes)],
                    ["Idle share", row.minutes > 0 ? `${row.share}%` : "—"],
                  ]}
                />
              ) : null;
            }}
          />
          <Bar dataKey="hours" fill="var(--warning)" radius={[6, 6, 2, 2]} maxBarSize={36} isAnimationActive={!reduced} animationDuration={900} />
        </BarChart>
      )}
    </ChartBox>
  );
}

/* ----------------------------- Rhythm ----------------------------- */

const HOUR_TICKS = [0, 3, 6, 9, 12, 15, 18, 21];

/** Weekday × hour heat map of when work happens; cells cascade in. */
export function RhythmHeatmap({ grid: cells, fmt, summary }: { grid: number[][]; fmt: Fmt; summary: string }) {
  const max = Math.max(1e-9, ...cells.flat());
  const [hover, setHover] = useState<{ day: number; hour: number } | null>(null);
  const shade = (v: number) =>
    v > 0 ? `color-mix(in srgb, var(--primary) ${Math.round(18 + 82 * (v / max))}%, var(--card))` : "var(--muted)";
  return (
    <div>
      <div className="overflow-x-auto overscroll-x-contain pb-1">
        <div role="img" aria-label={summary} className="grid min-w-[620px] gap-[3px]" style={{ gridTemplateColumns: "2.75rem repeat(24, minmax(0, 1fr))" }}>
          <div />
          {Array.from({ length: 24 }, (_, h) => (
            <div key={h} className="pb-1 text-center text-[10px] text-muted-foreground">
              {HOUR_TICKS.includes(h) ? String(h).padStart(2, "0") : ""}
            </div>
          ))}
          {cells.map((row, day) => (
            <Fragment key={day}>
              <div className="flex items-center text-[11px] font-medium text-muted-foreground">{WEEKDAYS[day]}</div>
              {row.map((v, hour) => (
                <div
                  key={hour}
                  title={`${WEEKDAYS[day]} ${hourRangeLabel(hour)}: ${v > 0 ? fmt(v) : "no time"}`}
                  onMouseEnter={() => setHover({ day, hour })}
                  onMouseLeave={() => setHover(null)}
                  className={cn(
                    "print-exact chart-cell-in h-6 rounded-[5px] transition-[outline] duration-100 sm:h-7",
                    hover?.day === day && hover?.hour === hour && "outline outline-2 outline-offset-1 outline-primary",
                  )}
                  style={{ background: shade(v), animationDelay: `${hour * 14 + day * 40}ms` }}
                />
              ))}
            </Fragment>
          ))}
        </div>
      </div>
      <div className="mt-3 flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
        <span aria-hidden className="min-h-4 font-medium text-foreground">
          {hover ? `${WEEKDAYS[hover.day]} ${hourRangeLabel(hover.hour)} · ${fmt(cells[hover.day][hover.hour])}` : "Hover a cell for the hours"}
        </span>
        <span aria-hidden className="flex items-center gap-1">
          Less
          {[0, 0.25, 0.5, 0.75, 1].map((t) => (
            <span key={t} className="print-exact h-3 w-3 rounded-[3px]" style={{ background: t === 0 ? "var(--muted)" : `color-mix(in srgb, var(--primary) ${Math.round(18 + 82 * t)}%, var(--card))` }} />
          ))}
          More
        </span>
      </div>
    </div>
  );
}

function sector(cx: number, cy: number, r0: number, r1: number, a0: number, a1: number): string {
  const p = (r: number, a: number) => `${(cx + r * Math.cos(a)).toFixed(2)},${(cy + r * Math.sin(a)).toFixed(2)}`;
  const large = a1 - a0 > Math.PI ? 1 : 0;
  return `M${p(r0, a0)} L${p(r1, a0)} A${r1},${r1} 0 ${large} 1 ${p(r1, a1)} L${p(r0, a1)} A${r0},${r0} 0 ${large} 0 ${p(r0, a0)} Z`;
}

/** A 24-hour clock: one petal per hour, as long as the time worked in it. */
export function DayClock({ profile, fmt }: { profile: HourStat[]; fmt: Fmt }) {
  const size = 280;
  const c = size / 2;
  const r0 = 50;
  const r1 = 116;
  const max = Math.max(1e-9, ...profile.map((p) => p.minutes));
  const peak = peakHour(profile);
  const gap = (1.6 * Math.PI) / 180;
  const angle = (h: number) => (h / 24) * 2 * Math.PI - Math.PI / 2;
  const label = peak === null ? "No tracked time" : `Most time is tracked ${hourRangeLabel(peak)}`;
  return (
    <svg viewBox={`0 0 ${size} ${size}`} role="img" aria-label={`24-hour clock of when work happens. ${label}.`} className="mx-auto block h-auto w-full max-w-[300px]">
      <circle cx={c} cy={c} r={r1} fill="none" stroke="var(--border)" strokeDasharray="2 5" />
      <circle cx={c} cy={c} r={(r0 + r1) / 2} fill="none" stroke="var(--border)" strokeDasharray="2 5" opacity={0.6} />
      {profile.map((p) => {
        const len = r0 + 3 + (p.minutes / max) * (r1 - r0 - 3);
        const isPeak = p.hour === peak;
        return (
          <path
            key={p.hour}
            d={sector(c, c, r0, p.minutes > 0 ? len : r0 + 3, angle(p.hour) + gap, angle(p.hour + 1) - gap)}
            fill={p.minutes > 0 ? (isPeak ? "#7c3aed" : "var(--primary)") : "var(--muted)"}
            fillOpacity={p.minutes > 0 ? (isPeak ? 1 : 0.3 + 0.7 * (p.minutes / max)) : 1}
            className="print-exact chart-grow"
            style={{ transformOrigin: `${c}px ${c}px`, transformBox: "view-box", animationDelay: `${p.hour * 35}ms` }}
          >
            <title>{`${hourRangeLabel(p.hour)}: ${p.minutes > 0 ? fmt(p.minutes) : "no time"}`}</title>
          </path>
        );
      })}
      {HOUR_TICKS.map((h) => {
        const a = angle(h);
        return (
          <text key={h} x={c + (r1 + 14) * Math.cos(a)} y={c + (r1 + 14) * Math.sin(a)} textAnchor="middle" dominantBaseline="central" fontSize={10} fill="var(--muted-foreground)">
            {String(h).padStart(2, "0")}
          </text>
        );
      })}
      <circle cx={c} cy={c} r={r0 - 6} fill="var(--card)" stroke="var(--border)" />
      <text x={c} y={c - 6} textAnchor="middle" fontSize={16} fontWeight={700} fill="var(--foreground)">
        {peak === null ? "—" : clockLabel(peak * 60)}
      </text>
      <text x={c} y={c + 12} textAnchor="middle" fontSize={9.5} fill="var(--muted-foreground)">
        peak hour
      </text>
    </svg>
  );
}

/** Each day's working window (first to last tracked minute, on the members' own clocks) as floating bars. */
export function WorkWindowsChart({ windows, fmt, avgStart, avgEnd }: { windows: WorkWindow[]; fmt: Fmt; avgStart: number | null; avgEnd: number | null }) {
  const reduced = usePrefersReducedMotion();
  const id = gradientId(useId());
  const worked = windows.filter((w) => w.start !== null);
  const lo = Math.max(0, Math.floor(Math.min(...worked.map((w) => w.start as number)) / 60) - 1);
  const hi = Math.min(24, Math.ceil(Math.max(...worked.map((w) => w.end as number)) / 60) + 1);
  const span = hi - lo;
  const step = span > 12 ? 3 : span > 6 ? 2 : 1;
  const ticks: number[] = [];
  for (let t = lo; t <= hi; t += step) ticks.push(t);
  const data = windows.map((w) => ({
    label: w.label,
    range: w.start === null ? null : [w.start / 60, (w.end as number) / 60],
    window: w,
  }));
  return (
    <ChartBox className="analytics-chart h-[260px] w-full min-w-0 sm:h-[300px]">
      {({ Bar, BarChart, CartesianGrid, ReferenceLine, Tooltip, XAxis, YAxis }) => (
        <BarChart data={data} margin={{ top: 10, right: 8, left: -4, bottom: 0 }}>
          <defs>
            <linearGradient id={`${id}-win`} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" style={{ stopColor: "#7c3aed", stopOpacity: 0.95 }} />
              <stop offset="100%" style={{ stopColor: "var(--primary)", stopOpacity: 0.95 }} />
            </linearGradient>
          </defs>
          <CartesianGrid vertical={false} {...grid} />
          <XAxis dataKey="label" tickLine={false} axisLine={false} tick={axis} interval="preserveStartEnd" minTickGap={10} />
          <YAxis reversed domain={[lo, hi]} ticks={ticks} tickLine={false} axisLine={false} tick={axis} width={46} tickFormatter={(v) => clockLabel(v * 60)} />
          <Tooltip
            cursor={{ fill: "var(--muted)", radius: 8 }}
            content={(p) => {
              const row = first<{ window: WorkWindow }>(p as TipProps);
              if (!row) return null;
              const w = row.window;
              return (
                <TooltipBox
                  title={w.label}
                  rows={
                    w.start === null
                      ? [["Worked", "No time tracked"]]
                      : [
                          ["First start", clockLabel(w.start)],
                          ["Last finish", clockLabel(w.end as number)],
                          ["Hours inside", fmt(w.minutes)],
                          ...(w.estimated ? ([["Clock", "report zone (no device zone)"]] as [string, string][]) : []),
                        ]
                  }
                />
              );
            }}
          />
          {avgStart !== null && <ReferenceLine y={avgStart / 60} stroke="var(--success)" strokeDasharray="4 4" />}
          {avgEnd !== null && <ReferenceLine y={avgEnd / 60} stroke="var(--danger)" strokeDasharray="4 4" />}
          <Bar dataKey="range" fill={`url(#${id}-win)`} radius={8} maxBarSize={30} isAnimationActive={!reduced} animationDuration={1000} />
        </BarChart>
      )}
    </ChartBox>
  );
}

/* ----------------------------- Day timeline ----------------------------- */

/** How each kind of gap looks (hatched, in its own colour) and reads. */
export const GAP_STYLE: Record<GapKind, string> = {
  break: "tl-gap tl-gap-break",
  idle: "tl-gap tl-gap-idle",
  lock: "tl-gap tl-gap-lock",
  sleep: "tl-gap tl-gap-sleep",
  off: "tl-gap tl-gap-off",
  untracked: "tl-gap tl-gap-untracked",
};

/** A lane's label: the member, and their start and finish on their own clock. */
export interface TimelineLaneMeta {
  name: string;
  /** e.g. "08:58–17:42 local (UTC−7)"; omitted when it would only repeat the axis. */
  local?: string | null;
}

/** The legend of a day timeline: worked time, and every kind of gap. */
export function TimelineLegend({ kinds }: { kinds: GapKind[] }) {
  const labels: Record<GapKind, string> = {
    break: "Break",
    idle: "Idle",
    lock: "Screen locked",
    sleep: "Asleep",
    off: "Paused / off",
    untracked: "Not tracked",
  };
  return (
    <ChartLegend>
      <LegendItem color="var(--primary)" label="Tracked (project colour)" />
      {kinds.map((k) => (
        <span key={k} className="flex items-center gap-1.5">
          <span className={cn("print-exact h-2.5 w-3.5 rounded-sm", GAP_STYLE[k])} />
          {labels[k]}
        </span>
      ))}
    </ChartLegend>
  );
}

/**
 * One day on a clock, a lane per member: tracked blocks in their project's
 * colour and the gaps between them, labelled with why the block before ended
 * (break, idle, screen locked, asleep…). The axis is the reporting zone; each
 * lane also names the member's own start and finish. Screen readers get each
 * lane as an ordered list of what happened when.
 */
export function DayTimelineChart({
  timeline,
  meta,
  colorOf,
  projectNameOf,
  clockOf,
  fmt,
  compact = false,
}: {
  timeline: DayTimeline;
  meta: (userId: string) => TimelineLaneMeta;
  colorOf: (projectId: string) => string;
  projectNameOf: (projectId: string) => string;
  /** "09:30" in the reporting zone. */
  clockOf: (ms: number) => string;
  fmt: Fmt;
  /** A denser layout for a day opened inside a table. */
  compact?: boolean;
}) {
  const span = Math.max(1, timeline.to - timeline.from);
  const pos = (ms: number) => Math.min(100, Math.max(0, ((ms - timeline.from) / span) * 100));
  const hours = Math.round(span / 3_600_000);
  const step = hours > 16 ? 4 : hours > 8 ? 2 : 1;
  const ticks: number[] = [];
  for (let t = timeline.from, i = 0; t <= timeline.to; t += 3_600_000, i++) if (i % step === 0) ticks.push(t);
  const range = (a: number, b: number) => `${clockOf(a)}–${clockOf(b)}`;
  return (
    <div className={cn("min-w-0", compact ? "space-y-2" : "space-y-3")}>
      <div aria-hidden className={cn("relative h-4 text-[10px] text-muted-foreground", compact ? "ml-0 sm:ml-36" : "ml-0 sm:ml-44")}>
        {ticks.map((t, i) => (
          <span
            key={t}
            // The end labels align inward, so the axis never reaches past its track.
            className={cn("absolute tabular-nums", i === 0 ? "" : i === ticks.length - 1 && pos(t) >= 99 ? "-translate-x-full" : "-translate-x-1/2")}
            style={{ left: `${pos(t)}%` }}
          >
            {clockOf(t)}
          </span>
        ))}
      </div>
      <ul className={compact ? "space-y-2" : "space-y-3"}>
        {timeline.lanes.map((lane) => {
          const info = meta(lane.userId);
          const events = [
            ...lane.segments.map((s) => ({ kind: "work" as const, start: s.start, end: s.end, segment: s })),
            ...lane.gaps.map((g) => ({ kind: "gap" as const, start: g.start, end: g.end, gap: g })),
          ].sort((a, b) => a.start - b.start);
          return (
            <li key={lane.userId} className="flex flex-col gap-1.5 sm:flex-row sm:items-center sm:gap-3">
              <div className={cn("min-w-0 shrink-0", compact ? "sm:w-33" : "sm:w-41")}>
                <p className="truncate text-sm font-medium" title={info.name}>
                  {info.name}
                </p>
                <p className="truncate text-[11px] text-muted-foreground tabular-nums">
                  {fmt(lane.minutes)}
                  {info.local ? ` · ${info.local}` : ` · ${range(lane.first, lane.last)}`}
                </p>
              </div>
              <ol
                aria-label={`${info.name}: ${range(lane.first, lane.last)}`}
                className={cn("print-exact relative w-full min-w-0 overflow-hidden rounded-lg bg-muted/60", compact ? "h-6" : "h-8")}
              >
                {events.map((e) => {
                  const left = pos(e.start);
                  const width = Math.max(0.6, pos(e.end) - left);
                  if (e.kind === "work") {
                    const s = e.segment;
                    const detail = `${range(s.start, s.end)} · ${projectNameOf(s.projectId)} · ${fmt((s.end - s.start) / 60_000)}${
                      s.activity === null ? "" : ` · ${s.activity}% activity`
                    }${s.note ? ` · ${s.note}` : ""}`;
                    return (
                      <li
                        key={`w${s.start}`}
                        title={detail}
                        className="print-exact absolute inset-y-0.5 rounded-[5px]"
                        style={{ left: `${left}%`, width: `${width}%`, background: colorOf(s.projectId) }}
                      >
                        <span className="sr-only">Worked {detail}</span>
                      </li>
                    );
                  }
                  const g = gapKind(e.gap.reason);
                  const detail = `${range(e.gap.start, e.gap.end)} · ${g.description} · ${fmt((e.gap.end - e.gap.start) / 60_000)}`;
                  const roomy = width >= (compact ? 9 : 7);
                  return (
                    <li
                      key={`g${e.gap.start}`}
                      title={detail}
                      className={cn(
                        "print-exact absolute inset-y-0.5 flex items-center justify-center overflow-hidden rounded-[5px] px-1",
                        GAP_STYLE[g.kind],
                      )}
                      style={{ left: `${left}%`, width: `${width}%` }}
                    >
                      <span aria-hidden className={cn("truncate text-[10px] font-semibold leading-none", !roomy && "hidden")}>
                        {g.label}
                      </span>
                      <span className="sr-only">{detail}</span>
                    </li>
                  );
                })}
              </ol>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

/* ----------------------------- Focus & activity ----------------------------- */

function TopValue({ x, y, width, value, format }: { x?: number | string; y?: number | string; width?: number | string; value?: unknown; format: (v: number) => string }) {
  const v = Number(value);
  if (!(v > 0)) return null;
  return (
    <text
      x={Number(x) + Number(width) / 2}
      y={Number(y) - 7}
      textAnchor="middle"
      style={{ fill: "var(--muted-foreground)", fontSize: 11, fontWeight: 600, stroke: "var(--card)", strokeWidth: 4, paintOrder: "stroke" }}
    >
      {format(v)}
    </text>
  );
}

/** How many continuous stretches of work fall in each length bucket. */
export function FocusChart({ data, fmt }: { data: { label: string; count: number; minutes: number }[]; fmt: Fmt }) {
  const reduced = usePrefersReducedMotion();
  const id = gradientId(useId());
  return (
    <ChartBox className="analytics-chart h-[240px] w-full min-w-0">
      {({ Bar, BarChart, CartesianGrid, LabelList, Tooltip, XAxis, YAxis }) => (
        <BarChart data={data} margin={{ top: 24, right: 6, left: -6, bottom: 0 }}>
          <defs>
            <linearGradient id={`${id}-focus`} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" style={{ stopColor: "#7c3aed", stopOpacity: 1 }} />
              <stop offset="100%" style={{ stopColor: "#7c3aed", stopOpacity: 0.55 }} />
            </linearGradient>
          </defs>
          <CartesianGrid vertical={false} {...grid} />
          <XAxis dataKey="label" tickLine={false} axisLine={false} tick={axis} />
          <YAxis tickLine={false} axisLine={false} tick={axis} allowDecimals={false} width={40} />
          <Tooltip
            cursor={{ fill: "var(--muted)", radius: 8 }}
            content={(p) => {
              const row = first<{ label: string; count: number; minutes: number }>(p as TipProps);
              return row ? <TooltipBox title={row.label} rows={[["Stretches", String(row.count), "#7c3aed"], ["Time in them", fmt(row.minutes)]]} /> : null;
            }}
          />
          <Bar dataKey="count" fill={`url(#${id}-focus)`} radius={[8, 8, 3, 3]} maxBarSize={56} isAnimationActive={!reduced} animationDuration={1000}>
            <LabelList dataKey="count" content={(props) => <TopValue {...props} format={(v) => String(v)} />} />
          </Bar>
        </BarChart>
      )}
    </ChartBox>
  );
}

/** Red → amber → green by score, from the theme's own status colours; a muted grey for "not measured". */
export function scoreFill(score: number | null): string {
  if (score === null) return "var(--muted-foreground)";
  const s = Math.min(100, Math.max(0, score));
  return s < 50
    ? `color-mix(in srgb, var(--warning) ${Math.round((s / 50) * 100)}%, var(--danger))`
    : `color-mix(in srgb, var(--success) ${Math.round(((s - 50) / 50) * 100)}%, var(--warning))`;
}

/** Tracked hours by measured activity score. */
export function ActivityDistributionChart({ data, fmt }: { data: { label: string; from: number; minutes: number; blocks: number }[]; fmt: Fmt }) {
  const reduced = usePrefersReducedMotion();
  const rows = data.map((d) => ({ ...d, hours: Math.round((d.minutes / 60) * 100) / 100 }));
  return (
    <ChartBox className="analytics-chart h-[240px] w-full min-w-0">
      {({ Bar, BarChart, CartesianGrid, Cell, Tooltip, XAxis, YAxis }) => (
        <BarChart data={rows} margin={{ top: 10, right: 6, left: -12, bottom: 0 }}>
          <CartesianGrid vertical={false} {...grid} />
          <XAxis dataKey="from" tickLine={false} axisLine={false} tick={axis} tickFormatter={(v) => `${v}%`} />
          <YAxis tickLine={false} axisLine={false} tick={axis} width={40} tickFormatter={(v) => `${v}h`} />
          <Tooltip
            cursor={{ fill: "var(--muted)", radius: 6 }}
            content={(p) => {
              const row = first<(typeof rows)[number]>(p as TipProps);
              return row ? <TooltipBox title={`Activity ${row.label}`} rows={[["Hours", fmt(row.minutes)], ["Blocks", String(row.blocks)]]} /> : null;
            }}
          />
          <Bar dataKey="hours" radius={[6, 6, 2, 2]} isAnimationActive={!reduced} animationDuration={1100}>
            {rows.map((r) => (
              <Cell key={r.from} fill={scoreFill(r.from + 5)} />
            ))}
          </Bar>
        </BarChart>
      )}
    </ChartBox>
  );
}

/** Hours (bars) and measured activity % (line) by hour of day. */
export function HourActivityChart({ profile, fmt }: { profile: HourStat[]; fmt: Fmt }) {
  const reduced = usePrefersReducedMotion();
  const rows = profile.map((p) => ({
    hour: String(p.hour).padStart(2, "0"),
    stat: p,
    hours: Math.round((p.minutes / 60) * 100) / 100,
    activity: p.minutes > 0 ? p.activity : null,
  }));
  return (
    <ChartBox className="analytics-chart h-[260px] w-full min-w-0">
      {({ Bar, CartesianGrid, ComposedChart, Line, Tooltip, XAxis, YAxis }) => (
        <ComposedChart data={rows} margin={{ top: 10, right: 0, left: -12, bottom: 0 }}>
          <CartesianGrid vertical={false} {...grid} />
          <XAxis dataKey="hour" tickLine={false} axisLine={false} tick={axis} interval={2} />
          <YAxis yAxisId="h" tickLine={false} axisLine={false} tick={axis} width={40} tickFormatter={(v) => `${v}h`} />
          <YAxis yAxisId="a" orientation="right" tickLine={false} axisLine={false} tick={axis} width={36} domain={[0, 100]} tickFormatter={(v) => `${v}%`} />
          <Tooltip
            cursor={{ fill: "var(--muted)", radius: 4 }}
            content={(p) => {
              const row = first<(typeof rows)[number]>(p as TipProps);
              return row ? (
                <TooltipBox
                  title={hourRangeLabel(row.stat.hour)}
                  rows={[
                    ["Hours", fmt(row.stat.minutes), "var(--primary)"],
                    ["Activity", formatActivity(row.activity), "var(--success)"],
                    ["Key presses + clicks", (row.stat.keys + row.stat.clicks).toLocaleString("en")],
                  ]}
                />
              ) : null;
            }}
          />
          <Bar yAxisId="h" dataKey="hours" fill="var(--primary)" fillOpacity={0.35} radius={[4, 4, 0, 0]} isAnimationActive={!reduced} animationDuration={900} />
          <Line yAxisId="a" type="monotone" dataKey="activity" stroke="var(--success)" strokeWidth={2.5} dot={false} connectNulls={false} isAnimationActive={!reduced} animationDuration={1400} />
        </ComposedChart>
      )}
    </ChartBox>
  );
}

/* ----------------------------- Projects & apps ----------------------------- */

export interface TreemapItem {
  name: string;
  size: number;
  minutes: number;
  share: number;
  color: string;
}

/** Projects as tiles sized by hours (squarified), growing in one after another. */
export function ProjectTreemap({ items, fmt }: { items: TreemapItem[]; fmt: Fmt }) {
  const [ref, width] = useElementWidth<HTMLDivElement>();
  const height = 280;
  const tiles = squarify(items, (i) => i.size, width, height);
  return (
    <div ref={ref} className="analytics-chart w-full min-w-0">
      <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} role="img" aria-label={`Project map: ${items.map((i) => `${i.name} ${fmt(i.minutes)}`).join(", ")}`} className="block max-w-full">
        {tiles.map((t, i) => {
          const roomy = t.w > 84 && t.h > 46;
          const maxChars = Math.max(3, Math.floor((t.w - 20) / 7));
          const name = t.item.name.length > maxChars ? `${t.item.name.slice(0, maxChars - 1)}…` : t.item.name;
          return (
            <g
              key={t.item.name}
              className="chart-grow"
              style={{ transformOrigin: `${t.x + t.w / 2}px ${t.y + t.h / 2}px`, transformBox: "view-box", animationDelay: `${i * 70}ms` }}
            >
              <rect x={t.x + 2} y={t.y + 2} width={Math.max(0, t.w - 4)} height={Math.max(0, t.h - 4)} rx={10} fill={t.item.color} fillOpacity={0.92} className="print-exact transition-[fill-opacity] duration-150 hover:[fill-opacity:1]">
                <title>{`${t.item.name}: ${fmt(t.item.minutes)} (${t.item.share}%)`}</title>
              </rect>
              {roomy && (
                <>
                  <text x={t.x + 12} y={t.y + 24} fontSize={12.5} fontWeight={600} fill="#fff" pointerEvents="none" style={{ paintOrder: "stroke", stroke: "rgb(0 0 0 / 0.35)", strokeWidth: 2 }}>
                    {name}
                  </text>
                  <text x={t.x + 12} y={t.y + 41} fontSize={11} fill="#fff" pointerEvents="none" style={{ paintOrder: "stroke", stroke: "rgb(0 0 0 / 0.35)", strokeWidth: 2 }}>
                    {fmt(t.item.minutes)} · {t.item.share}%
                  </text>
                </>
              )}
            </g>
          );
        })}
      </svg>
    </div>
  );
}

/** Hours per day, stacked by project. */
export function ProjectStackChart({ rows, series, fmt }: { rows: Record<string, number | string>[]; series: { key: string; name: string; color: string }[]; fmt: Fmt }) {
  const reduced = usePrefersReducedMotion();
  return (
    <ChartBox className="analytics-chart h-[260px] w-full min-w-0">
      {({ Bar, BarChart, CartesianGrid, Tooltip, XAxis, YAxis }) => (
        <BarChart data={rows} margin={{ top: 10, right: 6, left: -8, bottom: 0 }}>
          <CartesianGrid vertical={false} {...grid} />
          <XAxis dataKey="label" tickLine={false} axisLine={false} tick={axis} interval="preserveStartEnd" minTickGap={12} />
          <YAxis tickLine={false} axisLine={false} tick={axis} width={40} tickFormatter={(v) => `${v}h`} />
          <Tooltip
            cursor={{ fill: "var(--muted)", radius: 6 }}
            content={(p) => {
              const row = first<Record<string, number | string>>(p as TipProps);
              if (!row) return null;
              const parts = series
                .map((s) => [s.name, Number(row[s.key]) || 0, s.color] as const)
                .filter(([, v]) => v > 0)
                .map(([n, v, c]) => [n, fmt(v * 60), c] as [string, string, string]);
              return <TooltipBox title={String(row.label)} rows={parts.length ? parts : [["Hours", "No time tracked"]]} />;
            }}
          />
          {series.map((s, i) => (
            <Bar key={s.key} dataKey={s.key} name={s.name} stackId="p" fill={s.color} radius={i === series.length - 1 ? [5, 5, 0, 0] : 0} isAnimationActive={!reduced} animationDuration={900} animationBegin={i * 80} />
          ))}
        </BarChart>
      )}
    </ChartBox>
  );
}

export interface FlowNodeData {
  name: string;
  kind: "member" | "project";
  id: string;
  color: string;
}

/**
 * Who works on what: members (left) flowing into projects (right), band
 * widths in hours. Bands take their PROJECT's colour all the way across and
 * member nodes stay neutral, so a member can never be mistaken for a project.
 */
export function FlowDiagram({ flow, fmt }: { flow: { nodes: FlowNodeData[]; links: FlowData["links"] }; fmt: Fmt }) {
  const [ref, width] = useElementWidth<HTMLDivElement>();
  const [hover, setHover] = useState<number | null>(null);
  const wide = width >= 640;
  const side = Math.max(
    flow.nodes.filter((n) => n.kind === "member").length,
    flow.nodes.filter((n) => n.kind === "project").length,
  );
  const height = Math.max(220, side * 46);
  const left = wide ? 190 : 104;
  const right = wide ? 210 : 104;
  const { nodes, links } = layoutFlow(flow, width, height, { nodeWidth: 12, gap: 14, left, right });
  const clip = (name: string) => {
    const max = wide ? 22 : 12;
    return name.length > max ? `${name.slice(0, max - 1)}…` : name;
  };
  const active = (i: number) => hover === null || links[i].source === hover || links[i].target === hover;
  return (
    <div ref={ref} className="analytics-chart w-full min-w-0">
      <svg
        width={width}
        height={height}
        viewBox={`0 0 ${width} ${height}`}
        role="img"
        aria-label={`Hours from members to projects: ${links
          .map((l) => `${flow.nodes[l.source].name} to ${flow.nodes[l.target].name} ${fmt(l.value * 60)}`)
          .join(", ")}`}
        className="block max-w-full"
        onMouseLeave={() => setHover(null)}
      >
        {links.map((l, i) => {
          const mid = (l.x0 + l.x1) / 2;
          return (
            <path
              key={`${l.source}-${l.target}`}
              d={`M${l.x0},${l.sy} C${mid},${l.sy} ${mid},${l.ty} ${l.x1},${l.ty}`}
              fill="none"
              stroke={flow.nodes[l.target].color}
              strokeWidth={l.thickness}
              strokeOpacity={active(i) ? (hover === null ? 0.35 : 0.65) : 0.08}
              className="print-exact animate-fade-in transition-[stroke-opacity] duration-200"
              style={{ animationDelay: `${i * 40}ms` }}
            >
              <title>{`${flow.nodes[l.source].name} → ${flow.nodes[l.target].name}: ${fmt(l.value * 60)}`}</title>
            </path>
          );
        })}
        {nodes.map((n) => {
          const isMember = n.node.kind === "member";
          return (
            <g key={n.index} onMouseEnter={() => setHover(n.index)} className="cursor-default">
              <rect
                x={n.x}
                y={n.y}
                width={12}
                height={n.height}
                rx={3}
                fill={flow.nodes[n.index].color}
                className="print-exact chart-grow"
                style={{ transformOrigin: `${n.x + 6}px ${n.y + n.height / 2}px`, transformBox: "view-box" }}
              />
              <text
                x={isMember ? n.x - 8 : n.x + 20}
                y={n.y + n.height / 2}
                textAnchor={isMember ? "end" : "start"}
                dominantBaseline="central"
                fontSize={12}
                fontWeight={hover === n.index ? 700 : 500}
                fill="var(--foreground)"
              >
                {clip(n.node.name)}
                {wide && <tspan fill="var(--muted-foreground)" fontSize={11} fontWeight={400}>{`  ${fmt(n.value * 60)}`}</tspan>}
                <title>{`${n.node.name}: ${fmt(n.value * 60)}`}</title>
              </text>
            </g>
          );
        })}
      </svg>
    </div>
  );
}

const APP_LABEL_MAX = 18;

/** The most-used applications as horizontal bars, each in its own colour (the same on every page). */
export function AppsChart({ apps, fmt }: { apps: AppStat[]; fmt: Fmt }) {
  const reduced = usePrefersReducedMotion();
  const rows = apps.map((a) => ({ ...a, hours: Math.round((a.minutes / 60) * 100) / 100 }));
  const height = Math.max(180, rows.length * 38);
  return (
    <ChartBox className="analytics-chart w-full min-w-0" style={{ height }}>
      {({ Bar, BarChart, Cell, LabelList, Tooltip, XAxis, YAxis }) => (
        <BarChart data={rows} layout="vertical" margin={{ top: 0, right: 56, left: 4, bottom: 0 }} barCategoryGap={8}>
          <XAxis type="number" hide />
          <YAxis
            type="category"
            dataKey="app"
            width={132}
            tickLine={false}
            axisLine={false}
            tick={{ ...axis, fill: "var(--foreground)", fontSize: 12 }}
            tickFormatter={(v: string) => (v.length > APP_LABEL_MAX ? `${v.slice(0, APP_LABEL_MAX - 1)}…` : v)}
          />
          <Tooltip
            cursor={{ fill: "var(--muted)", radius: 6 }}
            content={(p) => {
              const row = first<(typeof rows)[number]>(p as TipProps);
              return row ? <TooltipBox title={row.app} rows={[["Focused", fmt(row.minutes), row.color], ["Share", `${row.share}%`]]} /> : null;
            }}
          />
          <Bar dataKey="hours" radius={[0, 8, 8, 0]} isAnimationActive={!reduced} animationDuration={1000}>
            {rows.map((r) => (
              <Cell key={r.app} fill={r.color} />
            ))}
            <LabelList dataKey="share" position="right" formatter={(v: unknown) => `${String(v)}%`} style={{ fill: "var(--muted-foreground)", fontSize: 11, fontWeight: 600 }} />
          </Bar>
        </BarChart>
      )}
    </ChartBox>
  );
}

/* ----------------------------- Devices ----------------------------- */

/** Platform colours: Windows blue, macOS lavender, unreported slate (each ≥ 3:1 on both themes' cards). */
export const PLATFORM_COLORS: Record<"windows" | "macos" | "unknown", string> = {
  windows: "#2d6fd4",
  macos: "#937cf1",
  unknown: "#768390",
};

/** Tracked time by platform as one stacked bar, with each platform's devices, members and share. */
export function PlatformMix({ shares, fmt }: { shares: PlatformShare[]; fmt: Fmt }) {
  const color = (p: PlatformShare) => PLATFORM_COLORS[p.platform ?? "unknown"];
  return (
    <div className="space-y-3">
      <div aria-hidden className="print-exact flex h-3 w-full overflow-hidden rounded-full bg-muted">
        {shares.map((p) => (
          <span key={p.label} className="print-exact h-full" style={{ width: `${p.share}%`, background: color(p) }} />
        ))}
      </div>
      <ul aria-label="Tracked time by platform" className="space-y-2">
        {shares.map((p) => (
          <li key={p.label} className="flex items-center justify-between gap-3 text-sm">
            <span className="flex min-w-0 items-center gap-2">
              <span aria-hidden className="print-exact h-2.5 w-2.5 shrink-0 rounded-full" style={{ background: color(p) }} />
              <span className="truncate font-medium">{p.label}</span>
              <span className="shrink-0 text-xs text-muted-foreground">
                {p.devices} {p.devices === 1 ? "device" : "devices"}
              </span>
            </span>
            <span className="shrink-0 tabular-nums">
              <span className="font-semibold">{fmt(p.minutes)}</span>
              <span className="text-muted-foreground"> · {p.share}%</span>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/* ----------------------------- Team ----------------------------- */

export interface ScatterPoint {
  id: string;
  name: string;
  hours: number;
  /** Measured activity; members without a measurement aren't plotted. */
  activity: number;
  sessions: number;
  color: string;
}

const SCATTER_MARGIN = { top: 14, right: 16, bottom: 30, left: 44 };
const SCATTER_HEIGHT = 300;
const LABEL_FONT = 11;

/** Round axis steps covering `max`. */
function niceTicks(max: number): number[] {
  const want = Math.max(max, 1);
  const raw = want / 4;
  const pow = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * pow).find((s) => s >= raw) ?? raw;
  const ticks: number[] = [];
  for (let t = 0; t <= want + step * 0.999; t += step) ticks.push(Math.round(t * 100) / 100);
  return ticks;
}

/**
 * Each member by hours (x) and activity (y), bubbles sized by sessions;
 * dashed lines mark the team means. Labels are placed so they never overlap a
 * bubble or each other (a crowded point keeps its name in the tooltip and the
 * table). Every bubble can be focused, and reads its figures aloud.
 */
export function TeamScatter({ points, fmt }: { points: ScatterPoint[]; fmt: Fmt }) {
  const [ref, width] = useElementWidth<HTMLDivElement>();
  const [focus, setFocus] = useState<string | null>(null);
  const plotW = Math.max(40, width - SCATTER_MARGIN.left - SCATTER_MARGIN.right);
  const plotH = SCATTER_HEIGHT - SCATTER_MARGIN.top - SCATTER_MARGIN.bottom;
  const xTicks = niceTicks(Math.max(0, ...points.map((p) => p.hours)));
  const xMax = xTicks[xTicks.length - 1] || 1;
  const x = (hours: number) => SCATTER_MARGIN.left + (hours / xMax) * plotW;
  const y = (activity: number) => SCATTER_MARGIN.top + (1 - activity / 100) * plotH;
  const maxSessions = Math.max(1, ...points.map((p) => p.sessions));
  const radius = (sessions: number) => 6 + 12 * Math.sqrt(Math.max(0, sessions) / maxSessions);
  const meanHours = points.reduce((s, p) => s + p.hours, 0) / Math.max(1, points.length);
  const meanActivity = points.reduce((s, p) => s + p.activity, 0) / Math.max(1, points.length);
  const marks = [...points]
    .sort((a, b) => b.hours - a.hours || a.name.localeCompare(b.name))
    .map((p) => ({ ...p, cx: x(p.hours), cy: y(p.activity), r: radius(p.sessions), short: p.name.split(" ")[0] || p.name }));
  // Labels stay clear of the axis labels: inside the plot, or out into the right margin.
  const labels = placeLabels(
    marks.map((m) => ({ id: m.id, x: m.cx, y: m.cy, r: m.r, width: m.short.length * LABEL_FONT * 0.6 + 6, height: LABEL_FONT + 4 })),
    { x0: SCATTER_MARGIN.left, y0: 0, x1: width, y1: SCATTER_MARGIN.top + plotH },
  );
  const shown = marks.find((m) => m.id === focus) ?? null;
  const describe = (m: (typeof marks)[number]) => `${m.name}: ${fmt(m.hours * 60)}, ${m.activity}% activity, ${m.sessions} ${m.sessions === 1 ? "session" : "sessions"}`;
  return (
    <div ref={ref} className="analytics-chart w-full min-w-0">
      <svg width={width} height={SCATTER_HEIGHT} viewBox={`0 0 ${width} ${SCATTER_HEIGHT}`} role="group" aria-label="Hours and activity per member" className="block max-w-full overflow-visible">
        <g aria-hidden>
          {[0, 25, 50, 75, 100].map((t) => (
            <g key={`y${t}`}>
              <line x1={SCATTER_MARGIN.left} x2={SCATTER_MARGIN.left + plotW} y1={y(t)} y2={y(t)} stroke="var(--border)" strokeDasharray="3 4" />
              <text x={SCATTER_MARGIN.left - 8} y={y(t)} textAnchor="end" dominantBaseline="central" fontSize={11} fill="var(--muted-foreground)">
                {t}%
              </text>
            </g>
          ))}
          {xTicks.map((t) => (
            <g key={`x${t}`}>
              <line x1={x(t)} x2={x(t)} y1={SCATTER_MARGIN.top} y2={SCATTER_MARGIN.top + plotH} stroke="var(--border)" strokeDasharray="3 4" />
              <text x={x(t)} y={SCATTER_HEIGHT - 10} textAnchor="middle" fontSize={11} fill="var(--muted-foreground)">
                {t}h
              </text>
            </g>
          ))}
          {points.length > 0 && (
            <>
              <line x1={x(meanHours)} x2={x(meanHours)} y1={SCATTER_MARGIN.top} y2={SCATTER_MARGIN.top + plotH} stroke="var(--muted-foreground)" strokeDasharray="4 4" />
              <line x1={SCATTER_MARGIN.left} x2={SCATTER_MARGIN.left + plotW} y1={y(meanActivity)} y2={y(meanActivity)} stroke="var(--muted-foreground)" strokeDasharray="4 4" />
            </>
          )}
        </g>
        {marks.map((m) => (
          <g
            key={m.id}
            role="img"
            tabIndex={0}
            aria-label={describe(m)}
            onMouseEnter={() => setFocus(m.id)}
            onMouseLeave={() => setFocus((f) => (f === m.id ? null : f))}
            onFocus={() => setFocus(m.id)}
            onBlur={() => setFocus((f) => (f === m.id ? null : f))}
            className="cursor-default outline-none"
          >
            <circle
              cx={m.cx}
              cy={m.cy}
              r={m.r}
              fill={m.color}
              fillOpacity={focus === m.id ? 0.9 : 0.7}
              stroke={focus === m.id ? "var(--foreground)" : m.color}
              strokeWidth={focus === m.id ? 2 : 1.5}
              className="print-exact chart-grow"
              style={{ transformOrigin: `${m.cx}px ${m.cy}px`, transformBox: "view-box" }}
            />
          </g>
        ))}
        <g aria-hidden>
          {marks.map((m) => {
            const at = labels.get(m.id);
            return at ? (
              <text
                key={m.id}
                x={at.x}
                y={at.y}
                textAnchor="middle"
                dominantBaseline="central"
                fontSize={LABEL_FONT}
                fontWeight={600}
                fill="var(--foreground)"
                style={{ paintOrder: "stroke", stroke: "var(--card)", strokeWidth: 3 }}
              >
                {m.short}
              </text>
            ) : null;
          })}
        </g>
      </svg>
      <p aria-hidden className="mt-1 min-h-4 text-xs font-medium text-foreground">
        {shown ? describe(shown) : <span className="font-normal text-muted-foreground">Hover or focus a bubble for the figures</span>}
      </p>
    </div>
  );
}

/** Members' profiles on six comparable 0–100 scales. */
export function TeamRadar({ rows, members }: { rows: Record<string, number | string>[]; members: { id: string; name: string; color: string }[] }) {
  const reduced = usePrefersReducedMotion();
  return (
    <ChartBox className="analytics-chart h-[320px] w-full min-w-0">
      {({ PolarAngleAxis, PolarGrid, PolarRadiusAxis, Radar, RadarChart, Tooltip }) => (
        <RadarChart data={rows} outerRadius="72%" margin={{ top: 8, right: 24, bottom: 8, left: 24 }}>
          <PolarGrid stroke="var(--border)" />
          <PolarAngleAxis dataKey="dimension" tick={{ fill: "var(--muted-foreground)", fontSize: 11 }} />
          <PolarRadiusAxis domain={[0, 100]} tick={false} axisLine={false} />
          <Tooltip
            content={(p) => {
              const tip = p as unknown as { active?: boolean; label?: string; payload?: ReadonlyArray<{ name?: string; value?: number; color?: string }> };
              if (!tip.active || !tip.payload?.length) return null;
              return <TooltipBox title={String(tip.label ?? "")} rows={tip.payload.map((x) => [String(x.name), `${x.value ?? 0}`, x.color])} />;
            }}
          />
          {members.map((m, i) => (
            <Radar key={m.id} name={m.name} dataKey={m.id} stroke={m.color} fill={m.color} fillOpacity={0.16} strokeWidth={2} isAnimationActive={!reduced} animationDuration={900} animationBegin={i * 150} />
          ))}
        </RadarChart>
      )}
    </ChartBox>
  );
}
