"use client";

import { useCallback, useMemo, useState, type ComponentProps } from "react";
import Link from "next/link";
import type { LucideIcon } from "lucide-react";
import { ArrowLeft, CalendarClock, Info, Lock, RotateCw } from "lucide-react";
import type { BarShapeProps } from "recharts";
import { ChartLoading } from "@/components/charts/chart-loading";
import { loadedRecharts, useRecharts } from "@/components/charts/use-recharts";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader } from "@/components/ui/page-header";
import { useSession } from "@/components/session-provider";
import { usePrefersReducedMotion } from "@/hooks/use-count-up";
import { useIsMdUp } from "@/hooks/use-media-query";
import { useReportingZone } from "@/hooks/use-reporting-zone";
import { useStoredChoice } from "@/hooks/use-stored-choice";
import { referenceNow } from "@/lib/live-session";
import { filterActivitiesByRange } from "@/lib/reports-data";
import { activities } from "@/lib/tenant-data";
import { ALL, resolveReportPeriod, type DayBar, type ReportPeriod, type ReportPeriodPreset } from "@/lib/report-math";
import {
  HOURS_FORMATS,
  activityLevel,
  apportionPercent,
  barPercent,
  formatAxisDay,
  formatDayLabel,
  formatHours,
  hourTicks,
  type HoursFormat,
} from "@/lib/work-report";
import type { Activity, UserStatus } from "@/lib/types";
import { cn } from "@/lib/utils";
import { HOURS_FORMAT_KEY, PeriodToolbar, Segmented, type CustomRange, type ToolbarPeriod } from "./report-controls";
import { useLiveReport, useReportAccess } from "./report-access";

// The Export menu and the access/loading hooks live in light modules of their
// own (pages outside the reports use them without this kit's charts).
export { ExportMenu, type ExportMenuItem } from "./export-menu";
export { useLiveReport, useReportAccess };

/* ============================================================================
 * The kit the period reports are built from — Time & Activity, Weekly,
 * Productivity, Apps, Projects, Attendance, Payroll and Devices — in the look
 * of the Work Report and Work Analytics: the page frame and Export menu, the
 * period toolbar (presets, ‹ › stepping, a custom range, the reporting time
 * zone), the viewer's access, live loading with honest pending states, KPI
 * tiles, and the hour-axis charts.
 *
 * Every live request of these reports carries the reporting zone, and nothing
 * is fetched until that zone is known (useReportingZone's `ready`), so a
 * report is never drawn in one zone and then redrawn in another.
 * ========================================================================== */

export function ReportShell({
  title,
  description,
  actions,
  children,
}: {
  title: string;
  description?: string;
  actions?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-6">
      <Link href="/reports" className="no-print inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground">
        <ArrowLeft className="h-4 w-4" /> All reports
      </Link>
      <PageHeader title={title} description={description} actions={actions} eyebrow="Report" />
      {children}
    </div>
  );
}

/* ----------------------------- Live loading ----------------------------- */

/**
 * A period report's live data: loaded once the reporting zone is known.
 * `pending` is true while there is nothing real to show — loading, waiting
 * for the zone, or failed — so figures read "—" and exports stay off.
 */
export function usePeriodReport<T>(live: boolean, zoneReady: boolean, load: () => Promise<T>) {
  const state = useLiveReport(live && zoneReady, load);
  const loading = live && (!zoneReady || state.loading);
  return { ...state, loading, pending: live && (loading || !!state.error || !state.data) };
}

/** Honest live status: a loading line, or a danger notice with Retry. Never "showing demo data". */
export function LiveReportNotice({
  loading,
  error,
  onRetry,
}: {
  loading: boolean;
  error: string | null;
  onRetry: () => void;
}) {
  if (error) {
    return (
      <div
        role="alert"
        className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger-on-tint"
      >
        <span className="min-w-0" title={error}>
          Couldn&apos;t load this report from the server. Figures stay empty until it loads.
        </span>
        <Button variant="outline" size="sm" className="shrink-0 border-danger/40" onClick={() => void onRetry()}>
          <RotateCw className="h-3.5 w-3.5" /> Retry
        </Button>
      </div>
    );
  }
  if (loading) {
    return (
      <div role="status" aria-live="polite" className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">
        Loading report data…
      </div>
    );
  }
  return null;
}

/* ----------------------------- Access & scope ----------------------------- */

/**
 * A report's project and member filters, and who they can name: a live viewer
 * without Tracker.Activities.ViewAll is only reported on themselves by the
 * server, so the member picker offers just them.
 */
export function useReportScope() {
  const session = useSession();
  const access = useReportAccess();
  const [projectId, setProjectId] = useState(ALL);
  const [memberId, setMemberId] = useState(ALL);
  return {
    isLive: session.isLive,
    user: session.user,
    access,
    projectId,
    setProjectId,
    memberId,
    setMemberId,
    liveWorkerId: session.isLive && !access.viewAll ? session.user.id : null,
  };
}

/* ----------------------------- Period ----------------------------- */

type PeriodToolbarProps = ComponentProps<typeof PeriodToolbar>;
type PeriodPart = Pick<
  PeriodToolbarProps,
  "preset" | "onPreset" | "title" | "offset" | "onStep" | "allowCustom" | "range" | "onRange" | "suggestedRange" | "today" | "timeZone" | "spanLabel"
>;

export interface ReportPeriodState {
  period: ReportPeriod;
  /** The reporting zone every live query is sent. */
  timeZone: string;
  /** False while a live session's reporting zone is loading: fetch nothing until it is. */
  ready: boolean;
  choosePreset: (preset: ReportPeriodPreset) => void;
  step: (by: number) => void;
  /** The period half of the toolbar's props. */
  toolbar: PeriodPart;
}

/**
 * A report's period: one-click presets stepping back in time, or a custom
 * range of days — whole days of the reporting zone (the viewer's preference,
 * then the workspace default, then the browser's), on the session's clock.
 */
export function useReportPeriod(initial: ReportPeriodPreset, live: boolean): ReportPeriodState {
  const zone = useReportingZone(live);
  const [anchor] = useState(() => referenceNow());
  const [preset, setPreset] = useState<ToolbarPeriod>(initial);
  const [offset, setOffset] = useState(0);
  const [range, setRange] = useState<CustomRange | null>(null);
  const period = useMemo(
    () => resolveReportPeriod({ preset, offset, range, now: anchor, timeZone: zone.timeZone }),
    [preset, offset, range, anchor, zone.timeZone],
  );
  const choosePreset = useCallback((next: ReportPeriodPreset) => {
    setPreset(next);
    setOffset(0);
  }, []);
  const step = useCallback((by: number) => setOffset((o) => Math.min(0, o + by)), []);
  const applyRange = useCallback((next: CustomRange) => {
    setRange(next);
    setPreset("custom");
  }, []);
  return {
    period,
    timeZone: zone.timeZone,
    ready: zone.ready,
    choosePreset,
    step,
    toolbar: {
      preset,
      onPreset: choosePreset,
      title: period.title,
      offset,
      onStep: step,
      allowCustom: true,
      range,
      onRange: applyRange,
      suggestedRange: { from: period.fromKey, to: period.toKey },
      today: period.today,
      timeZone: zone.timeZone,
      spanLabel: period.label,
    },
  };
}

/**
 * DEMO ONLY: the seeded activity of a period — booked by its end (metrics.ts'
 * rule) inside [from, to) — narrowed to the project and member filters. A live
 * session never reads it.
 */
export function demoActivities(period: Pick<ReportPeriod, "from" | "to">, projectId: string, memberId: string): Activity[] {
  return filterActivitiesByRange(activities, period.from, new Date(period.to.getTime() - 1)).filter(
    (a) => (projectId === ALL || a.projectId === projectId) && (memberId === ALL || a.userId === memberId),
  );
}

/** The period toolbar with a report's period state and filters. */
export function ReportToolbar({
  period,
  ...filters
}: { period: ReportPeriodState } & Pick<PeriodToolbarProps, "projectId" | "onProject" | "memberId" | "onMember" | "liveWorkerId" | "extra">) {
  return <PeriodToolbar {...period.toolbar} {...filters} />;
}

/* ----------------------------- Hours format ----------------------------- */

const FORMAT_KEYS = HOURS_FORMATS.map((f) => f.key);

/** How hours read (7h 30m / 7:30 / 7.50): the viewer's choice, shared with the Work Report and Analytics. */
export function useHoursFormat(): { format: HoursFormat; setFormat: (f: HoursFormat) => void; fmt: (minutes: number) => string } {
  const [format, setFormat] = useStoredChoice<HoursFormat>(HOURS_FORMAT_KEY, FORMAT_KEYS, "duration");
  const fmt = useCallback((minutes: number) => formatHours(minutes, format), [format]);
  return { format, setFormat, fmt };
}

export function HoursFormatToggle({ value, onChange }: { value: HoursFormat; onChange: (f: HoursFormat) => void }) {
  return <Segmented label="Hours format" options={HOURS_FORMATS} value={value} onChange={onChange} />;
}

/* ----------------------------- KPI tiles ----------------------------- */

export type KpiAccent = "blue" | "green" | "violet" | "amber" | "pink" | "cyan";

const kpiAccent: Record<KpiAccent, string> = {
  blue: "bg-primary/10 text-primary",
  green: "bg-success/12 text-success-on-tint",
  violet: "bg-violet-500/12 text-violet-700 dark:text-violet-300",
  amber: "bg-warning/15 text-warning-on-tint",
  pink: "bg-pink-500/12 text-pink-700 dark:text-pink-300",
  cyan: "bg-cyan-500/12 text-cyan-700 dark:text-cyan-300",
};

export function KpiGrid({ children, label = "Key figures" }: { children: React.ReactNode; label?: string }) {
  return (
    <dl aria-label={label} className="grid grid-cols-2 gap-3 sm:gap-4 lg:grid-cols-4">
      {children}
    </dl>
  );
}

/**
 * A headline figure, label above value (the Work Report's tile). The value
 * wraps instead of being cut off, so a phone always shows all of it.
 */
export function Kpi({
  label,
  value,
  icon: Icon,
  accent = "blue",
  sub,
  children,
}: {
  label: string;
  value: string;
  icon?: LucideIcon;
  accent?: KpiAccent;
  sub?: string;
  /** A small visual under the value (a mini bar). */
  children?: React.ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col rounded-2xl border border-border bg-card p-4 card-elev sm:p-5">
      <dt className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
        {Icon && (
          <span aria-hidden className={cn("print-exact flex h-7 w-7 shrink-0 items-center justify-center rounded-lg", kpiAccent[accent])}>
            <Icon className="h-3.5 w-3.5" />
          </span>
        )}
        <span className="min-w-0 leading-snug">{label}</span>
      </dt>
      <dd className="mt-2.5 font-display text-xl font-bold leading-tight tracking-tight text-ink tabular-nums [overflow-wrap:anywhere] sm:mt-3 sm:text-2xl">
        {value}
      </dd>
      {children && <dd className="mt-2.5">{children}</dd>}
      {sub && <dd className="mt-1.5 text-xs leading-snug text-muted-foreground">{sub}</dd>}
    </div>
  );
}

/** A thin proportional bar (0–100 %). */
export function MiniBar({ percent, className }: { percent: number; className: string }) {
  return (
    <div aria-hidden className="print-exact h-1.5 w-full overflow-hidden rounded-full bg-muted">
      <div className={cn("print-exact h-full rounded-full", className)} style={{ width: `${Math.max(0, Math.min(100, percent))}%` }} />
    </div>
  );
}

/* ----------------------------- Cells ----------------------------- */

/** "—" for a figure that isn't there, read out as what it means. */
export function Dash({ label = "Not available" }: { label?: string }) {
  return (
    <>
      <span aria-hidden className="text-muted-foreground">
        —
      </span>
      <span className="sr-only">{label}</span>
    </>
  );
}

const levelTone = { good: "success", ok: "warning", low: "danger", none: "muted" } as const;

/** Activity % as a toned badge (the Work Report's thresholds); "—" when it wasn't measured. */
export function ActivityBadge({ percent }: { percent: number | null }) {
  if (percent === null) return <Dash label="Not measured" />;
  return (
    <Badge tone={levelTone[activityLevel(percent)]} className="print-exact tabular-nums">
      {percent}%
    </Badge>
  );
}

/** A member: avatar, name and an optional quieter line. */
export function MemberCell({ name, sub, status }: { name: string; sub?: string | null; status?: UserStatus }) {
  return (
    <div className="flex min-w-0 items-center gap-2.5">
      <Avatar name={name} size="sm" status={status} />
      <div className="min-w-0">
        <div className="truncate font-medium" title={name}>
          {name}
        </div>
        {sub && <div className="truncate text-xs text-muted-foreground">{sub}</div>}
      </div>
    </div>
  );
}

/* ----------------------------- Sections ----------------------------- */

/** A card's heading row: title, a quiet line under it, and controls on the right. */
export function SectionHeading({ title, sub, children }: { title: string; sub?: string; children?: React.ReactNode }) {
  return (
    <div className="flex flex-wrap items-start justify-between gap-3 p-5 pb-0">
      <div className="min-w-0">
        <h2 className="font-display text-sm font-semibold tracking-tight">{title}</h2>
        {sub && <p className="mt-0.5 text-xs text-muted-foreground">{sub}</p>}
      </div>
      {children}
    </div>
  );
}

/** Placeholder blocks while a live report loads (still when motion is unwelcome). */
export function ReportSkeleton() {
  return (
    <div aria-hidden className="no-print grid grid-cols-1 gap-5 xl:grid-cols-3">
      <div className="h-[320px] rounded-2xl border border-border bg-muted/50 motion-safe:animate-pulse xl:col-span-2" />
      <div className="h-[320px] rounded-2xl border border-border bg-muted/50 motion-safe:animate-pulse" />
      <div className="h-[280px] rounded-2xl border border-border bg-muted/50 motion-safe:animate-pulse xl:col-span-3" />
    </div>
  );
}

/** Nothing was tracked in the period: say so, and offer a longer look back. */
export function NoTimeState({
  period,
  what = "No tracked time in this period",
  onWiden,
}: {
  period: ReportPeriod;
  what?: string;
  /** Show a longer period (omit when there is nothing longer to offer). */
  onWiden?: () => void;
}) {
  return (
    <EmptyState
      icon={CalendarClock}
      title={what}
      description={`Nothing was recorded for ${period.label}.`}
      action={onWiden ? { label: "Show the last 30 days", onClick: onWiden } : undefined}
    />
  );
}

/** A report the viewer's permissions don't cover (the server would refuse or scope it to them). */
export function AccessNotice({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <Card variant="quiet" className="flex flex-col items-center gap-3 px-6 py-14 text-center">
      <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-muted text-muted-foreground">
        <Lock aria-hidden className="h-6 w-6" />
      </div>
      <div className="max-w-md">
        <p className="font-display text-base font-semibold tracking-tight">{title}</p>
        <p className="mt-1.5 text-sm text-muted-foreground">{children}</p>
      </div>
    </Card>
  );
}

/** How the figures were measured, under the report. */
export function ReportFootnote({ children }: { children: React.ReactNode }) {
  return (
    <p className="flex gap-2 border-t border-border pt-4 text-xs leading-relaxed text-muted-foreground">
      <Info aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0" />
      <span>{children}</span>
    </p>
  );
}

/* ----------------------------- Charts ----------------------------- */

/** Axis labels: the muted text colour (≥ 4.5:1 on cards in both themes). */
const tickFill = "var(--muted-foreground)";

/** Width for a value axis from its longest tick label, so labels are never clipped. */
export function axisWidth(labels: readonly string[], fontSize: number): number {
  const longest = labels.reduce((m, l) => Math.max(m, l.length), 0);
  return Math.ceil(longest * fontSize * 0.62) + 10;
}

function TooltipCard({ title, rows }: { title: string; rows: [string, string, string?][] }) {
  return (
    <div className="min-w-[170px] rounded-xl border border-border bg-card px-3 py-2 text-xs text-card-foreground card-elev-lg">
      <div className="font-semibold">{title}</div>
      {rows.map(([label, value, swatch]) => (
        <div key={label} className="mt-1 flex items-center justify-between gap-4">
          <span className="flex items-center gap-1.5 text-muted-foreground">
            {swatch && <span className={cn("h-2 w-2 rounded-full", swatch)} />}
            {label}
          </span>
          <span className="font-semibold tabular-nums">{value}</span>
        </div>
      ))}
    </div>
  );
}

const SEGMENT_CLASS = {
  active: "bg-primary",
  other: "bg-primary/30",
  idle: "bg-warning",
} as const;

/** The legend of the active / other / idle split (the same three colours everywhere). */
export function ActivityLegend({ idle, other = true }: { idle: boolean; other?: boolean }) {
  return (
    <div aria-hidden className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
      <span className="flex items-center gap-1.5">
        <span className={cn("print-exact h-2.5 w-2.5 rounded-sm", SEGMENT_CLASS.active)} /> Active
      </span>
      {other && (
        <span className="flex items-center gap-1.5">
          <span className={cn("print-exact h-2.5 w-2.5 rounded-sm", SEGMENT_CLASS.other)} /> Other tracked
        </span>
      )}
      {idle && (
        <span className="flex items-center gap-1.5">
          <span className={cn("print-exact h-2.5 w-2.5 rounded-sm", SEGMENT_CLASS.idle)} /> Idle
        </span>
      )}
    </div>
  );
}

interface ChartDay extends DayBar {
  activeH: number;
  otherH: number;
  idleH: number;
  /** Which stacked part is on top (it gets the rounded corners). */
  top: "active" | "other" | "idle" | null;
}

/** A stacked part whose top corners are rounded only when it is the day's topmost part. */
function stackPart(part: "active" | "other" | "idle") {
  function StackPart(props: BarShapeProps) {
    const day = props.payload as ChartDay | undefined;
    // Drawn only inside a chart, so recharts has loaded.
    const { Rectangle } = loadedRecharts();
    return <Rectangle {...props} radius={day?.top === part ? [6, 6, 0, 0] : 0} />;
  }
  return StackPart;
}

const ActivePart = stackPart("active");
const OtherPart = stackPart("other");
const IdlePart = stackPart("idle");

/**
 * Hours per day of the period, each bar split into active (keyboard/mouse),
 * other tracked and idle time. The value axis is in hours with round steps
 * and sized to its labels; dense periods label days by date.
 */
export function DailyHoursChart({ bars, fmt, label }: { bars: DayBar[]; fmt: (minutes: number) => string; label: string }) {
  const recharts = useRecharts();
  const md = useIsMdUp();
  const reduced = usePrefersReducedMotion();
  const dense = bars.length > 14;
  const fontSize = md ? 12 : 10;
  const data: ChartDay[] = bars.map((b) => {
    const active = b.active ?? 0;
    const idle = b.idle ?? 0;
    return {
      ...b,
      activeH: active / 60,
      otherH: b.other / 60,
      idleH: idle / 60,
      top: idle > 0 ? "idle" : b.other > 0 ? "other" : active > 0 ? "active" : null,
    };
  });
  const axis = hourTicks(Math.max(0, ...bars.map((b) => b.minutes / 60)));
  const yWidth = axisWidth(axis.ticks.map((t) => `${t}h`), fontSize);
  if (!recharts) {
    return (
      <figure className="h-[220px] w-full min-w-0 sm:h-[270px]">
        <figcaption className="sr-only">{label}</figcaption>
        <ChartLoading />
      </figure>
    );
  }
  const { ResponsiveContainer, BarChart, Bar, CartesianGrid, XAxis, YAxis, Tooltip } = recharts;
  // A figure, not role="img": the chart itself stays keyboard-explorable (recharts' accessibility layer).
  return (
    <figure className="h-[220px] w-full min-w-0 sm:h-[270px]">
      <figcaption className="sr-only">{label}</figcaption>
      <ResponsiveContainer width="100%" height="100%">
        <BarChart data={data} margin={{ top: 10, right: 4, left: 0, bottom: 0 }} barCategoryGap={dense ? "16%" : "28%"}>
          <CartesianGrid vertical={false} stroke="var(--border)" strokeDasharray="3 4" />
          <XAxis
            dataKey="date"
            tickLine={false}
            axisLine={false}
            tick={{ fill: tickFill, fontSize }}
            tickFormatter={(d) => formatAxisDay(String(d), dense)}
            interval="preserveStartEnd"
            minTickGap={md ? 12 : 6}
          />
          <YAxis
            tickLine={false}
            axisLine={false}
            tick={{ fill: tickFill, fontSize }}
            width={yWidth}
            domain={[0, axis.top]}
            ticks={axis.ticks}
            tickFormatter={(v) => `${v}h`}
          />
          <Tooltip
            cursor={{ fill: "var(--muted)", radius: 6 }}
            content={({ active, payload }) => {
              const day = active && payload?.length ? (payload[0]?.payload as ChartDay | undefined) : undefined;
              if (!day) return null;
              const rows: [string, string, string?][] = [["Tracked", fmt(day.minutes)]];
              if (day.active !== null) rows.push(["Active", fmt(day.active), SEGMENT_CLASS.active]);
              if (day.idle !== null) rows.push(["Idle", fmt(day.idle), SEGMENT_CLASS.idle]);
              rows.push(["Activity", day.minutes > 0 && day.activity !== null ? `${day.activity}%` : "—"]);
              return <TooltipCard title={`${formatDayLabel(day.date, { year: true })}${day.weekend ? " · Weekend" : ""}`} rows={rows} />;
            }}
          />
          <Bar dataKey="activeH" name="Active" stackId="day" fill="var(--primary)" maxBarSize={44} isAnimationActive={!reduced} shape={ActivePart} />
          <Bar dataKey="otherH" name="Other tracked" stackId="day" fill="var(--primary)" fillOpacity={0.3} maxBarSize={44} isAnimationActive={!reduced} shape={OtherPart} />
          <Bar dataKey="idleH" name="Idle" stackId="day" fill="var(--warning)" maxBarSize={44} isAnimationActive={!reduced} shape={IdlePart} />
        </BarChart>
      </ResponsiveContainer>
    </figure>
  );
}

export interface RankedSegment {
  key: "active" | "other" | "idle";
  label: string;
  minutes: number;
}

export interface RankedItem {
  key: string;
  label: string;
  /** What the bar measures (its length, against the longest). */
  value: number;
  /** The value as shown ("7h 30m", "$120.00"). */
  display: string;
  /** Quieter text after it ("· 42%"). */
  detail?: string;
  /** A swatch before the label, and the bar's colour without segments. */
  color?: string;
  /** Marks after the label (an "Archived" badge…). */
  badge?: React.ReactNode;
  /** The bar split into active / other / idle parts. */
  segments?: RankedSegment[];
}

/**
 * A ranked list with proportional bars — names and values are text, never
 * squeezed into an axis, so it reads on a phone as well as on a desktop.
 */
export function RankedBars({ items, label, max }: { items: RankedItem[]; label: string; max?: number }) {
  const top = max ?? items.reduce((m, i) => Math.max(m, i.value), 0);
  return (
    <ul aria-label={label} className="space-y-3">
      {items.map((item) => {
        const parts = (item.segments ?? []).filter((s) => s.minutes > 0);
        const partTotal = parts.reduce((s, p) => s + p.minutes, 0);
        return (
          <li key={item.key}>
            <div className="flex items-baseline justify-between gap-3 text-sm">
              <span className="flex min-w-0 items-center gap-2 font-medium">
                {item.color && <span aria-hidden className="print-exact h-2.5 w-2.5 shrink-0 rounded-full" style={{ background: item.color }} />}
                <span className="truncate" title={item.label}>
                  {item.label}
                </span>
                {item.badge}
              </span>
              <span className="shrink-0 tabular-nums">
                <span className="font-semibold">{item.display}</span>
                {item.detail && <span className="text-muted-foreground"> {item.detail}</span>}
              </span>
            </div>
            <div aria-hidden className="print-exact mt-1.5 h-2 overflow-hidden rounded-full bg-muted">
              <div className="flex h-full overflow-hidden rounded-full" style={{ width: `${barPercent(item.value, top)}%` }}>
                {parts.length ? (
                  parts.map((p) => (
                    <span key={p.key} title={`${p.label}`} className={cn("print-exact h-full", SEGMENT_CLASS[p.key])} style={{ width: `${(p.minutes / partTotal) * 100}%` }} />
                  ))
                ) : (
                  <span className="print-exact h-full w-full bg-primary" style={item.color ? { background: item.color } : undefined} />
                )}
              </div>
            </div>
          </li>
        );
      })}
    </ul>
  );
}

export interface ShareSlice {
  key: string;
  label: string;
  minutes: number;
  color: string;
}

/**
 * A share of time as a donut, with every slice listed beside it (value and a
 * whole-percent share; the shares always add up to 100).
 */
export function ShareDonut({ slices, fmt, label }: { slices: ShareSlice[]; fmt: (minutes: number) => string; label: string }) {
  const recharts = useRecharts();
  const reduced = usePrefersReducedMotion();
  const total = slices.reduce((s, x) => s + x.minutes, 0);
  const shares = apportionPercent(slices.map((s) => s.minutes));
  const drawn = slices.filter((s) => s.minutes > 0).map((s) => ({ ...s, fill: s.color }));
  return (
    <div className="flex flex-col items-center gap-4 sm:flex-row sm:items-center sm:gap-6 xl:flex-col">
      <div className="relative h-[168px] w-[168px] shrink-0" aria-hidden>
        {total > 0 && !recharts ? (
          <ChartLoading className="rounded-full" />
        ) : total > 0 && recharts ? (
          // Decorative (the list beside it carries every value), so not a keyboard stop either.
          <recharts.PieChart width={168} height={168} accessibilityLayer={false}>
            <recharts.Pie
              data={drawn}
              dataKey="minutes"
              nameKey="label"
              innerRadius={56}
              outerRadius={80}
              paddingAngle={drawn.length > 1 ? 2 : 0}
              stroke="var(--card)"
              strokeWidth={2}
              isAnimationActive={!reduced}
            />
          </recharts.PieChart>
        ) : (
          <div className="h-full w-full rounded-full border-[24px] border-muted" />
        )}
        <div className="absolute inset-0 flex flex-col items-center justify-center">
          <span className="font-display text-lg font-bold tabular-nums">{fmt(total)}</span>
          <span className="text-[11px] text-muted-foreground">total</span>
        </div>
      </div>
      <ul aria-label={label} className="w-full min-w-0 space-y-2">
        {slices.map((s, i) => (
          <li key={s.key} className="flex items-center justify-between gap-3 text-sm">
            <span className="flex min-w-0 items-center gap-2">
              <span aria-hidden className="print-exact h-2.5 w-2.5 shrink-0 rounded-full" style={{ background: s.color }} />
              <span className="truncate">{s.label}</span>
            </span>
            <span className="shrink-0 tabular-nums">
              <span className="font-semibold">{fmt(s.minutes)}</span>
              <span className="text-muted-foreground"> · {shares[i]}%</span>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
