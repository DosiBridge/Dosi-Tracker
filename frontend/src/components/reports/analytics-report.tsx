"use client";

import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import type { LucideIcon } from "lucide-react";
import {
  Activity as ActivityIcon,
  CalendarCheck,
  CalendarClock,
  Clock3,
  Coffee,
  FolderKanban,
  Gauge,
  Keyboard,
  LayoutDashboard,
  MonitorSmartphone,
  RotateCw,
  Sheet,
  Sparkles,
  Sunrise,
  Sunset,
  Timer,
  TrendingUp,
  Users,
  Zap,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/input";
import { EmptyState } from "@/components/ui/empty-state";
import { useSession } from "@/components/session-provider";
import { getAllPagesApi, getApi } from "@/hooks/useApi";
import { useReportingZone } from "@/hooks/use-reporting-zone";
import { useStoredChoice } from "@/hooks/use-stored-choice";
import { activities as tenantActivities, projectById, userById } from "@/lib/tenant-data";
import { referenceNow } from "@/lib/live-session";
import { ALL, memberName } from "@/lib/report-math";
import { MAX_REPORT_RANGE_DAYS, activitiesRoute, reportingRoute } from "@/lib/api-contract";
import { addZonedDays, formatDayRange, startOfZonedDay, timeZoneLabel, zonedDayKey } from "@/lib/time-zone";
import { downloadCSV } from "@/lib/export";
import { cn } from "@/lib/utils";
import {
  HOURS_FORMATS,
  activityLevel,
  activityLevelLabel,
  changeVs,
  cleanMinutes,
  comparisonFor,
  comparisonText,
  decimalHours,
  formatActivity,
  formatHours,
  idleShare,
  optionalPercent,
  parseDailySeries,
  parseSummary,
  parseUserDailySeries,
  projectLabel,
  resolvePeriod,
  scopeLabel,
  selectionTitle,
  type HoursFormat,
  type PeriodPreset,
  type PeriodSelection,
  type ReportAggregates,
} from "@/lib/work-report";
import {
  BLOCK_PAGE,
  MAX_BLOCKS,
  OTHER,
  WEEKDAYS,
  activityHistogram,
  agentVersions,
  aggregateBlocks,
  appStats,
  appUsageFromBlocks,
  blocksFromApi,
  blocksStartingIn,
  clockLabel,
  coversDay,
  cumulative,
  dayLabel,
  dayStats,
  dayTimeline,
  devicesFromBlocks,
  focusStretches,
  gapKind,
  hourCellsFromBlocks,
  hourProfile,
  hourRangeLabel,
  hourTotals,
  localSpanLabel,
  longDayLabel,
  meanOf,
  memberClock,
  memberProjectFlow,
  memberProjectsFromBlocks,
  memberStats,
  movingAverage,
  paceTarget,
  parseDevices,
  parseHourly,
  platformMix,
  projectDays,
  radarRows,
  rhythmMatrix,
  stretchHistogram,
  summarize,
  toBlocks,
  unmeasuredMinutes,
  withBlockBounds,
  workWindows,
  zoneCalendar,
  type AnalyticsSummary,
  type Block,
  type DayStat,
  type DeviceRow,
  type GapKind,
  type HourCell,
  type MemberClock,
  type MemberStat,
  type WorkWindow,
  type ZoneCalendar,
} from "@/lib/analytics";
import { ExportMenu, LiveReportNotice, ReportShell, useLiveReport, useReportAccess } from "./report-shell";
import { HOURS_FORMAT_KEY, PeriodToolbar, Segmented, type CustomRange, type ToolbarPeriod } from "./report-controls";
import {
  ActivityDistributionChart,
  AppsChart,
  ChartLegend,
  DayClock,
  DayTimelineChart,
  FlowDiagram,
  FocusChart,
  HourActivityChart,
  IdleChart,
  KpiCard,
  LegendItem,
  OTHER_COLOR,
  PaceChart,
  PlatformMix,
  ProjectStackChart,
  ProjectTreemap,
  RhythmHeatmap,
  SERIES_COLORS,
  SampleBadge,
  Sparkline,
  TeamRadar,
  TeamScatter,
  TimelineLegend,
  TrendChart,
  WorkWindowsChart,
  scoreFill,
  type Fmt,
  type FlowNodeData,
  type TreemapItem,
} from "./analytics-charts";

/* ============================================================================
 * WORK ANALYTICS — the deep-dive report: how much, when, how focused, on what
 * and by whom, as charts and diagrams across five views.
 *
 * Every figure that adds up — hours, activity, idle time, sessions, days,
 * members, projects, who works on what, the hour-of-week rhythm, devices —
 * comes from the reporting endpoints, in the reporting zone. Charts of how the
 * time is distributed (focus stretches, activity levels, apps, a day's
 * timeline) need the tracked blocks, which a live session pages in up to
 * MAX_BLOCKS, newest first; past that, those charts carry a "Sample" mark.
 * The 7-day average is seeded with the days before the period, and the change
 * badges compare with a period that never overlaps this one.
 *
 * Demo sessions compute the same figures from the seeded activities by the
 * backend's rules, without the network. Like every report, a live session
 * never shows demo rows.
 * ========================================================================== */

const FORMAT_KEYS = HOURS_FORMATS.map((f) => f.key);
/** Days before the period that seed its 7-day average. */
const SEED_DAYS = 6;
const EMPTY_BLOCKS: Block[] = [];

const EMPTY_AGGREGATES: ReportAggregates = {
  totals: { minutes: 0, sessions: 0, activity: null, idleMinutes: null, keys: 0, clicks: 0 },
  daily: [],
  members: [],
  projects: [],
  memberDays: [],
  memberProjects: null,
};

type TabKey = "overview" | "rhythm" | "focus" | "projects" | "team";

const TABS: { key: TabKey; label: string; icon: LucideIcon }[] = [
  { key: "overview", label: "Overview", icon: LayoutDashboard },
  { key: "rhythm", label: "Work rhythm", icon: Clock3 },
  { key: "focus", label: "Focus & activity", icon: Zap },
  { key: "projects", label: "Projects & apps", icon: FolderKanban },
  { key: "team", label: "Team", icon: Users },
];

interface PeriodData {
  aggregates: ReportAggregates;
  /** Minutes on each of the days just before the period (oldest first): the 7-day average's seed. */
  seed: number[];
  /** Weekday × hour cells; null when the server has no /reporting/hourly (the blocks stand in). */
  hourly: HourCell[] | null;
  /** Devices; null when the server has no /reporting/devices. */
  devices: DeviceRow[] | null;
  /** The comparison period's totals; null when they couldn't be read. */
  previous: { minutes: number; activity: number | null } | null;
}

interface LiveBlocks {
  blocks: Block[];
  totalCount: number;
  truncated: boolean;
}

/** Where the blocks behind the distribution charts stand. */
type BlocksState = "loading" | "error" | "ready";

/** Re-render with every view expanded while the browser prints. */
function usePrinting(): boolean {
  const [printing, setPrinting] = useState(false);
  useEffect(() => {
    const on = () => setPrinting(true);
    const off = () => setPrinting(false);
    window.addEventListener("beforeprint", on);
    window.addEventListener("afterprint", off);
    return () => {
      window.removeEventListener("beforeprint", on);
      window.removeEventListener("afterprint", off);
    };
  }, []);
  return printing;
}

/** A project's colour: the roster's, else a stable pick by id (the same in every chart). */
function projectColorOf(projectId: string): string {
  const known = projectById(projectId)?.color;
  if (known) return known;
  let hash = 0;
  for (let i = 0; i < projectId.length; i++) hash = (hash * 31 + projectId.charCodeAt(i)) >>> 0;
  return SERIES_COLORS[hash % SERIES_COLORS.length];
}

export function AnalyticsReport() {
  const session = useSession();
  const { isLive } = session;
  const access = useReportAccess();
  // Without Tracker.Activities.ViewAll the server reports only the viewer, so the member filter offers just them.
  const liveWorkerId = isLive && !access.viewAll ? session.user.id : null;
  const zone = useReportingZone(isLive);
  const timeZone = zone.timeZone;
  const calendar = zoneCalendar(timeZone);

  const [anchor] = useState(() => referenceNow());
  const [preset, setPreset] = useState<ToolbarPeriod>("7d");
  const [offset, setOffset] = useState(0);
  const [range, setRange] = useState<CustomRange | null>(null);
  const [projectId, setProjectId] = useState(ALL);
  const [memberId, setMemberId] = useState(ALL);
  const [format, setFormat] = useStoredChoice<HoursFormat>(HOURS_FORMAT_KEY, FORMAT_KEYS, "duration");
  const [tab, setTab] = useState<TabKey>("overview");
  const printing = usePrinting();

  const fmt = useCallback<Fmt>((minutes) => formatHours(minutes, format), [format]);
  const selection = useMemo<PeriodSelection>(
    () =>
      preset === "custom" && range
        ? { kind: "custom", from: range.from, to: range.to }
        : { kind: "preset", preset: preset === "custom" ? "7d" : preset, offset },
    [preset, range, offset],
  );
  const period = useMemo(() => resolvePeriod(selection, anchor, timeZone), [selection, anchor, timeZone]);
  const compare = useMemo(() => comparisonFor(selection, anchor, timeZone, period), [selection, anchor, timeZone, period]);
  // The days just before the period that seed its 7-day average (never past the server's longest range).
  const seedKeys = useMemo(() => {
    const count = Math.min(SEED_DAYS, Math.max(0, MAX_REPORT_RANGE_DAYS - period.days.length));
    return Array.from({ length: count }, (_, i) => addZonedDays(period.fromKey, i - count));
  }, [period]);

  const loadLive = useCallback(async (): Promise<PeriodData> => {
    const query = { from: period.from, to: period.to, projectId, userId: memberId, timeZone };
    const seedFrom = seedKeys.length ? startOfZonedDay(seedKeys[0], timeZone) : period.from;
    const [summary, series, userDays, hourly, devices, previous] = await Promise.all([
      getApi(reportingRoute("summary", query)),
      getApi(reportingRoute("daily-series", { ...query, from: seedFrom })),
      getApi(reportingRoute("user-daily-series", query)).catch(() => null),
      getApi(reportingRoute("hourly", query)).catch(() => null),
      getApi(reportingRoute("devices", query)).catch(() => null),
      compare.days.length
        ? getApi(reportingRoute("summary", { ...query, from: compare.from, to: compare.to })).catch(() => null)
        : Promise.resolve(null),
    ]);
    const daily = parseDailySeries(series);
    const seedMinutes = new Map(daily.map((d) => [d.day, d.minutes]));
    const prev = previous && typeof previous === "object" ? (previous as { totalTrackedMinutes?: unknown; averageProductivity?: unknown }) : null;
    return {
      aggregates: { ...parseSummary(summary), daily: daily.filter((d) => d.day >= period.fromKey), memberDays: parseUserDailySeries(userDays) },
      seed: seedKeys.map((day) => seedMinutes.get(day) ?? 0),
      hourly: Array.isArray(hourly) ? parseHourly(hourly) : null,
      devices: Array.isArray(devices) ? parseDevices(devices) : null,
      previous: prev ? { minutes: cleanMinutes(prev.totalTrackedMinutes), activity: optionalPercent(prev.averageProductivity) } : null,
    };
  }, [period, compare, seedKeys, projectId, memberId, timeZone]);

  // The blocks behind the distribution charts: read alongside the totals, which never wait for them.
  const loadBlocks = useCallback(async (): Promise<LiveBlocks> => {
    // Pinned to the moment the read starts, so a block uploaded while it pages can't shift the pages.
    const to = new Date(Math.min(period.to.getTime(), Date.now()));
    const page = await getAllPagesApi<unknown>(activitiesRoute({ from: period.from, to, projectId, userId: memberId }), {
      pageSize: BLOCK_PAGE,
      maxItems: MAX_BLOCKS,
    });
    return { blocks: blocksFromApi(page.items), totalCount: page.totalCount, truncated: page.truncated };
  }, [period, projectId, memberId]);

  const live = useLiveReport(isLive && zone.ready, loadLive);
  const blockLoad = useLiveReport(isLive && zone.ready, loadBlocks);

  const demo = useMemo(() => {
    if (isLive) return null;
    const scoped = toBlocks(
      tenantActivities.filter((a) => (projectId === ALL || a.projectId === projectId) && (memberId === ALL || a.userId === memberId)),
    );
    const blocks = blocksStartingIn(scoped, period.from.getTime(), period.to.getTime());
    const seedFrom = seedKeys.length ? startOfZonedDay(seedKeys[0], timeZone).getTime() : period.from.getTime();
    const seed = aggregateBlocks(blocksStartingIn(scoped, seedFrom, period.from.getTime()), calendar).daily;
    const previous = compare.days.length
      ? aggregateBlocks(blocksStartingIn(scoped, compare.from.getTime(), compare.to.getTime()), calendar).totals
      : null;
    const data: PeriodData = {
      aggregates: aggregateBlocks(blocks, calendar),
      seed: seedKeys.map((day) => seed.find((d) => d.day === day)?.minutes ?? 0),
      hourly: hourCellsFromBlocks(blocks, calendar),
      devices: devicesFromBlocks(blocks),
      previous: previous ? { minutes: previous.minutes, activity: previous.activity } : null,
    };
    return { data, blocks };
  }, [isLive, projectId, memberId, period, compare, seedKeys, timeZone, calendar]);

  const loaded = isLive ? live.data : (demo?.data ?? null);
  const loading = isLive && (!zone.ready || live.loading);
  const pending = isLive && (loading || !!live.error || !live.data);
  const agg = loaded?.aggregates ?? EMPTY_AGGREGATES;
  const blocks = isLive ? (blockLoad.data?.blocks ?? EMPTY_BLOCKS) : (demo?.blocks ?? EMPTY_BLOCKS);
  const blocksState: BlocksState = !isLive || blockLoad.data ? "ready" : blockLoad.error ? "error" : "loading";
  const truncated = isLive && !!blockLoad.data?.truncated;
  const sampleNote = truncated
    ? `From the newest ${MAX_BLOCKS.toLocaleString("en")} of ${(blockLoad.data?.totalCount ?? 0).toLocaleString("en")} tracked blocks in this period`
    : null;

  // Hour-of-week cells from /reporting/hourly; a server without it falls back to the loaded blocks.
  const cells = useMemo(() => loaded?.hourly ?? hourCellsFromBlocks(blocks, calendar), [loaded, blocks, calendar]);
  const cellsSampled = !loaded?.hourly && truncated;
  const memberDays = useMemo(() => withBlockBounds(agg.memberDays, blocks, calendar), [agg, blocks, calendar]);
  const full = useMemo<ReportAggregates>(() => ({ ...agg, memberDays }), [agg, memberDays]);
  const clock = useMemo(
    () => memberClock(blocks, calendar, isLive ? undefined : (id) => userById(id)?.timezone),
    [blocks, calendar, isLive],
  );
  const daily = useMemo(() => dayStats(full, period.days), [full, period.days]);
  const summary = useMemo(() => summarize(full, period.days, blocks, cells), [full, period.days, blocks, cells]);
  const members = useMemo(() => memberStats(full, blocks, period.days, clock), [full, blocks, period.days, clock]);
  const windows = useMemo(() => workWindows(memberDays, period.days, clock), [memberDays, period.days, clock]);
  const team = members.length > 1;

  const serverNames = useMemo(() => new Map(agg.members.map((m) => [m.userId, m.name])), [agg]);
  const projectTitles = useMemo(() => new Map(agg.projects.map((p) => [p.projectId, p.title])), [agg]);
  const nameOf = useCallback(
    (id: string) => {
      const hint = serverNames.get(id);
      return memberName(id, userById(id) ?? (hint ? { name: hint } : null));
    },
    [serverNames],
  );
  const projectNameOf = useCallback((id: string) => projectLabel(id, projectById(id) ?? { title: projectTitles.get(id) ?? null }), [projectTitles]);
  const memberColor = useMemo(() => {
    const map = new Map<string, string>();
    members.forEach((m, i) => map.set(m.userId, SERIES_COLORS[i % SERIES_COLORS.length]));
    return (id: string) => map.get(id) ?? OTHER_COLOR;
  }, [members]);

  const comparison = comparisonText(selection, period.days.length);
  const previous = loaded?.previous ?? null;
  const hoursDelta = pending ? null : changeVs(summary.totalMinutes, previous?.minutes);
  const activityDelta =
    pending || !previous || previous.activity === null || previous.activity <= 0 || summary.activity === null || summary.totalMinutes <= 0
      ? null
      : { percent: Math.round(((summary.activity - previous.activity) / previous.activity) * 100) };

  const scope = scopeLabel(
    projectId === ALL ? null : projectNameOf(projectId),
    memberId !== ALL ? nameOf(memberId) : liveWorkerId ? memberName(session.user.id, userById(session.user.id) ?? session.user) : null,
  );
  const periodText =
    period.days.length > 1
      ? `${longDayLabel(period.days[0])} – ${longDayLabel(period.days[period.days.length - 1])}`
      : period.days[0]
        ? longDayLabel(period.days[0])
        : "—";
  const zoneText = timeZoneLabel(timeZone, anchor);
  const hasTime = summary.totalMinutes > 0;

  function choosePreset(next: PeriodPreset) {
    setPreset(next);
    setOffset(0);
  }
  function step(by: number) {
    setOffset((o) => Math.min(0, o + by));
  }
  function applyRange(next: CustomRange) {
    setRange(next);
    setPreset("custom");
  }
  const tabs = TABS.filter((t) => t.key !== "team" || team);
  const activeTab: TabKey = tabs.some((t) => t.key === tab) ? tab : "overview";

  const fileBase = `work-analytics-${period.days[0] ?? "period"}`;
  const profile = useMemo(() => hourProfile(blocks, calendar), [blocks, calendar]);
  const exports = [
    {
      label: "Hours by hour of day (CSV)",
      icon: Sheet,
      onSelect: () =>
        downloadCSV(
          `${fileBase}-hours`,
          ["Hour", "Hours (decimal)", "Activity %", "Key presses", "Clicks"],
          hourTotals(cells).map((h) => {
            const p = profile[h.hour];
            return [hourRangeLabel(h.hour), decimalHours(h.minutes), h.minutes > 0 && p.activity !== null ? p.activity : "", p.keys, p.clicks];
          }),
        ),
    },
    ...(team
      ? [
          {
            label: "Members (CSV)",
            icon: Sheet,
            onSelect: () =>
              downloadCSV(
                `${fileBase}-members`,
                [
                  "Member",
                  "Hours (decimal)",
                  "Idle (decimal)",
                  "Activity %",
                  "Sessions",
                  "Days active",
                  "Typical start (local)",
                  "Typical finish (local)",
                  "Average stretch (min)",
                  "Longest stretch (min)",
                  "Input per hour",
                  "Consistency %",
                ],
                members.map((m) => [
                  nameOf(m.userId),
                  decimalHours(m.minutes),
                  m.idleMinutes === null ? "" : decimalHours(m.idleMinutes),
                  m.activity ?? "",
                  m.sessions,
                  m.daysActive,
                  m.typicalStart === null ? "" : clockLabel(m.typicalStart),
                  m.typicalEnd === null ? "" : clockLabel(m.typicalEnd),
                  m.avgStretch === null ? "" : Math.round(m.avgStretch),
                  m.longestStretch === null ? "" : Math.round(m.longestStretch),
                  m.inputPerHour ?? "",
                  m.consistency ?? "",
                ]),
              ),
          },
        ]
      : []),
  ];
  function exportDaily() {
    downloadCSV(
      `${fileBase}-daily`,
      ["Date", "Hours (decimal)", "Activity %", "Idle (decimal)", "Sessions", "Members", "Key presses", "Clicks"],
      daily.map((d) => [
        d.day,
        decimalHours(d.minutes),
        d.minutes > 0 && d.activity !== null ? Math.round(d.activity) : "",
        d.idleMinutes === null ? "" : decimalHours(d.idleMinutes),
        d.sessions,
        d.members,
        d.keys,
        d.clicks,
      ]),
    );
  }

  const shared = { fmt, blocksState, sampleNote, onRetryBlocks: blockLoad.retry };
  const views: Record<TabKey, React.ReactNode> = hasTime
    ? {
        overview: (
          <OverviewView
            daily={daily}
            seed={loaded?.seed ?? []}
            summary={summary}
            members={members}
            agg={full}
            devices={loaded?.devices ?? null}
            projectNameOf={projectNameOf}
            fmt={fmt}
          />
        ),
        rhythm: (
          <RhythmView
            {...shared}
            cells={cells}
            cellsFromBlocks={!loaded?.hourly}
            cellsSampled={cellsSampled}
            windows={windows}
            daily={daily}
            blocks={blocks}
            truncated={truncated}
            memberDays={memberDays}
            calendar={calendar}
            clock={clock}
            zoneText={zoneText}
            nameOf={nameOf}
            projectNameOf={projectNameOf}
          />
        ),
        focus: <FocusView {...shared} blocks={blocks} profile={profile} summary={summary} />,
        projects: (
          <ProjectsView
            {...shared}
            agg={full}
            blocks={blocks}
            days={period.days}
            calendar={calendar}
            nameOf={nameOf}
            projectNameOf={projectNameOf}
          />
        ),
        team: <TeamView {...shared} members={members} days={period.days} nameOf={nameOf} memberColor={memberColor} />,
      }
    : { overview: null, rhythm: null, focus: null, projects: null, team: null };

  return (
    <ReportShell
      title="Work Analytics"
      description="When, how and on what your team works — trends, work rhythm, focus, projects and people, in charts."
      actions={
        <ExportMenu
          onExportCSV={exportDaily}
          csvLabel="Daily figures (CSV)"
          extraItems={hasTime ? exports : []}
          disabled={pending || !hasTime}
          disabledReason={pending ? "Export is available once the report has loaded" : "Nothing was tracked in this period"}
        />
      }
    >
      <PeriodToolbar
        preset={preset}
        onPreset={choosePreset}
        title={selectionTitle(selection, anchor, timeZone)}
        offset={selection.kind === "preset" ? selection.offset : 0}
        onStep={step}
        projectId={projectId}
        onProject={setProjectId}
        memberId={memberId}
        onMember={setMemberId}
        liveWorkerId={liveWorkerId}
        extra={<Segmented label="Hours format" options={HOURS_FORMATS} value={format} onChange={setFormat} />}
        allowCustom
        range={range}
        onRange={applyRange}
        suggestedRange={{ from: period.fromKey, to: period.toKey }}
        today={zonedDayKey(anchor, timeZone)}
        timeZone={timeZone}
        spanLabel={formatDayRange(period.fromKey, period.toKey)}
      />

      {isLive && (
        <div className="no-print space-y-2">
          <LiveReportNotice loading={loading} error={live.error} onRetry={live.retry} />
          {!pending && hasTime && blocksState === "loading" && (
            <p role="status" className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">
              Loading the activity blocks behind the focus, app and timeline charts…
            </p>
          )}
        </div>
      )}
      {!pending && truncated && hasTime && (
        <p role="status" className="no-print rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-sm text-warning-on-tint">
          This period has {(blockLoad.data?.totalCount ?? 0).toLocaleString("en")} tracked blocks. Totals, days, members, projects and
          the work rhythm cover all of them; charts marked “Sample” use the newest {MAX_BLOCKS.toLocaleString("en")}.
        </p>
      )}

      <section aria-labelledby="analytics-period" className="analytics relative overflow-hidden rounded-3xl border border-border bg-card p-5 card-elev sm:p-6">
        <div
          aria-hidden
          className="print-exact pointer-events-none absolute inset-0 bg-[radial-gradient(70%_120%_at_0%_0%,color-mix(in_srgb,#7c3aed_12%,transparent)_0%,transparent_60%),radial-gradient(60%_120%_at_100%_0%,color-mix(in_srgb,var(--primary)_14%,transparent)_0%,transparent_60%)]"
        />
        <div className="relative flex flex-wrap items-end justify-between gap-4">
          <div className="min-w-0">
            <p className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.16em] text-primary-strong">
              <Sparkles aria-hidden className="h-3.5 w-3.5" /> Work analytics
            </p>
            <h2 id="analytics-period" className="mt-2 font-display text-2xl font-bold tracking-tight text-ink sm:text-[1.75rem]">
              {periodText}
            </h2>
            <p className="mt-1 text-sm text-muted-foreground">
              {scope} · days and hours in {zoneText}
            </p>
          </div>
          {!pending && hasTime && <Highlights summary={summary} windows={windows} fmt={fmt} />}
        </div>
      </section>

      <dl aria-label="Key figures" className="stagger grid grid-cols-2 gap-3 sm:gap-4 lg:grid-cols-3 xl:grid-cols-6">
        <KpiCard
          label="Hours tracked"
          icon={Clock3}
          accent="blue"
          value={pending ? 0 : summary.totalMinutes}
          format={(v) => (pending ? "—" : fmt(v))}
          delta={hoursDelta ? { percent: hoursDelta.percent, label: comparison } : null}
          spark={pending ? undefined : daily.map((d) => d.minutes)}
        />
        <KpiCard
          label="Average activity"
          icon={Gauge}
          accent="green"
          value={pending || summary.activity === null ? 0 : summary.activity}
          format={(v) => (pending || !hasTime || summary.activity === null ? "—" : `${Math.round(v)}%`)}
          sub={!pending && hasTime && summary.activity === null ? "Not measured" : undefined}
          delta={activityDelta ? { percent: activityDelta.percent, label: comparison } : null}
          spark={pending ? undefined : daily.filter((d) => d.activity !== null).map((d) => d.activity as number)}
        />
        <KpiCard
          label="Idle time"
          icon={Coffee}
          accent="amber"
          value={pending || summary.idleMinutes === null ? 0 : summary.idleMinutes}
          format={(v) => (pending || !hasTime || summary.idleMinutes === null ? "—" : fmt(v))}
          sub={
            pending || !hasTime
              ? undefined
              : summary.idleMinutes === null
                ? "Not measured"
                : `${idleShare(summary.idleMinutes, summary.totalMinutes)}% of tracked time`
          }
          spark={pending || summary.idleMinutes === null ? undefined : daily.map((d) => d.idleMinutes ?? 0)}
        />
        <KpiCard
          label="Days worked"
          icon={CalendarCheck}
          accent="violet"
          value={pending ? 0 : summary.daysWorked}
          format={(v) => (pending ? "—" : String(Math.round(v)))}
          sub={pending ? undefined : `of ${period.days.length} days`}
        />
        <KpiCard
          label={team ? "Active members" : "Tracked sessions"}
          icon={team ? Users : ActivityIcon}
          accent="cyan"
          value={pending ? 0 : team ? summary.members : summary.sessions}
          format={(v) => (pending ? "—" : String(Math.round(v)))}
          sub={pending ? undefined : team ? `${summary.sessions} sessions` : "blocks recorded"}
          spark={pending || team ? undefined : daily.map((d) => d.sessions)}
        />
        <KpiCard
          label="Focus stretch"
          icon={Timer}
          accent="pink"
          value={pending || summary.avgStretch === null ? 0 : summary.avgStretch}
          format={(v) => (pending || !hasTime || summary.avgStretch === null ? "—" : fmt(v))}
          sub={
            pending || !hasTime
              ? undefined
              : blocksState === "loading"
                ? "Loading…"
                : blocksState === "error"
                  ? "Couldn't load the blocks"
                  : summary.longestStretch === null
                    ? undefined
                    : `average · longest ${fmt(summary.longestStretch)}`
          }
          badge={sampleNote && !pending && hasTime ? <SampleBadge detail={sampleNote} /> : undefined}
        />
      </dl>

      {pending ? (
        loading ? <AnalyticsSkeleton /> : null
      ) : !hasTime ? (
        <EmptyState
          icon={CalendarClock}
          title="No tracked time in this period"
          description={`Nothing was tracked in ${periodText} (${scope}).`}
          action={
            selection.kind === "preset" && selection.preset === "30d"
              ? { label: "Show the previous 30 days", onClick: () => step(-1) }
              : { label: "Show the last 30 days", onClick: () => choosePreset("30d") }
          }
        />
      ) : (
        <div>
          <Tabs tabs={tabs} active={activeTab} onChange={setTab} />
          {tabs.map((t) =>
            printing || t.key === activeTab ? (
              <div
                key={t.key}
                role="tabpanel"
                id={`analytics-panel-${t.key}`}
                aria-labelledby={`analytics-tab-${t.key}`}
                className="mt-5 space-y-5 motion-safe:animate-fade-in print:mt-6"
              >
                <h2 className="hidden font-display text-lg font-bold print:block">{t.label}</h2>
                {views[t.key]}
              </div>
            ) : null,
          )}
        </div>
      )}

      <p className="flex gap-2 border-t border-border pt-4 text-xs leading-relaxed text-muted-foreground">
        <Sparkles aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0" />
        <span>
          Days and hours are read in {zoneText}; each block counts on the day it started. Start and finish times are each
          member&apos;s own local time when their desktop app reports its time zone. Activity % is the share of tracked minutes
          with keyboard or mouse input (“—” where input isn&apos;t measured); idle time is tracked time without any. A focus
          stretch is back-to-back blocks with gaps of at most 5 minutes. Consistency compares a member&apos;s hours across the
          days they worked.
        </span>
      </p>
    </ReportShell>
  );
}

/* ----------------------------- Tabs ----------------------------- */

function Tabs({ tabs, active, onChange }: { tabs: { key: TabKey; label: string; icon: LucideIcon }[]; active: TabKey; onChange: (t: TabKey) => void }) {
  const refs = useRef<Record<string, HTMLButtonElement | null>>({});
  const listRef = useRef<HTMLDivElement>(null);
  // Fades on the edges that have more tabs beyond them (phones), so the strip reads as scrollable.
  const [edges, setEdges] = useState({ start: false, end: false });
  const measure = useCallback(() => {
    const el = listRef.current;
    if (!el) return;
    const start = el.scrollLeft > 2;
    const end = el.scrollLeft + el.clientWidth < el.scrollWidth - 2;
    setEdges((e) => (e.start === start && e.end === end ? e : { start, end }));
  }, []);
  useEffect(() => {
    const el = listRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    // The observer reports the size as soon as it starts observing, which measures the strip once.
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [measure]);
  const move = (to: number) => {
    const next = tabs[(to + tabs.length) % tabs.length];
    onChange(next.key);
    refs.current[next.key]?.focus();
  };
  return (
    <div className="no-print relative">
      <div
        ref={listRef}
        role="tablist"
        aria-label="Analytics views"
        onScroll={measure}
        className="flex gap-1 overflow-x-auto rounded-2xl border border-border bg-card p-1.5 card-elev [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      >
        {tabs.map((t, i) => {
          const Icon = t.icon;
          const selected = t.key === active;
          return (
            <button
              key={t.key}
              ref={(el) => {
                refs.current[t.key] = el;
              }}
              type="button"
              role="tab"
              id={`analytics-tab-${t.key}`}
              aria-selected={selected}
              aria-controls={`analytics-panel-${t.key}`}
              tabIndex={selected ? 0 : -1}
              onClick={() => onChange(t.key)}
              onKeyDown={(e) => {
                if (e.key === "ArrowRight") move(i + 1);
                else if (e.key === "ArrowLeft") move(i - 1);
                else if (e.key === "Home") move(0);
                else if (e.key === "End") move(tabs.length - 1);
                else return;
                e.preventDefault();
              }}
              className={cn(
                "relative inline-flex h-10 shrink-0 items-center gap-2 whitespace-nowrap rounded-xl px-4 text-sm font-medium transition-all motion-reduce:transition-none",
                "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                // Dark mode drops the gradient: neither white nor dark ink clears 4.5:1 across teal-to-violet.
                selected
                  ? "bg-gradient-to-r from-primary to-[#6d5efc] text-white shadow-md shadow-primary/20 dark:bg-primary dark:bg-none dark:text-primary-foreground"
                  : "text-muted-foreground hover:bg-muted hover:text-foreground",
              )}
            >
              <Icon aria-hidden className="h-4 w-4" />
              {t.label}
            </button>
          );
        })}
      </div>
      {edges.start && <span aria-hidden className="pointer-events-none absolute inset-y-px left-px w-8 rounded-l-2xl bg-gradient-to-r from-card to-transparent" />}
      {edges.end && <span aria-hidden className="pointer-events-none absolute inset-y-px right-px w-8 rounded-r-2xl bg-gradient-to-l from-card to-transparent" />}
    </div>
  );
}

/* ----------------------------- Shared bits ----------------------------- */

function ChartCard({
  title,
  subtitle,
  legend,
  className,
  sampleNote,
  children,
}: {
  title: string;
  subtitle?: string;
  legend?: React.ReactNode;
  className?: string;
  /** Set when the chart is drawn from a sample of the period's blocks. */
  sampleNote?: string | null;
  children: React.ReactNode;
}) {
  return (
    <Card className={cn("wr-keep min-w-0", className)}>
      <CardHeader>
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <CardTitle>{title}</CardTitle>
            {sampleNote && <SampleBadge detail={sampleNote} />}
          </div>
          {subtitle && <p className="mt-0.5 text-xs text-muted-foreground">{subtitle}</p>}
        </div>
        {legend}
      </CardHeader>
      <CardContent>{children}</CardContent>
    </Card>
  );
}

/** A chart that needs the period's blocks: a placeholder while they load, an honest note (and Retry) if they failed. */
function BlocksGate({ state, onRetry, height = 240, children }: { state: BlocksState; onRetry: () => void; height?: number; children: React.ReactNode }) {
  if (state === "loading") {
    return <div aria-hidden className="rounded-xl bg-muted/50 motion-safe:animate-pulse" style={{ height }} />;
  }
  if (state === "error") {
    return (
      <div className="flex flex-col items-start gap-3 text-sm text-muted-foreground" style={{ minHeight: Math.min(height, 120) }}>
        <span>The activity blocks behind this chart couldn&apos;t be loaded. The totals elsewhere on this page are complete.</span>
        <Button variant="outline" size="sm" onClick={() => void onRetry()} className="no-print">
          <RotateCw className="h-3.5 w-3.5" /> Retry
        </Button>
      </div>
    );
  }
  return <>{children}</>;
}

function Stat({ icon: Icon, label, value, tone = "text-primary" }: { icon: LucideIcon; label: string; value: string; tone?: string }) {
  return (
    <div className="flex items-center gap-3 rounded-xl border border-border bg-muted/30 px-3 py-2.5">
      <Icon aria-hidden className={cn("h-4 w-4 shrink-0", tone)} />
      <div className="min-w-0">
        <p className="text-[11px] text-muted-foreground">{label}</p>
        <p className="truncate text-sm font-semibold tabular-nums">{value}</p>
      </div>
    </div>
  );
}

function Highlights({ summary, windows, fmt }: { summary: AnalyticsSummary; windows: WorkWindow[]; fmt: Fmt }) {
  const start = meanOf(windows.map((w) => w.start));
  const end = meanOf(windows.map((w) => w.end));
  const chips: { icon: LucideIcon; text: string }[] = [];
  if (summary.peakHour !== null) chips.push({ icon: TrendingUp, text: `Peak ${hourRangeLabel(summary.peakHour)}` });
  if (summary.busiestWeekday !== null) chips.push({ icon: CalendarCheck, text: `Busiest on ${WEEKDAYS[summary.busiestWeekday]}` });
  if (start !== null && end !== null) chips.push({ icon: Sunrise, text: `Typical day ${clockLabel(start)}–${clockLabel(end)}` });
  if (summary.longestStretch !== null && summary.longestStretch > 0) chips.push({ icon: Timer, text: `Longest stretch ${fmt(summary.longestStretch)}` });
  return (
    <ul aria-label="Highlights" className="flex max-w-full flex-wrap gap-2">
      {chips.map((c) => (
        <li key={c.text} className="inline-flex items-center gap-1.5 rounded-full border border-border bg-card/80 px-3 py-1.5 text-xs font-medium backdrop-blur">
          <c.icon aria-hidden className="h-3.5 w-3.5 text-primary" />
          {c.text}
        </li>
      ))}
    </ul>
  );
}

function AnalyticsSkeleton() {
  return (
    <div aria-hidden className="no-print space-y-5">
      <div className="h-14 rounded-2xl border border-border bg-muted/50 motion-safe:animate-pulse" />
      <div className="grid grid-cols-1 gap-5 xl:grid-cols-3">
        <div className="h-[340px] rounded-2xl border border-border bg-muted/50 motion-safe:animate-pulse xl:col-span-2" />
        <div className="h-[340px] rounded-2xl border border-border bg-muted/50 motion-safe:animate-pulse" />
      </div>
    </div>
  );
}

interface BlocksProps {
  fmt: Fmt;
  blocksState: BlocksState;
  sampleNote: string | null;
  onRetryBlocks: () => void;
}

/* ----------------------------- Overview ----------------------------- */

function OverviewView({
  daily,
  seed,
  summary,
  members,
  agg,
  devices,
  projectNameOf,
  fmt,
}: {
  daily: DayStat[];
  seed: number[];
  summary: AnalyticsSummary;
  members: MemberStat[];
  agg: ReportAggregates;
  devices: DeviceRow[] | null;
  projectNameOf: (id: string) => string;
  fmt: Fmt;
}) {
  const dense = daily.length > 14;
  // The average is seeded with the days before the period, so its first days already cover a full week.
  const averages = movingAverage(
    daily.map((d) => d.minutes / 60),
    7,
    seed.map((m) => m / 60),
  );
  const showAverage = daily.length > 1;
  const trend = daily.map((d, i) => ({
    label: dayLabel(d.day, dense),
    longLabel: longDayLabel(d.day),
    hours: Math.round((d.minutes / 60) * 100) / 100,
    minutes: d.minutes,
    average: showAverage ? Math.round(averages[i] * 100) / 100 : null,
    activity: d.minutes > 0 ? d.activity : null,
  }));
  // Pace: a full 8h weekday for each member who tracked time.
  const perWeekday = 8 * 60 * Math.max(1, members.length);
  const actual = cumulative(daily.map((d) => d.minutes));
  const target = paceTarget(daily, perWeekday);
  const pace = daily.map((d, i) => ({
    label: dayLabel(d.day, dense),
    longLabel: longDayLabel(d.day),
    actual: Math.round((actual[i] / 60) * 100) / 100,
    target: Math.round((target[i] / 60) * 100) / 100,
  }));
  const gap = (actual.at(-1) ?? 0) - (target.at(-1) ?? 0);
  const level = activityLevel(summary.activity, summary.totalMinutes);
  const projects = agg.projects.filter((p) => p.minutes > 0).slice(0, 5);
  const projectMax = projects[0]?.minutes ?? 0;
  const idleKnown = daily.some((d) => d.idleMinutes !== null);
  const idleRows = daily.map((d) => ({
    label: dayLabel(d.day, dense),
    longLabel: longDayLabel(d.day),
    minutes: d.minutes,
    idle: d.idleMinutes ?? 0,
    share: idleShare(d.idleMinutes ?? 0, d.minutes),
  }));

  return (
    <>
      <ChartCard
        title="Hours and activity trend"
        subtitle={showAverage ? "Hours tracked per day, the 7-day average (including the days before), and activity %" : "Hours tracked and activity %"}
        legend={
          <ChartLegend>
            <LegendItem color="var(--primary)" label="Hours" />
            {showAverage && <LegendItem color="#7c3aed" label="7-day average" line dashed />}
            <LegendItem color="var(--success)" label="Activity %" line />
          </ChartLegend>
        }
      >
        <TrendChart data={trend} fmt={fmt} showAverage={showAverage} />
      </ChartCard>

      <div className="grid grid-cols-1 gap-5 xl:grid-cols-3">
        <ChartCard
          className="xl:col-span-2"
          title="Pace"
          subtitle={`Running total against a full 8h weekday for ${members.length === 1 ? "one person" : `${members.length} members`}`}
          legend={
            <span
              className={cn(
                "print-exact rounded-full px-2.5 py-1 text-xs font-semibold",
                gap >= 0 ? "bg-success/12 text-success-on-tint" : "bg-warning/15 text-warning-on-tint",
              )}
            >
              {gap >= 0 ? `Ahead by ${fmt(gap)}` : `Behind by ${fmt(-gap)}`}
            </span>
          }
        >
          <PaceChart data={pace} fmt={fmt} />
        </ChartCard>

        <ChartCard title="Activity level" subtitle="Share of tracked minutes with input">
          <ActivityGauge value={summary.activity} label={summary.activity === null ? "Not measured" : activityLevelLabel[level]} />
          <ul className="mt-4 space-y-2.5">
            {projects.map((p) => (
              <li key={p.projectId} className="text-sm">
                <div className="flex items-baseline justify-between gap-2">
                  <span className="truncate font-medium">{projectNameOf(p.projectId)}</span>
                  <span className="shrink-0 tabular-nums text-muted-foreground">
                    {fmt(p.minutes)} · {formatActivity(p.activity === null ? null : Math.round(p.activity))}
                  </span>
                </div>
                <div aria-hidden className="print-exact mt-1 h-1.5 overflow-hidden rounded-full bg-muted">
                  <div
                    className="print-exact h-full rounded-full"
                    style={{ width: `${Math.max(3, (p.minutes / Math.max(1, projectMax)) * 100)}%`, background: scoreFill(p.activity) }}
                  />
                </div>
              </li>
            ))}
          </ul>
          <p className="mt-3 text-[11px] text-muted-foreground">Top projects; bar colour shows each project&apos;s activity level.</p>
        </ChartCard>
      </div>

      <div className="grid grid-cols-1 gap-5 xl:grid-cols-3">
        <ChartCard
          className="xl:col-span-2"
          title="Idle time per day"
          subtitle={
            idleKnown && summary.idleMinutes !== null
              ? `Tracked time with no keyboard or mouse input · ${fmt(summary.idleMinutes)} in all (${idleShare(summary.idleMinutes, summary.totalMinutes)}%)`
              : "Tracked time with no keyboard or mouse input"
          }
        >
          {idleKnown ? (
            <IdleChart data={idleRows} fmt={fmt} />
          ) : (
            <p className="text-sm text-muted-foreground">Idle time isn&apos;t measured for this period&apos;s tracked time.</p>
          )}
        </ChartCard>
        <DevicesCard devices={devices} fmt={fmt} />
      </div>
    </>
  );
}

/** A semicircular gauge for the activity %; "—" when nothing was measured. */
function ActivityGauge({ value, label }: { value: number | null; label: string }) {
  const id = useId().replace(/[^a-zA-Z0-9_-]/g, "");
  const v = value === null ? 0 : Math.min(100, Math.max(0, value));
  const r = 70;
  const cx = 90;
  const cy = 86;
  const angle = Math.PI * (1 - v / 100);
  const x = cx + r * Math.cos(angle);
  const y = cy - r * Math.sin(angle);
  return (
    <svg
      viewBox="0 0 180 100"
      role="img"
      aria-label={value === null ? "Average activity not measured" : `Average activity ${Math.round(v)}%, ${label}`}
      className="mx-auto block h-auto w-full max-w-[240px]"
    >
      <defs>
        <linearGradient id={`${id}-g`} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0%" style={{ stopColor: "var(--danger)" }} />
          <stop offset="50%" style={{ stopColor: "var(--warning)" }} />
          <stop offset="100%" style={{ stopColor: "var(--success)" }} />
        </linearGradient>
      </defs>
      <path d={`M${cx - r},${cy} A${r},${r} 0 0 1 ${cx + r},${cy}`} fill="none" stroke="var(--muted)" strokeWidth={14} strokeLinecap="round" />
      {value !== null && (
        <>
          <path
            d={`M${cx - r},${cy} A${r},${r} 0 0 1 ${x.toFixed(2)},${y.toFixed(2)}`}
            fill="none"
            stroke={`url(#${id}-g)`}
            strokeWidth={14}
            strokeLinecap="round"
            pathLength={1}
            className="print-exact chart-draw"
          />
          <circle cx={x} cy={y} r={7} fill="var(--card)" stroke="var(--foreground)" strokeWidth={2} className="chart-grow" style={{ transformOrigin: `${x}px ${y}px` }} />
        </>
      )}
      <text x={cx} y={cy - 14} textAnchor="middle" fontSize={24} fontWeight={700} fill="var(--foreground)">
        {value === null ? "—" : `${Math.round(v)}%`}
      </text>
      <text x={cx} y={cy + 4} textAnchor="middle" fontSize={11} fill="var(--muted-foreground)">
        {label}
      </text>
    </svg>
  );
}

/** Which platforms the time was tracked on, and how current the desktop apps are. */
function DevicesCard({ devices, fmt }: { devices: DeviceRow[] | null; fmt: Fmt }) {
  const shares = devices ? platformMix(devices) : [];
  const versions = devices ? agentVersions(devices) : [];
  const deviceCount = devices?.length ?? 0;
  const memberCount = devices ? new Set(devices.map((d) => d.userId)).size : 0;
  return (
    <ChartCard
      title="Devices & platforms"
      subtitle={
        devices && deviceCount
          ? `${deviceCount} ${deviceCount === 1 ? "device" : "devices"} · ${memberCount} ${memberCount === 1 ? "member" : "members"}`
          : "Where the time was tracked"
      }
    >
      {devices === null ? (
        <p className="text-sm text-muted-foreground">Device details aren&apos;t available from this server.</p>
      ) : shares.length === 0 ? (
        <p className="text-sm text-muted-foreground">No device details were reported for this period. Desktop apps report their platform and version from 0.2 on.</p>
      ) : (
        <div className="space-y-4">
          <PlatformMix shares={shares} fmt={fmt} />
          <div>
            <p className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
              <MonitorSmartphone aria-hidden className="h-3.5 w-3.5" /> Desktop app versions
            </p>
            <ul aria-label="Desktop app versions" className="flex flex-wrap gap-1.5">
              {versions.map((v) => (
                <li key={v.version ?? "none"}>
                  <Badge tone={v.version ? "default" : "warning"} className="tabular-nums">
                    {v.version ? `v${v.version}` : "No version"} · {v.devices}
                  </Badge>
                </li>
              ))}
            </ul>
          </div>
        </div>
      )}
    </ChartCard>
  );
}

/* ----------------------------- Rhythm ----------------------------- */

function RhythmView({
  fmt,
  blocksState,
  sampleNote,
  onRetryBlocks,
  cells,
  cellsFromBlocks,
  cellsSampled,
  windows,
  daily,
  blocks,
  truncated,
  memberDays,
  calendar,
  clock,
  zoneText,
  nameOf,
  projectNameOf,
}: BlocksProps & {
  cells: HourCell[];
  /** The cells were read from the blocks (the server has no /reporting/hourly). */
  cellsFromBlocks: boolean;
  cellsSampled: boolean;
  windows: WorkWindow[];
  daily: DayStat[];
  blocks: Block[];
  truncated: boolean;
  memberDays: ReportAggregates["memberDays"];
  calendar: ZoneCalendar;
  clock: MemberClock;
  zoneText: string;
  nameOf: (id: string) => string;
  projectNameOf: (id: string) => string;
}) {
  const grid = rhythmMatrix(cells);
  const profile = hourTotals(cells);
  const avgStart = meanOf(windows.map((w) => w.start));
  const avgEnd = meanOf(windows.map((w) => w.end));
  const worked = windows.filter((w) => w.start !== null);
  const avgSpan = meanOf(worked.map((w) => (w.end as number) - (w.start as number)));
  let best: { day: number; hour: number; v: number } | null = null;
  grid.forEach((row, day) => row.forEach((v, hour) => (best = v > (best?.v ?? 0) ? { day, hour, v } : best)));
  const top = best as { day: number; hour: number; v: number } | null;
  const summary = top
    ? `Weekday by hour heat map of tracked time. The busiest slot is ${WEEKDAYS[top.day]} ${hourRangeLabel(top.hour)} with ${fmt(top.v)}.`
    : "Weekday by hour heat map of tracked time.";
  return (
    <>
      <ChartCard
        title="Work rhythm"
        subtitle={`When work happens: weekday × hour of day, in ${zoneText}`}
        sampleNote={cellsSampled ? sampleNote : null}
      >
        {/* Without /reporting/hourly (an older server) the rhythm is read from the blocks, so it waits for them. */}
        <BlocksGate state={cellsFromBlocks ? blocksState : "ready"} onRetry={onRetryBlocks} height={220}>
          <RhythmHeatmap grid={grid} fmt={fmt} summary={summary} />
        </BlocksGate>
      </ChartCard>
      <div className="grid grid-cols-1 gap-5 xl:grid-cols-5">
        <ChartCard className="xl:col-span-2" title="24-hour clock" subtitle={`Time tracked in each hour of the day, in ${zoneText}`} sampleNote={cellsSampled ? sampleNote : null}>
          <BlocksGate state={cellsFromBlocks ? blocksState : "ready"} onRetry={onRetryBlocks} height={280}>
            <DayClock profile={profile} fmt={fmt} />
          </BlocksGate>
        </ChartCard>
        <ChartCard
          className="xl:col-span-3"
          title="Working window per day"
          subtitle="From the first start to the last finish, each member on their own local clock"
          legend={
            <ChartLegend>
              <LegendItem color="var(--success)" label="Average start" line dashed />
              <LegendItem color="var(--danger)" label="Average finish" line dashed />
            </ChartLegend>
          }
        >
          <WorkWindowsChart windows={windows} fmt={fmt} avgStart={avgStart} avgEnd={avgEnd} />
          <div className="mt-4 grid grid-cols-1 gap-2 sm:grid-cols-3">
            <Stat icon={Sunrise} label="Average start" value={avgStart === null ? "—" : clockLabel(avgStart)} tone="text-success" />
            <Stat icon={Sunset} label="Average finish" value={avgEnd === null ? "—" : clockLabel(avgEnd)} tone="text-danger" />
            <Stat icon={Clock3} label="Average span" value={avgSpan === null ? "—" : fmt(avgSpan)} />
          </div>
        </ChartCard>
      </div>
      <DayTimelineCard
        fmt={fmt}
        blocksState={blocksState}
        sampleNote={sampleNote}
        onRetryBlocks={onRetryBlocks}
        daily={daily}
        blocks={blocks}
        truncated={truncated}
        memberDays={memberDays}
        calendar={calendar}
        clock={clock}
        zoneText={zoneText}
        nameOf={nameOf}
        projectNameOf={projectNameOf}
      />
    </>
  );
}

/** One day of the period on a clock, a lane per member, with the gaps between blocks labelled by why they ended. */
function DayTimelineCard({
  fmt,
  blocksState,
  sampleNote,
  onRetryBlocks,
  daily,
  blocks,
  truncated,
  memberDays,
  calendar,
  clock,
  zoneText,
  nameOf,
  projectNameOf,
}: BlocksProps & {
  daily: DayStat[];
  blocks: Block[];
  truncated: boolean;
  memberDays: ReportAggregates["memberDays"];
  calendar: ZoneCalendar;
  clock: MemberClock;
  zoneText: string;
  nameOf: (id: string) => string;
  projectNameOf: (id: string) => string;
}) {
  const pickerId = useId();
  const workedDays = daily.filter((d) => d.minutes > 0).map((d) => d.day);
  const [picked, setPicked] = useState<string | null>(null);
  const day = picked && workedDays.includes(picked) ? picked : (workedDays[workedDays.length - 1] ?? null);
  const covered = day !== null && coversDay(blocks, truncated, day, calendar);
  // One day's blocks: cheap enough to lay out on every render.
  const timeline = day && blocksState === "ready" && covered ? dayTimeline(blocks, day, calendar) : null;
  const present = new Set<GapKind>();
  for (const lane of timeline?.lanes ?? []) for (const g of lane.gaps) present.add(gapKind(g.reason).kind);
  const kinds = (["break", "idle", "lock", "sleep", "off", "untracked"] as const).filter((k) => present.has(k));
  const spans = new Map(memberDays.filter((md) => md.day === day).map((md) => [md.userId, md]));
  return (
    <ChartCard
      title="Day timeline"
      subtitle={`Tracked blocks and the gaps between them — breaks, idle, screen locked, asleep — on ${zoneText}`}
      sampleNote={sampleNote}
      legend={
        workedDays.length > 0 ? (
          <div className="no-print flex items-center gap-2">
            <label htmlFor={pickerId} className="text-xs font-medium text-muted-foreground">
              Day
            </label>
            <Select id={pickerId} value={day ?? ""} onChange={(e) => setPicked(e.target.value)} className="h-9 w-auto min-w-36">
              {[...workedDays].reverse().map((d) => (
                <option key={d} value={d}>
                  {longDayLabel(d)}
                </option>
              ))}
            </Select>
          </div>
        ) : undefined
      }
    >
      <BlocksGate state={blocksState} onRetry={onRetryBlocks} height={160}>
        {!day ? (
          <p className="text-sm text-muted-foreground">No day of this period has tracked time.</p>
        ) : !covered || !timeline ? (
          <p className="text-sm text-muted-foreground">
            {longDayLabel(day)} is older than the newest {MAX_BLOCKS.toLocaleString("en")} blocks of the period, so its timeline isn&apos;t shown. Pick a
            later day, or narrow the period or filters.
          </p>
        ) : timeline.lanes.length === 0 ? (
          <p className="text-sm text-muted-foreground">No blocks to draw for {longDayLabel(day)}.</p>
        ) : (
          <div className="space-y-3">
            <DayTimelineChart
              timeline={timeline}
              meta={(userId) => {
                const md = spans.get(userId);
                return { name: nameOf(userId), local: md ? localSpanLabel(clock, calendar, userId, md.firstStart, md.lastEnd) : null };
              }}
              colorOf={projectColorOf}
              projectNameOf={projectNameOf}
              clockOf={(ms) => clockLabel(calendar.minuteOfDay(ms))}
              fmt={fmt}
            />
            <TimelineLegend kinds={kinds} />
          </div>
        )}
      </BlocksGate>
    </ChartCard>
  );
}

/* ----------------------------- Focus & activity ----------------------------- */

function FocusView({
  fmt,
  blocksState,
  sampleNote,
  onRetryBlocks,
  blocks,
  profile,
  summary,
}: BlocksProps & { blocks: Block[]; profile: ReturnType<typeof hourProfile>; summary: AnalyticsSummary }) {
  const stretches = focusStretches(blocks);
  const histogram = stretchHistogram(stretches);
  const deepMinutes = stretches.filter((s) => s.minutes >= 60).reduce((sum, s) => sum + s.minutes, 0);
  const allMinutes = stretches.reduce((sum, s) => sum + s.minutes, 0);
  const deepShare = allMinutes > 0 ? Math.round((deepMinutes / allMinutes) * 100) : 0;
  const unmeasured = unmeasuredMinutes(blocks);
  return (
    <>
      <div className="grid grid-cols-1 gap-5 xl:grid-cols-3">
        <ChartCard
          className="xl:col-span-2"
          title="Focus stretches"
          subtitle="Continuous sittings by length (back-to-back blocks, gaps of 5 minutes or less)"
          sampleNote={sampleNote}
        >
          <BlocksGate state={blocksState} onRetry={onRetryBlocks}>
            <FocusChart data={histogram} fmt={fmt} />
          </BlocksGate>
        </ChartCard>
        <ChartCard title="Deep work" subtitle="Time in stretches of an hour or more" sampleNote={sampleNote}>
          <BlocksGate state={blocksState} onRetry={onRetryBlocks} height={144}>
            <DeepWorkRing percent={deepShare} />
          </BlocksGate>
          <div className="mt-4 grid grid-cols-2 gap-2">
            <Stat icon={Timer} label="Average stretch" value={summary.avgStretch === null ? "—" : fmt(summary.avgStretch)} tone="text-violet-600 dark:text-violet-300" />
            <Stat icon={TrendingUp} label="Longest stretch" value={summary.longestStretch === null ? "—" : fmt(summary.longestStretch)} tone="text-violet-600 dark:text-violet-300" />
            <Stat icon={Zap} label="Stretches" value={blocksState === "ready" ? String(stretches.length) : "—"} tone="text-violet-600 dark:text-violet-300" />
            <Stat icon={Clock3} label="In 1h+ stretches" value={blocksState === "ready" ? fmt(deepMinutes) : "—"} tone="text-violet-600 dark:text-violet-300" />
            <Stat
              icon={Keyboard}
              label="Input per tracked hour"
              value={summary.inputPerHour === null ? "—" : summary.inputPerHour.toLocaleString("en")}
              tone="text-cyan-700 dark:text-cyan-300"
            />
          </div>
        </ChartCard>
      </div>
      <div className="grid grid-cols-1 gap-5 xl:grid-cols-2">
        <ChartCard
          title="Activity distribution"
          subtitle={
            unmeasured > 0 && blocksState === "ready"
              ? `Hours tracked at each activity level · ${fmt(unmeasured)} not measured, left out`
              : "Hours tracked at each activity level"
          }
          sampleNote={sampleNote}
        >
          <BlocksGate state={blocksState} onRetry={onRetryBlocks}>
            <ActivityDistributionChart data={activityHistogram(blocks)} fmt={fmt} />
          </BlocksGate>
        </ChartCard>
        <ChartCard
          title="Activity through the day"
          subtitle="Hours (bars) and activity % (line) by hour"
          sampleNote={sampleNote}
          legend={
            <ChartLegend>
              <LegendItem color="color-mix(in srgb, var(--primary) 35%, transparent)" label="Hours" />
              <LegendItem color="var(--success)" label="Activity %" line />
            </ChartLegend>
          }
        >
          <BlocksGate state={blocksState} onRetry={onRetryBlocks} height={260}>
            <HourActivityChart profile={profile} fmt={fmt} />
          </BlocksGate>
        </ChartCard>
      </div>
    </>
  );
}

/** A ring filling to the share of deep work. */
function DeepWorkRing({ percent }: { percent: number }) {
  const r = 52;
  const c = 2 * Math.PI * r;
  const p = Math.min(100, Math.max(0, percent));
  return (
    <svg viewBox="0 0 140 140" role="img" aria-label={`${p}% of tracked time was in stretches of an hour or more`} className="mx-auto block h-36 w-36">
      <circle cx={70} cy={70} r={r} fill="none" stroke="var(--muted)" strokeWidth={12} />
      <circle
        cx={70}
        cy={70}
        r={r}
        fill="none"
        stroke="#7c3aed"
        strokeWidth={12}
        strokeLinecap="round"
        strokeDasharray={`${(p / 100) * c} ${c}`}
        transform="rotate(-90 70 70)"
        className="print-exact chart-grow"
        style={{ transformOrigin: "70px 70px" }}
      />
      <text x={70} y={68} textAnchor="middle" fontSize={26} fontWeight={700} fill="var(--foreground)">
        {p}%
      </text>
      <text x={70} y={88} textAnchor="middle" fontSize={10} fill="var(--muted-foreground)">
        deep work
      </text>
    </svg>
  );
}

/* ----------------------------- Projects & apps ----------------------------- */

const STACK_TOP = 5;
/** Member nodes of the flow diagram stay neutral, so only projects carry colour. */
const MEMBER_NODE_COLOR = "var(--muted-foreground)";

function ProjectsView({
  fmt,
  blocksState,
  sampleNote,
  onRetryBlocks,
  agg,
  blocks,
  days,
  calendar,
  nameOf,
  projectNameOf,
}: BlocksProps & {
  agg: ReportAggregates;
  blocks: Block[];
  days: string[];
  calendar: ZoneCalendar;
  nameOf: (id: string) => string;
  projectNameOf: (id: string) => string;
}) {
  const stats = agg.projects.filter((p) => p.minutes > 0);
  const total = stats.reduce((s, p) => s + p.minutes, 0);
  const items: TreemapItem[] = stats.map((p) => ({
    name: projectNameOf(p.projectId),
    size: Math.max(0.01, p.minutes / 60),
    minutes: p.minutes,
    share: total > 0 ? Math.round((p.minutes / total) * 100) : 0,
    color: projectColorOf(p.projectId),
  }));
  const top = stats.slice(0, STACK_TOP).map((p) => p.projectId);
  const series = [
    ...top.map((id) => ({ key: id, name: projectNameOf(id), color: projectColorOf(id) })),
    ...(stats.length > STACK_TOP ? [{ key: OTHER, name: "Other projects", color: OTHER_COLOR }] : []),
  ];
  const pairsSampled = agg.memberProjects === null;
  const raw = memberProjectFlow(agg.memberProjects ?? memberProjectsFromBlocks(blocks), nameOf, projectNameOf);
  const flow = {
    links: raw.links,
    nodes: raw.nodes.map(
      (n): FlowNodeData => ({
        ...n,
        color: n.kind === "member" ? MEMBER_NODE_COLOR : n.id === OTHER ? OTHER_COLOR : projectColorOf(n.id),
      }),
    ),
  };
  const apps = appStats(appUsageFromBlocks(blocks), 8);
  return (
    <>
      <div className="grid grid-cols-1 gap-5 xl:grid-cols-2">
        <ChartCard title="Project map" subtitle={`${stats.length} ${stats.length === 1 ? "project" : "projects"} sized by hours`}>
          <ProjectTreemap items={items} fmt={fmt} />
        </ChartCard>
        <ChartCard
          title="Projects per day"
          subtitle="Hours each day, by project"
          sampleNote={sampleNote}
          legend={
            <ChartLegend>
              {series.map((s) => (
                <LegendItem key={s.key} color={s.color} label={s.name} />
              ))}
            </ChartLegend>
          }
        >
          <BlocksGate state={blocksState} onRetry={onRetryBlocks} height={260}>
            <ProjectStackChart rows={projectDays(blocks, days, top, calendar)} series={series} fmt={fmt} />
          </BlocksGate>
        </ChartCard>
      </div>
      <ChartCard
        title="Who works on what"
        subtitle="Hours flowing from members (left) to projects (right); each band carries its project's colour"
        sampleNote={pairsSampled ? sampleNote : null}
      >
        {pairsSampled ? (
          <BlocksGate state={blocksState} onRetry={onRetryBlocks} height={220}>
            <FlowDiagram flow={flow} fmt={fmt} />
          </BlocksGate>
        ) : (
          <FlowDiagram flow={flow} fmt={fmt} />
        )}
      </ChartCard>
      <ChartCard title="Top apps" subtitle="Focused time per application" sampleNote={sampleNote}>
        <BlocksGate state={blocksState} onRetry={onRetryBlocks} height={180}>
          {apps.length ? <AppsChart apps={apps} fmt={fmt} /> : <p className="text-sm text-muted-foreground">No app usage was recorded for these blocks.</p>}
        </BlocksGate>
      </ChartCard>
    </>
  );
}

/* ----------------------------- Team ----------------------------- */

const RADAR_MAX = 3;

function TeamView({
  fmt,
  blocksState,
  sampleNote,
  onRetryBlocks,
  members,
  days,
  nameOf,
  memberColor,
}: BlocksProps & {
  members: MemberStat[];
  days: string[];
  nameOf: (id: string) => string;
  memberColor: (id: string) => string;
}) {
  const [picked, setPicked] = useState<string[]>(() => members.slice(0, RADAR_MAX).map((m) => m.userId));
  const chosen = picked.filter((id) => members.some((m) => m.userId === id));
  const toggle = (id: string) =>
    setPicked((cur) => (cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id].slice(-RADAR_MAX)));
  const measured = members.filter((m) => m.activity !== null);
  const points = measured.map((m) => ({
    id: m.userId,
    name: nameOf(m.userId),
    hours: Math.round((m.minutes / 60) * 100) / 100,
    activity: m.activity as number,
    sessions: m.sessions,
    color: memberColor(m.userId),
  }));
  const unplotted = members.length - measured.length;
  const radar = radarRows(members, days.length);
  const maxMinutes = members[0]?.minutes ?? 0;
  return (
    <>
      <div className="grid grid-cols-1 gap-5 xl:grid-cols-2">
        <ChartCard
          title="Hours × activity"
          subtitle={`Each member; bubble size is sessions. Dashed lines are the team averages.${
            unplotted > 0 ? ` ${unplotted} ${unplotted === 1 ? "member" : "members"} without measured activity ${unplotted === 1 ? "isn't" : "aren't"} plotted.` : ""
          }`}
        >
          <TeamScatter points={points} fmt={fmt} />
        </ChartCard>
        <ChartCard title="Member profiles" subtitle={`Compare up to ${RADAR_MAX} members on six 0–100 scales`} sampleNote={sampleNote}>
          <div role="group" aria-label="Members to compare" className="no-print mb-2 flex flex-wrap gap-1.5">
            {members.map((m) => {
              const on = chosen.includes(m.userId);
              return (
                <button
                  key={m.userId}
                  type="button"
                  aria-pressed={on}
                  onClick={() => toggle(m.userId)}
                  className={cn(
                    "inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs font-medium transition-colors",
                    "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                    on ? "border-transparent text-white" : "border-border text-muted-foreground hover:bg-muted",
                  )}
                  style={on ? { background: memberColor(m.userId) } : undefined}
                >
                  {nameOf(m.userId)}
                </button>
              );
            })}
          </div>
          {/* Focus and input come from the blocks: the profiles wait for them. */}
          <BlocksGate state={blocksState} onRetry={onRetryBlocks} height={320}>
            <TeamRadar rows={radar} members={chosen.map((id) => ({ id, name: nameOf(id), color: memberColor(id) }))} />
          </BlocksGate>
        </ChartCard>
      </div>

      <ChartCard title="Member breakdown" subtitle={`${members.length} members with tracked time · start and finish on each member's own clock`}>
        <div className="overflow-x-auto overscroll-x-contain">
          <table className="w-full min-w-[880px] border-collapse text-sm">
            <caption className="sr-only">Per-member hours, idle time, activity, days active, typical working day, focus and consistency.</caption>
            <thead>
              <tr className="border-b border-border text-[11px] uppercase tracking-wider text-muted-foreground">
                <th scope="col" className="py-2 pr-3 text-left font-semibold">Member</th>
                <th scope="col" className="px-3 py-2 text-right font-semibold">Hours</th>
                <th scope="col" className="w-[14%] px-3 py-2 text-left font-semibold">Trend</th>
                <th scope="col" className="px-3 py-2 text-right font-semibold">Idle</th>
                <th scope="col" className="px-3 py-2 text-right font-semibold">Activity</th>
                <th scope="col" className="px-3 py-2 text-right font-semibold">Days</th>
                <th scope="col" className="px-3 py-2 text-right font-semibold">Typical day</th>
                <th scope="col" className="px-3 py-2 text-right font-semibold">Avg stretch</th>
                <th scope="col" className="py-2 pl-3 text-right font-semibold">Consistency</th>
              </tr>
            </thead>
            <tbody>
              {members.map((m) => (
                <tr key={m.userId} className="border-b border-border/60 transition-colors hover:bg-muted/40">
                  <th scope="row" className="py-2.5 pr-3 text-left font-medium">
                    <span className="flex items-center gap-2.5">
                      <Avatar name={nameOf(m.userId)} size="sm" className="print:hidden" />
                      <span className="min-w-0">
                        <span className="block truncate">{nameOf(m.userId)}</span>
                        <span aria-hidden className="print-exact mt-1 block h-1 w-24 overflow-hidden rounded-full bg-muted">
                          <span className="block h-full rounded-full" style={{ width: `${(m.minutes / Math.max(1, maxMinutes)) * 100}%`, background: memberColor(m.userId) }} />
                        </span>
                      </span>
                    </span>
                  </th>
                  <td className="whitespace-nowrap px-3 py-2.5 text-right font-semibold tabular-nums">{fmt(m.minutes)}</td>
                  <td className="px-3 py-2.5">
                    <Sparkline values={m.daily} color={memberColor(m.userId)} className="h-7" />
                  </td>
                  <td className="whitespace-nowrap px-3 py-2.5 text-right tabular-nums text-muted-foreground">
                    {m.idleMinutes === null ? <NotMeasured /> : fmt(m.idleMinutes)}
                  </td>
                  <td className="px-3 py-2.5 text-right">
                    {m.activity === null ? (
                      <NotMeasured />
                    ) : (
                      <Badge tone={m.activity >= 75 ? "success" : m.activity >= 50 ? "warning" : "danger"} className="print-exact tabular-nums">
                        {m.activity}%
                      </Badge>
                    )}
                  </td>
                  <td className="px-3 py-2.5 text-right tabular-nums">
                    {m.daysActive}
                    <span className="text-muted-foreground">/{days.length}</span>
                  </td>
                  <td className="whitespace-nowrap px-3 py-2.5 text-right tabular-nums" title={m.localEstimated ? "No time zone reported: shown in the report's time zone" : undefined}>
                    {m.typicalStart === null || m.typicalEnd === null ? (
                      <NotMeasured label="Not known" />
                    ) : (
                      `${clockLabel(m.typicalStart)}–${clockLabel(m.typicalEnd)}${m.localEstimated ? "*" : ""}`
                    )}
                  </td>
                  <td className="whitespace-nowrap px-3 py-2.5 text-right tabular-nums">{m.avgStretch === null ? <NotMeasured label="Not loaded" /> : fmt(m.avgStretch)}</td>
                  <td className="py-2.5 pl-3 text-right tabular-nums">
                    {m.consistency === null ? (
                      <NotMeasured label="Fewer than two days" />
                    ) : (
                      <span className="inline-flex items-center gap-2">
                        <span aria-hidden className="print-exact hidden h-1.5 w-14 overflow-hidden rounded-full bg-muted sm:block">
                          <span className="block h-full rounded-full" style={{ width: `${m.consistency}%`, background: scoreFill(m.consistency) }} />
                        </span>
                        {m.consistency}%
                      </span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {members.some((m) => m.localEstimated) && (
          <p className="mt-3 text-[11px] text-muted-foreground">* No time zone reported by the member&apos;s desktop app: read in the report&apos;s time zone.</p>
        )}
      </ChartCard>
    </>
  );
}

/** "—", read out as what it means. */
function NotMeasured({ label = "Not measured" }: { label?: string }) {
  return (
    <>
      <span aria-hidden className="text-muted-foreground">
        —
      </span>
      <span className="sr-only">{label}</span>
    </>
  );
}
