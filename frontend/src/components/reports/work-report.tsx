"use client";

import { Fragment, useCallback, useId, useMemo, useState } from "react";
import type { LucideIcon } from "lucide-react";
import {
  ArrowDownRight,
  ArrowRight,
  ArrowUpRight,
  BarChart3,
  CalendarCheck,
  CalendarClock,
  CalendarDays,
  ChevronDown,
  Clipboard,
  Coffee,
  Filter,
  FolderKanban,
  Gauge,
  Globe,
  Info,
  LayoutGrid,
  ListChecks,
  RotateCw,
  Sheet,
  Sparkles,
  Sun,
  Target,
  Timer,
  Trophy,
  Users,
  Zap,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { toast } from "@/components/toast";
import { ExportMenu, LiveReportNotice, ReportShell, useLiveReport, useReportAccess } from "./report-shell";
import { HOURS_FORMAT_KEY, PeriodToolbar, Segmented, type CustomRange, type ToolbarPeriod } from "./report-controls";
import { DayTimelineChart, SampleBadge, TimelineLegend } from "./analytics-charts";
import { ChartLoading } from "@/components/charts/chart-loading";
import { useRecharts } from "@/components/charts/use-recharts";
import { useIsMdUp } from "@/hooks/use-media-query";
import { usePrefersReducedMotion } from "@/hooks/use-count-up";
import { useReportingZone } from "@/hooks/use-reporting-zone";
import { useStoredChoice } from "@/hooks/use-stored-choice";
import { getAllPagesApi, getApi } from "@/hooks/useApi";
import { useSession } from "@/components/session-provider";
import { activities as tenantActivities, projectById, userById } from "@/lib/tenant-data";
import { referenceNow } from "@/lib/live-session";
import { ALL, memberName } from "@/lib/report-math";
import { activitiesRoute, reportingRoute } from "@/lib/api-contract";
import { formatDayRange, timeZoneLabel, zonedDayKey } from "@/lib/time-zone";
import { downloadCSV } from "@/lib/export";
import { cn } from "@/lib/utils";
import {
  BLOCK_PAGE,
  MAX_BLOCKS,
  aggregateBlocks,
  blocksFromApi,
  blocksStartingIn,
  clockLabel,
  coversDay,
  dayTimeline,
  gapKind,
  localSpanLabel,
  memberClock,
  toBlocks,
  zoneCalendar,
  type Block,
  type GapKind,
  type ZoneCalendar,
} from "@/lib/analytics";
import {
  DAILY_LOG_HEADERS,
  HOURS_FORMATS,
  NOTES_CSV_HEADERS,
  NO_NOTE,
  TARGET_DAY_MINUTES,
  activityLevelLabel,
  aggregatesFromLive,
  barPercent,
  buildDailyLog,
  buildHighlights,
  buildTimesheetMatrix,
  calendarWeeks,
  changeVs,
  chartSeries,
  cleanMinutes,
  comparisonFor,
  comparisonText,
  countMembers,
  dailyLogCsvRows,
  formatActivity,
  formatAxisDay,
  formatDayLabel,
  formatHours,
  formatPeriod,
  formatStampInZone,
  formatTotal,
  heatLevel,
  hourTicks,
  idleShare,
  inputsFromAggregates,
  matrixCsv,
  membersOnDay,
  noteBreakdown,
  notesByDay,
  notesByMember,
  notesCsvRows,
  perMemberDay,
  projectBreakdown,
  projectLabel,
  resolvePeriod,
  scopeLabel,
  selectionTitle,
  summarizeLog,
  summaryText,
  weekdayShort,
  type ActivityLevel,
  type Change,
  type ChartDay,
  type DayRow,
  type Highlight,
  type HighlightKind,
  type HoursFormat,
  type MemberDay,
  type NoteBlock,
  type NoteEntry,
  type NoteRow,
  type NoteSection,
  type PeriodPreset,
  type PeriodSelection,
  type ProjectShare,
  type ReportAggregates,
  type TimesheetMatrix,
  type WorkSummary,
} from "@/lib/work-report";

/* ============================================================================
 * WORK REPORT — the client-ready, per-day hours document.
 *
 * Built to be read at a glance and sent as is: a hero with the period and the
 * headline total (and how it compares with the period before), the key
 * figures, hours per day as bars or a calendar, plain-language highlights, the
 * daily log (each day opens to who worked, when, and a timeline of the day),
 * the project split, what was worked on (the task notes), the team and its
 * timesheet, then how the hours were measured. Periods are one click (week /
 * 7 days / 30 days / month), step back with the arrows, or are a custom range
 * of days; hours read as "7h 30m", "7:30" or decimal "7.50" — the viewer's
 * choice, remembered.
 *
 * Every day is a calendar day of the reporting zone (useReportingZone), the
 * same zone the backend is asked to bucket by. The totals come from
 * /api/app/reporting/*; the task notes and day timelines need the blocks
 * themselves, read up to MAX_BLOCKS (the newest first) — past that, those two
 * say they are a sample.
 *
 * All arithmetic lives in lib/work-report.ts and lib/analytics.ts; this file
 * only lays it out. Live sessions never fall back to demo rows: a load shows
 * an honest notice, an empty period an honest empty state.
 * ========================================================================== */

const FORMAT_KEYS = HOURS_FORMATS.map((f) => f.key);
const EMPTY_BLOCKS: Block[] = [];

const EMPTY_AGGREGATES: ReportAggregates = {
  totals: { minutes: 0, sessions: 0, activity: null, idleMinutes: null, keys: 0, clicks: 0 },
  daily: [],
  members: [],
  projects: [],
  memberDays: [],
  memberProjects: null,
};

/** Colours for projects the roster has no colour for. */
const FALLBACK_COLORS = ["#006bff", "#00a389", "#7c3aed", "#db2777", "#ea580c", "#0891b2", "#65a30d"];

/** A project's colour: the roster's, else a stable pick by id (the same in every chart of the report). */
function projectColor(projectId: string): string {
  const known = projectById(projectId)?.color;
  if (known) return known;
  let hash = 0;
  for (let i = 0; i < projectId.length; i++) hash = (hash * 31 + projectId.charCodeAt(i)) >>> 0;
  return FALLBACK_COLORS[hash % FALLBACK_COLORS.length];
}

const levelTone: Record<ActivityLevel, "success" | "warning" | "danger" | "muted"> = {
  good: "success",
  ok: "warning",
  low: "danger",
  none: "muted",
};

/** Heat shading (primary at rising strength; the top step is solid, with light text). */
const heatClass: Record<0 | 1 | 2 | 3 | 4, string> = {
  0: "text-muted-foreground",
  1: "bg-primary/10 text-foreground",
  2: "bg-primary/25 text-foreground",
  3: "bg-primary/45 text-foreground",
  4: "bg-primary text-primary-foreground font-semibold",
};

interface LiveWorkReport {
  aggregates: ReportAggregates;
  /** False when the per-member day series couldn't be read (the timesheet is then unavailable). */
  memberDaysLoaded: boolean;
  /** Minutes tracked in the comparison period; null when it couldn't be read. */
  previousMinutes: number | null;
  generatedAt: Date;
}

interface LiveBlocks {
  blocks: Block[];
  /** Blocks the server holds for the period. */
  totalCount: number;
  /** Fewer were read than it holds (the newest were kept). */
  truncated: boolean;
}

/** Where the blocks behind the notes and timelines stand. */
type BlocksState = "loading" | "error" | "ready";

type ChartView = "bars" | "calendar";

export function WorkReport() {
  const session = useSession();
  const { isLive } = session;
  const workspaceName = session.workspace?.name?.trim() || "Workspace";
  const access = useReportAccess();
  // Without Tracker.Activities.ViewAll the server reports only the viewer, so the member filter offers just them.
  const liveWorkerId = isLive && !access.viewAll ? session.user.id : null;
  const zone = useReportingZone(isLive);
  const timeZone = zone.timeZone;
  const calendar = zoneCalendar(timeZone);

  // The clock the periods hang off: the real one live, the frozen demo instant otherwise.
  const [anchor] = useState(() => referenceNow());
  const [preset, setPreset] = useState<ToolbarPeriod>("7d");
  const [offset, setOffset] = useState(0);
  const [range, setRange] = useState<CustomRange | null>(null);
  const [projectId, setProjectId] = useState(ALL);
  const [memberId, setMemberId] = useState(ALL);
  const [format, setFormat] = useStoredChoice<HoursFormat>(HOURS_FORMAT_KEY, FORMAT_KEYS, "duration");
  const [chartChoice, setChartChoice] = useState<ChartView | null>(null);
  const [openDay, setOpenDay] = useState<string | null>(null);
  const [hideEmpty, setHideEmpty] = useState(false);
  const [stamp, setStamp] = useState(() => new Date());

  const fmt = useCallback((minutes: number) => formatHours(minutes, format), [format]);
  const selection = useMemo<PeriodSelection>(
    () =>
      preset === "custom" && range
        ? { kind: "custom", from: range.from, to: range.to }
        : { kind: "preset", preset: preset === "custom" ? "7d" : preset, offset },
    [preset, range, offset],
  );
  const period = useMemo(() => resolvePeriod(selection, anchor, timeZone), [selection, anchor, timeZone]);
  const compare = useMemo(() => comparisonFor(selection, anchor, timeZone, period), [selection, anchor, timeZone, period]);

  const loadLive = useCallback(async (): Promise<LiveWorkReport> => {
    const query = { from: period.from, to: period.to, projectId, userId: memberId, timeZone };
    const [summary, series, userDays, previous] = await Promise.all([
      getApi(reportingRoute("summary", query)),
      getApi(reportingRoute("daily-series", query)),
      // The team timesheet is a secondary section: without it the rest of the report still stands.
      getApi(reportingRoute("user-daily-series", query)).catch(() => null),
      // So is the comparison: without it the change simply isn't shown.
      compare.days.length
        ? getApi(reportingRoute("summary", { ...query, from: compare.from, to: compare.to })).catch(() => null)
        : Promise.resolve(null),
    ]);
    return {
      aggregates: aggregatesFromLive(summary, series, userDays),
      memberDaysLoaded: Array.isArray(userDays),
      previousMinutes:
        previous && typeof previous === "object" ? cleanMinutes((previous as { totalTrackedMinutes?: unknown }).totalTrackedMinutes) : null,
      generatedAt: new Date(),
    };
  }, [period, compare, projectId, memberId, timeZone]);

  // The blocks behind the task notes and the day timelines: read after the totals, so those never wait.
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
    const previous = compare.days.length ? blocksStartingIn(scoped, compare.from.getTime(), compare.to.getTime()) : null;
    return {
      aggregates: aggregateBlocks(blocks, calendar),
      blocks,
      previousMinutes: previous ? previous.reduce((s, b) => s + b.minutes, 0) : null,
    };
  }, [isLive, projectId, memberId, period, compare, calendar]);

  // A live session shows only real figures (or nothing) — never the demo dataset.
  const aggregates = isLive ? (live.data?.aggregates ?? EMPTY_AGGREGATES) : (demo?.aggregates ?? EMPTY_AGGREGATES);
  const previousMinutes = isLive ? (live.data?.previousMinutes ?? null) : (demo?.previousMinutes ?? null);
  const loading = isLive && (!zone.ready || live.loading);
  const pending = isLive && (loading || !!live.error || !live.data);
  const blocks = isLive ? (blockLoad.data?.blocks ?? EMPTY_BLOCKS) : (demo?.blocks ?? EMPTY_BLOCKS);
  const blocksState: BlocksState = !isLive || blockLoad.data ? "ready" : blockLoad.error ? "error" : "loading";
  const truncated = isLive && !!blockLoad.data?.truncated;
  const sampleNote = truncated
    ? `From the newest ${MAX_BLOCKS.toLocaleString("en")} of ${(blockLoad.data?.totalCount ?? 0).toLocaleString("en")} tracked blocks in this period`
    : null;

  const inputs = useMemo(() => inputsFromAggregates(aggregates), [aggregates]);
  const log = useMemo(() => buildDailyLog(period.days, inputs.daily), [period, inputs]);
  const summary = useMemo(() => summarizeLog(log, { activity: inputs.activity }), [log, inputs]);
  const nameOf = useCallback((id: string, hint: string | null) => memberName(id, userById(id) ?? (hint ? { name: hint } : null)), []);
  const matrix = useMemo(() => buildTimesheetMatrix(log.map((r) => r.date), inputs.memberDays, nameOf), [log, inputs, nameOf]);
  const projectRows = useMemo(() => projectBreakdown(inputs.projects, (id) => projectById(id)), [inputs]);
  const memberCount = countMembers(inputs.members);
  const clock = useMemo(
    () => memberClock(blocks, calendar, isLive ? undefined : (id) => userById(id)?.timezone),
    [blocks, calendar, isLive],
  );
  const noteBlocks = useMemo<NoteBlock[]>(
    () => blocks.map((b) => ({ userId: b.userId, day: calendar.dayKey(b.start), minutes: b.minutes, note: b.note })),
    [blocks, calendar],
  );
  const notes = useMemo(() => noteBreakdown(noteBlocks), [noteBlocks]);
  const notesPerDay = useMemo(() => notesByDay(noteBlocks), [noteBlocks]);
  const notesPerMember = useMemo(() => notesByMember(noteBlocks), [noteBlocks]);

  const days = log.map((r) => r.date);
  const periodText = formatPeriod(days);
  const projectScope = projectId === ALL ? null : projectLabel(projectId, projectById(projectId));
  const memberScope =
    memberId !== ALL
      ? memberName(memberId, userById(memberId))
      : liveWorkerId
        ? memberName(session.user.id, userById(session.user.id) ?? session.user)
        : null;
  const scope = scopeLabel(projectScope, memberScope);
  const generatedAt = (isLive ? live.data?.generatedAt : null) ?? stamp;
  const today = zonedDayKey(anchor, timeZone);
  const zoneText = timeZoneLabel(timeZone, anchor);

  // With a team in scope a day's hours are the team's total, so per-person ideas (an 8h day)
  // only apply to one member; a team's average day is per member-day instead.
  const team = memberCount > 1;
  const perMember = team && matrix.rows.length > 0 ? perMemberDay(matrix) : null;
  const showTarget = !team && summary.maxDayMinutes >= TARGET_DAY_MINUTES / 2;
  const dayScale = showTarget ? Math.max(summary.maxDayMinutes, TARGET_DAY_MINUTES) : summary.maxDayMinutes;
  const hasTime = summary.totalMinutes > 0;
  const showMatrix = !pending && hasTime && matrix.rows.length > 1;
  const matrixMissing = isLive && !pending && hasTime && memberCount > 1 && live.data?.memberDaysLoaded === false;
  const change = pending ? null : changeVs(summary.totalMinutes, previousMinutes);
  const comparison = comparisonText(selection, period.days.length);
  const chartView: ChartView = chartChoice ?? (log.length > 14 ? "calendar" : "bars");
  const highlights = buildHighlights({
    summary,
    projects: projectRows,
    matrix,
    team,
    weekendDays: log.filter((r) => r.weekend).length,
    fmt,
  });
  const total = formatTotal(
    log.map((r) => r.minutes),
    format,
  );

  function touch() {
    setStamp(new Date());
    setOpenDay(null);
  }
  function choosePreset(next: PeriodPreset) {
    setPreset(next);
    setOffset(0);
    touch();
  }
  function step(by: number) {
    setOffset((o) => Math.min(0, o + by));
    touch();
  }
  function applyRange(next: CustomRange) {
    setRange(next);
    setPreset("custom");
    touch();
  }

  /** Where a member's day starts and ends, on their own clock ("08:58–17:42 local (UTC−7)"). */
  const localText = useCallback(
    (userId: string, firstStart: number | null, lastEnd: number | null) => localSpanLabel(clock, calendar, userId, firstStart, lastEnd),
    [clock, calendar],
  );

  const fileBase = `work-report-${days[0] ?? "period"}${days.length > 1 ? `-to-${days[days.length - 1]}` : ""}`;
  function exportDaily() {
    downloadCSV(`${fileBase}-daily`, DAILY_LOG_HEADERS, dailyLogCsvRows(log, summary));
  }
  function exportMatrix() {
    const { headers, rows } = matrixCsv(matrix);
    downloadCSV(`${fileBase}-timesheet`, headers, rows);
  }
  function exportNotes() {
    downloadCSV(`${fileBase}-notes`, NOTES_CSV_HEADERS, notesCsvRows(noteBlocks, (id) => nameOf(id, null)));
  }
  async function copySummary() {
    const text = summaryText({
      workspace: workspaceName,
      period: periodText,
      scope,
      summary,
      projects: projectRows,
      change,
      comparison,
      perMember,
      fmt,
      notes: blocksState === "ready" ? notes : [],
    });
    try {
      await navigator.clipboard.writeText(text);
      toast({ tone: "success", title: "Summary copied", description: "Paste it into an email or a chat message." });
    } catch {
      toast({ tone: "warning", title: "Couldn't copy the summary", description: "Your browser blocked clipboard access." });
    }
  }

  const exportItems = [
    ...(showMatrix ? [{ label: "Team timesheet (CSV)", onSelect: exportMatrix, icon: Sheet }] : []),
    ...(blocksState === "ready" && notes.length ? [{ label: "Work notes (CSV)", onSelect: exportNotes, icon: ListChecks }] : []),
  ];

  return (
    <ReportShell
      aiExplain={{ slug: "work-report", query: { from: period.from, to: period.to, projectId, userId: memberId, timeZone } }}
      title="Work Report"
      description="Hours worked per day, ready to share — copy a summary, export a CSV, or save it as a PDF."
      actions={
        <div className="grid w-full grid-cols-2 gap-2 sm:flex sm:w-auto">
          <Button variant="outline" onClick={() => void copySummary()} disabled={pending || !hasTime} className="h-10 whitespace-nowrap">
            <Clipboard className="h-4 w-4" /> Copy summary
          </Button>
          <div className="min-w-0">
            <ExportMenu
              onExportCSV={exportDaily}
              csvLabel="Daily log (CSV)"
              extraItems={exportItems}
              disabled={pending || !hasTime}
              disabledReason={pending ? "Export is available once the report has loaded" : "Nothing was tracked in this period"}
            />
          </div>
        </div>
      }
    >
      <PeriodToolbar
        preset={preset}
        onPreset={choosePreset}
        title={selectionTitle(selection, anchor, timeZone)}
        offset={selection.kind === "preset" ? selection.offset : 0}
        onStep={step}
        projectId={projectId}
        onProject={(v) => {
          setProjectId(v);
          touch();
        }}
        memberId={memberId}
        onMember={(v) => {
          setMemberId(v);
          touch();
        }}
        liveWorkerId={liveWorkerId}
        extra={<Segmented label="Hours format" options={HOURS_FORMATS} value={format} onChange={setFormat} />}
        allowCustom
        range={range}
        onRange={applyRange}
        suggestedRange={{ from: period.fromKey, to: period.toKey }}
        today={today}
        timeZone={timeZone}
        spanLabel={formatDayRange(period.fromKey, period.toKey)}
      />

      {isLive && (
        <div className="no-print">
          <LiveReportNotice loading={loading} error={live.error} onRetry={live.retry} />
        </div>
      )}

      <div className="work-report space-y-5">
        <Hero
          workspaceName={workspaceName}
          period={periodText}
          dayCount={summary.dayCount}
          zone={zoneText}
          scope={scope}
          generated={formatStampInZone(generatedAt, timeZone)}
          total={pending ? "—" : total}
          change={change}
          comparison={comparison}
        />

        <KpiTiles summary={summary} log={log} memberCount={memberCount} perMember={perMember} pending={pending} fmt={fmt} />

        {pending ? (
          loading ? <LoadingSkeleton /> : null
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
          <>
            <div className="grid grid-cols-1 gap-5 xl:grid-cols-3">
              <Card className="wr-keep xl:col-span-2">
                <CardHeader>
                  <div>
                    <CardTitle>{team ? "Team hours per day" : "Hours per day"}</CardTitle>
                    <p className="mt-0.5 text-xs text-muted-foreground">
                      {periodText} · days in {zoneText}
                    </p>
                  </div>
                  <div className="flex flex-wrap items-center gap-3">
                    {chartView === "bars" && (
                      <ChartLegend showTarget={showTarget} average={summary.averagePerWorkedDay > 0 ? fmt(summary.averagePerWorkedDay) : null} />
                    )}
                    <Segmented
                      label="Chart view"
                      className="no-print"
                      value={chartView}
                      onChange={(v) => setChartChoice(v)}
                      options={[
                        { key: "bars", label: "Bars", icon: BarChart3 },
                        { key: "calendar", label: "Calendar", icon: LayoutGrid },
                      ]}
                    />
                  </div>
                </CardHeader>
                <CardContent>
                  {chartView === "bars" ? (
                    <HoursChart
                      data={chartSeries(log)}
                      showTarget={showTarget}
                      averageMinutes={summary.averagePerWorkedDay}
                      dense={log.length > 14}
                      fmt={fmt}
                    />
                  ) : (
                    <CalendarHeat log={log} today={today} fmt={fmt} />
                  )}
                  <p className="sr-only">
                    {summary.workedDays} of {summary.dayCount} days have tracked time; the busiest day was{" "}
                    {summary.busiest ? `${formatDayLabel(summary.busiest.date)} with ${fmt(summary.busiest.minutes)}` : "none"}.
                    The daily log below lists every day.
                  </p>
                </CardContent>
              </Card>

              <Highlights items={highlights} />
            </div>

            <div className="grid grid-cols-1 gap-5 xl:grid-cols-3">
              <div className="min-w-0 xl:col-span-2">
                <DailyLog
                  log={log}
                  summary={summary}
                  period={periodText}
                  zone={zoneText}
                  scale={dayScale}
                  perMember={perMember}
                  team={team}
                  today={today}
                  openDay={openDay}
                  onToggleDay={(d) => setOpenDay((cur) => (cur === d ? null : d))}
                  hideEmpty={hideEmpty}
                  onHideEmpty={setHideEmpty}
                  format={format}
                  fmt={fmt}
                  renderDay={(row) => (
                    <DayDetail
                      row={row}
                      team={team}
                      members={membersOnDay(aggregates.memberDays, row.date, nameOf)}
                      blocks={blocks}
                      blocksState={blocksState}
                      covered={coversDay(blocks, truncated, row.date, calendar)}
                      calendar={calendar}
                      notes={notesPerDay.find((d) => d.id === row.date)?.notes ?? []}
                      nameOf={nameOf}
                      localText={localText}
                      fmt={fmt}
                    />
                  )}
                />
              </div>
              <div className="min-w-0 space-y-5">
                {projectRows.length > 0 && <ProjectBreakdown rows={projectRows} format={format} fmt={fmt} />}
                {showMatrix && <TeamCard matrix={matrix} format={format} fmt={fmt} />}
              </div>
            </div>

            <NotesCard
              rows={notes}
              byDay={notesPerDay}
              byMember={team ? notesPerMember : []}
              team={team}
              state={blocksState}
              sampleNote={sampleNote}
              onRetry={blockLoad.retry}
              nameOf={nameOf}
              fmt={fmt}
            />

            {showMatrix && (
              <TeamTimesheet
                matrix={matrix}
                period={periodText}
                zone={zoneText}
                scale={Math.max(matrix.maxCell, TARGET_DAY_MINUTES)}
                format={format}
                fmt={fmt}
              />
            )}
            {matrixMissing && (
              <Card variant="quiet" className="px-5 py-4 text-sm text-muted-foreground">
                The per-member timesheet couldn&apos;t be loaded. The daily totals above are complete.
              </Card>
            )}
          </>
        )}

        <Footnote zone={zoneText} />
      </div>
    </ReportShell>
  );
}

/* ----------------------------- Hero ----------------------------- */

function ChangePill({ change, comparison }: { change: Change; comparison: string }) {
  const Icon = change.direction === "up" ? ArrowUpRight : change.direction === "down" ? ArrowDownRight : ArrowRight;
  const tone =
    change.direction === "up"
      ? "bg-success/12 text-success-on-tint"
      : change.direction === "down"
        ? "bg-danger/12 text-danger-on-tint"
        : "bg-muted text-muted-on-tint";
  const sign = change.percent > 0 ? "+" : "";
  return (
    <span className={cn("print-exact inline-flex items-center gap-1 rounded-full px-2.5 py-1 text-xs font-semibold", tone)}>
      <Icon aria-hidden className="h-3.5 w-3.5" />
      {sign}
      {change.percent}%<span className="font-normal opacity-90"> {comparison}</span>
    </span>
  );
}

function Hero({
  workspaceName,
  period,
  dayCount,
  zone,
  scope,
  generated,
  total,
  change,
  comparison,
}: {
  workspaceName: string;
  period: string;
  dayCount: number;
  zone: string;
  scope: string;
  generated: string;
  total: string;
  change: Change | null;
  comparison: string;
}) {
  return (
    <section
      aria-labelledby="work-report-period"
      className="wr-keep relative overflow-hidden rounded-3xl border border-border bg-card card-elev"
    >
      {/* Soft brand glow, top right. */}
      <div
        aria-hidden
        className="print-exact pointer-events-none absolute inset-0 bg-[radial-gradient(90%_140%_at_100%_0%,color-mix(in_srgb,var(--primary)_16%,transparent)_0%,transparent_60%)]"
      />
      <div
        aria-hidden
        className="print-exact pointer-events-none absolute inset-0 opacity-[0.35] [background-image:radial-gradient(color-mix(in_srgb,var(--primary)_22%,transparent)_1px,transparent_1px)] [background-size:18px_18px] [mask-image:linear-gradient(to_left,black,transparent_55%)]"
      />
      <div className="relative grid gap-6 p-5 sm:p-7 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-end">
        <div className="min-w-0">
          <div className="flex min-w-0 items-center gap-3">
            <div
              aria-hidden
              className="print-exact flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl bg-gradient-to-br from-primary to-[#6d5efc] font-display text-lg font-bold text-white shadow-sm"
            >
              {workspaceName.charAt(0).toUpperCase()}
            </div>
            <div className="min-w-0">
              <p className="truncate font-display text-base font-semibold tracking-tight">{workspaceName}</p>
              <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-primary-strong">Work Report</p>
            </div>
          </div>

          <h2 id="work-report-period" className="mt-6 font-display text-2xl font-bold tracking-tight text-ink sm:text-[2rem] sm:leading-tight">
            {period}
          </h2>

          <dl className="mt-4 flex flex-wrap items-center gap-2 text-xs">
            <div className="inline-flex items-center gap-1.5 rounded-full border border-border bg-card/80 px-3 py-1.5">
              <dt>
                <CalendarDays aria-hidden className="h-3.5 w-3.5 text-primary" />
                <span className="sr-only">Period</span>
              </dt>
              <dd className="font-medium">
                {dayCount} {dayCount === 1 ? "day" : "days"}
              </dd>
            </div>
            <div className="inline-flex min-w-0 max-w-full items-center gap-1.5 rounded-full border border-border bg-card/80 px-3 py-1.5">
              <dt>
                <Globe aria-hidden className="h-3.5 w-3.5 text-primary" />
                <span className="sr-only">Days are calendar days in</span>
              </dt>
              <dd className="truncate font-medium" title={`Days are calendar days in ${zone}`}>
                {zone}
              </dd>
            </div>
            <div className="inline-flex min-w-0 max-w-full items-center gap-1.5 rounded-full border border-border bg-card/80 px-3 py-1.5">
              <dt>
                <Filter aria-hidden className="h-3.5 w-3.5 text-primary" />
                <span className="sr-only">Scope</span>
              </dt>
              <dd className="truncate font-medium" title={scope}>
                {scope}
              </dd>
            </div>
            <div className="inline-flex items-center gap-1.5 rounded-full border border-border bg-card/80 px-3 py-1.5 text-muted-foreground">
              <dt>Generated</dt>
              <dd className="font-medium text-foreground">{generated}</dd>
            </div>
          </dl>
        </div>

        <div className="lg:text-right">
          <p className="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">Total hours worked</p>
          <p className="mt-1 font-display text-4xl font-bold tracking-tight text-ink tabular-nums sm:text-5xl">{total}</p>
          {change && (
            <div className="mt-2.5 flex lg:justify-end">
              <ChangePill change={change} comparison={comparison} />
            </div>
          )}
        </div>
      </div>
    </section>
  );
}

/* ----------------------------- KPI tiles ----------------------------- */

const tileAccent = {
  blue: "bg-primary/10 text-primary",
  green: "bg-success/12 text-success-on-tint",
  violet: "bg-violet-500/12 text-violet-700 dark:text-violet-300",
  amber: "bg-warning/15 text-warning-on-tint",
  cyan: "bg-cyan-500/12 text-cyan-700 dark:text-cyan-300",
} as const;

function Tile({
  label,
  icon: Icon,
  accent,
  value,
  unit,
  sub,
  className,
  children,
}: {
  label: string;
  icon: LucideIcon;
  accent: keyof typeof tileAccent;
  value: string;
  unit?: string;
  sub?: string;
  className?: string;
  children?: React.ReactNode;
}) {
  return (
    <div className={cn("flex min-w-0 flex-col rounded-2xl border border-border bg-card p-4 card-elev sm:p-5 print:p-3", className)}>
      <dt className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
        <span aria-hidden className={cn("print-exact flex h-7 w-7 shrink-0 items-center justify-center rounded-lg", tileAccent[accent])}>
          <Icon className="h-3.5 w-3.5" />
        </span>
        <span className="min-w-0 leading-snug">{label}</span>
      </dt>
      <dd className="mt-3 font-display text-2xl font-bold tracking-tight text-ink tabular-nums [overflow-wrap:anywhere] sm:text-[1.7rem]">
        {value}
        {unit && <span className="ml-1 text-sm font-medium tracking-normal text-muted-foreground">{unit}</span>}
      </dd>
      {children && <dd className="mt-2.5">{children}</dd>}
      {sub && <dd className="mt-1.5 text-xs leading-snug text-muted-foreground">{sub}</dd>}
    </div>
  );
}

/** One dot per day of the period: filled where time was tracked. */
function DayDots({ log }: { log: DayRow[] }) {
  return (
    <div aria-hidden className="flex flex-wrap gap-1">
      {log.map((r) => (
        <span
          key={r.date}
          title={r.label}
          className={cn(
            "print-exact h-2 w-2 rounded-full",
            r.minutes > 0 ? (r.weekend ? "bg-primary/45" : "bg-primary") : "bg-border",
          )}
        />
      ))}
    </div>
  );
}

function MiniBar({ percent, className }: { percent: number; className: string }) {
  return (
    <div aria-hidden className="print-exact h-1.5 w-full overflow-hidden rounded-full bg-muted">
      <div className={cn("print-exact h-full rounded-full", className)} style={{ width: `${Math.max(0, Math.min(100, percent))}%` }} />
    </div>
  );
}

function KpiTiles({
  summary,
  log,
  memberCount,
  perMember,
  pending,
  fmt,
}: {
  summary: WorkSummary;
  log: DayRow[];
  memberCount: number;
  /** A team's average per member-day (null for one member, or without the per-member series). */
  perMember: { minutes: number; memberDays: number } | null;
  pending: boolean;
  fmt: (minutes: number) => string;
}) {
  const dash = "—";
  const team = memberCount > 1;
  const average = perMember ? perMember.minutes : summary.averagePerWorkedDay;
  const hasTime = summary.totalMinutes > 0;
  const activity = summary.averageActivity;
  const idle = summary.idleMinutes;
  const idlePercent = idle === null ? 0 : idleShare(idle, summary.totalMinutes);
  const levelBar: Record<ActivityLevel, string> = { good: "bg-success", ok: "bg-warning", low: "bg-danger", none: "bg-muted-foreground" };
  return (
    <dl aria-label="Key figures" className="wr-keep grid grid-cols-2 gap-3 sm:grid-cols-3 sm:gap-4 lg:grid-cols-5 print:grid-cols-5">
      <Tile
        label="Days worked"
        icon={CalendarCheck}
        accent="blue"
        value={pending ? dash : String(summary.workedDays)}
        unit={pending ? undefined : `of ${summary.dayCount}`}
        sub={pending ? undefined : `${summary.weekdayCount} weekdays in the period`}
      >
        {!pending && <DayDots log={log} />}
      </Tile>
      <Tile
        label={perMember ? "Average per member-day" : "Average per worked day"}
        icon={Timer}
        accent="violet"
        value={pending ? dash : fmt(average)}
        sub={
          pending
            ? undefined
            : perMember
              ? `over ${perMember.memberDays} member-${perMember.memberDays === 1 ? "day" : "days"} worked`
              : team
                ? "team hours per day"
                : summary.targetDays
                  ? `${summary.targetDays} ${summary.targetDays === 1 ? "day" : "days"} at 8h or more`
                  : "of an 8h day"
        }
      >
        {!pending && <MiniBar percent={barPercent(average, TARGET_DAY_MINUTES)} className="bg-violet-500" />}
      </Tile>
      <Tile
        label="Average activity"
        icon={Gauge}
        accent="green"
        value={pending || !hasTime ? dash : formatActivity(activity)}
        sub={pending || !hasTime ? undefined : activity === null ? "Not measured" : `${activityLevelLabel[summary.level]} · keyboard & mouse`}
      >
        {!pending && hasTime && activity !== null && <MiniBar percent={activity} className={levelBar[summary.level]} />}
      </Tile>
      <Tile
        label="Idle time"
        icon={Coffee}
        accent="amber"
        value={pending || !hasTime || idle === null ? dash : fmt(idle)}
        sub={pending || !hasTime ? undefined : idle === null ? "Not measured" : `${idlePercent}% of tracked time · no input`}
      >
        {!pending && hasTime && idle !== null && <MiniBar percent={idlePercent} className="bg-warning" />}
      </Tile>
      {team && !pending ? (
        <Tile label="Members" icon={Users} accent="cyan" value={String(memberCount)} sub="with tracked time" className="col-span-2 sm:col-span-1" />
      ) : (
        <Tile
          label="Tracked sessions"
          icon={Zap}
          accent="cyan"
          value={pending ? dash : String(summary.sessions)}
          sub={pending ? undefined : "blocks recorded by the desktop app"}
          className="col-span-2 sm:col-span-1"
        />
      )}
    </dl>
  );
}

/* ----------------------------- Hours per day ----------------------------- */

const axisColor = "var(--muted-foreground)";

function ChartLegend({ showTarget, average }: { showTarget: boolean; average: string | null }) {
  return (
    <div aria-hidden className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
      <span className="flex items-center gap-1.5">
        <span className="print-exact h-2.5 w-2.5 rounded-sm bg-primary" /> Weekday
      </span>
      <span className="flex items-center gap-1.5">
        <span className="print-exact h-2.5 w-2.5 rounded-sm bg-primary/35" /> Weekend
      </span>
      {average && (
        <span className="flex items-center gap-1.5">
          <span className="w-3 border-t-2 border-dotted border-muted-foreground" /> Avg {average}
        </span>
      )}
      {showTarget && (
        <span className="flex items-center gap-1.5">
          <span className="w-3 border-t-2 border-dashed border-success" /> 8h target
        </span>
      )}
    </div>
  );
}

function DayTooltip({
  active,
  payload,
  fmt,
}: {
  active?: boolean;
  payload?: ReadonlyArray<{ payload?: unknown }>;
  fmt: (minutes: number) => string;
}) {
  const day = active && payload?.length ? (payload[0]?.payload as ChartDay | undefined) : undefined;
  if (!day) return null;
  return (
    <div className="min-w-[160px] rounded-xl border border-border bg-card px-3 py-2 text-xs text-card-foreground card-elev-lg">
      <div className="font-semibold">
        {formatDayLabel(day.date, { year: true })}
        {day.weekend && <span className="font-normal text-muted-foreground"> · Weekend</span>}
      </div>
      <div className="mt-1.5 flex items-center justify-between gap-4">
        <span className="text-muted-foreground">Hours</span>
        <span className="font-semibold tabular-nums">{fmt(day.minutes)}</span>
      </div>
      {day.idleMinutes !== null && day.minutes > 0 && (
        <div className="flex items-center justify-between gap-4">
          <span className="text-muted-foreground">Idle</span>
          <span className="font-semibold tabular-nums">{fmt(day.idleMinutes)}</span>
        </div>
      )}
      <div className="flex items-center justify-between gap-4">
        <span className="text-muted-foreground">Activity</span>
        <span className="font-semibold tabular-nums">{day.minutes > 0 ? formatActivity(day.activity) : "—"}</span>
      </div>
    </div>
  );
}

function HoursChart({
  data,
  showTarget,
  averageMinutes,
  dense,
  fmt,
}: {
  data: ChartDay[];
  showTarget: boolean;
  averageMinutes: number;
  dense: boolean;
  fmt: (minutes: number) => string;
}) {
  const recharts = useRecharts();
  const md = useIsMdUp();
  const reduced = usePrefersReducedMotion();
  const id = useId().replace(/:/g, "");
  const maxHours = data.reduce((m, d) => Math.max(m, d.hours), 0);
  const axis = hourTicks(maxHours, showTarget ? TARGET_DAY_MINUTES / 60 : 0);
  const averageHours = averageMinutes / 60;
  const labels = md && data.length <= 10;
  if (!recharts) {
    return (
      <div className="wr-chart h-[230px] w-full min-w-0 sm:h-[280px]">
        <ChartLoading />
      </div>
    );
  }
  const { ResponsiveContainer, BarChart, Bar, Cell, LabelList, CartesianGrid, ReferenceLine, XAxis, YAxis, Tooltip } = recharts;
  return (
    <div className="wr-chart h-[230px] w-full min-w-0 sm:h-[280px]">
      <ResponsiveContainer width="100%" height="100%">
        <BarChart data={data} margin={{ top: labels ? 24 : 16, right: 8, left: md ? -6 : -12, bottom: 0 }}>
          <defs>
            <linearGradient id={`${id}-weekday`} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" style={{ stopColor: "var(--primary)", stopOpacity: 1 }} />
              <stop offset="100%" style={{ stopColor: "var(--primary)", stopOpacity: 0.62 }} />
            </linearGradient>
            <linearGradient id={`${id}-weekend`} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" style={{ stopColor: "var(--primary)", stopOpacity: 0.42 }} />
              <stop offset="100%" style={{ stopColor: "var(--primary)", stopOpacity: 0.2 }} />
            </linearGradient>
          </defs>
          <CartesianGrid vertical={false} stroke="var(--border)" strokeDasharray="3 4" />
          <XAxis
            dataKey="date"
            tickLine={false}
            axisLine={false}
            tick={{ fill: axisColor, fontSize: md ? 12 : 10 }}
            tickFormatter={(d) => formatAxisDay(String(d), dense)}
            interval="preserveStartEnd"
            minTickGap={10}
          />
          <YAxis
            tickLine={false}
            axisLine={false}
            tick={{ fill: axisColor, fontSize: md ? 12 : 10 }}
            width={md ? 44 : 38}
            domain={[0, axis.top]}
            ticks={axis.ticks}
            tickFormatter={(v) => `${v}h`}
          />
          <Tooltip
            cursor={{ fill: "var(--muted)", radius: 8 }}
            content={({ active, payload }) => <DayTooltip active={active} payload={payload} fmt={fmt} />}
          />
          {showTarget && (
            <ReferenceLine y={TARGET_DAY_MINUTES / 60} stroke="var(--success)" strokeDasharray="5 5" strokeOpacity={0.8} />
          )}
          {averageHours > 0 && <ReferenceLine y={averageHours} stroke="var(--muted-foreground)" strokeDasharray="2 4" />}
          <Bar dataKey="hours" name="Hours" radius={[8, 8, 3, 3]} maxBarSize={46} isAnimationActive={!reduced}>
            {data.map((d) => (
              <Cell key={d.date} fill={`url(#${id}-${d.weekend ? "weekend" : "weekday"})`} />
            ))}
            {labels && (
              <LabelList dataKey="minutes" content={(props) => <BarValue {...props} fmt={fmt} />} />
            )}
          </Bar>
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
}

/** A bar's value above it, on one line. */
function BarValue({
  x,
  y,
  width,
  value,
  fmt,
}: {
  x?: number | string;
  y?: number | string;
  width?: number | string;
  value?: unknown;
  fmt: (minutes: number) => string;
}) {
  const minutes = Number(value);
  if (!(minutes > 0)) return null;
  return (
    <text
      x={Number(x) + Number(width) / 2}
      y={Number(y) - 7}
      textAnchor="middle"
      // A card-coloured halo keeps the average line from striking through the value.
      style={{ fill: "var(--muted-foreground)", fontSize: 11, fontWeight: 600, stroke: "var(--card)", strokeWidth: 4, paintOrder: "stroke" }}
    >
      {fmt(minutes)}
    </text>
  );
}

/** Hours per day laid out as a Mon–Sun calendar, shaded by how much was worked. */
function CalendarHeat({ log, today, fmt }: { log: DayRow[]; today: string | null; fmt: (minutes: number) => string }) {
  const byDate = new Map(log.map((r) => [r.date, r]));
  const weeks = calendarWeeks(log.map((r) => r.date));
  const max = Math.max(TARGET_DAY_MINUTES, ...log.map((r) => r.minutes));
  return (
    <div
      role="img"
      aria-label="Calendar of hours worked per day. The daily log below lists every day."
      className="wr-calendar"
    >
      <div className="grid grid-cols-7 gap-1.5 sm:gap-2">
        {["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].map((d) => (
          <div key={d} className="pb-1 text-center text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
            {d}
          </div>
        ))}
        {weeks.flat().map((date, i) => {
          if (!date) return <div key={`pad-${i}`} className="aspect-[4/3] rounded-xl sm:aspect-[5/3]" />;
          const row = byDate.get(date);
          const minutes = row?.minutes ?? 0;
          const level = heatLevel(minutes, max);
          return (
            <div
              key={date}
              title={`${formatDayLabel(date, { year: true })}: ${minutes > 0 ? fmt(minutes) : "no time tracked"}`}
              className={cn(
                "print-exact flex aspect-[4/3] min-w-0 flex-col justify-between rounded-xl border p-1.5 sm:aspect-[5/3] sm:p-2",
                level === 0 ? "border-dashed border-border bg-transparent" : "border-transparent",
                heatClass[level],
                date === today && "ring-2 ring-primary ring-offset-2 ring-offset-card",
              )}
            >
              <span className={cn("text-[11px] font-semibold leading-none", level === 0 && "text-muted-foreground")}>
                {Number(date.slice(8, 10))}
              </span>
              <span className="truncate text-[10px] font-semibold leading-none tabular-nums sm:text-xs">
                {minutes > 0 ? fmt(minutes) : ""}
              </span>
            </div>
          );
        })}
      </div>
      <div aria-hidden className="mt-3 flex items-center justify-end gap-1.5 text-xs text-muted-foreground">
        <span>Less</span>
        {([1, 2, 3, 4] as const).map((l) => (
          <span key={l} className={cn("print-exact h-3 w-4 rounded-sm", heatClass[l])} />
        ))}
        <span>More</span>
      </div>
    </div>
  );
}

/* ----------------------------- Highlights ----------------------------- */

const highlightIcon: Record<HighlightKind, { icon: LucideIcon; className: string }> = {
  busiest: { icon: Trophy, className: "bg-primary/10 text-primary" },
  project: { icon: FolderKanban, className: "bg-violet-500/12 text-violet-700 dark:text-violet-300" },
  member: { icon: Users, className: "bg-warning/15 text-warning-on-tint" },
  fullDays: { icon: Target, className: "bg-success/12 text-success-on-tint" },
  weekend: { icon: Sun, className: "bg-info/12 text-info-on-tint" },
};

function Highlights({ items }: { items: Highlight[] }) {
  return (
    <Card className="wr-keep flex flex-col">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Sparkles aria-hidden className="h-4 w-4 text-primary" /> Highlights
        </CardTitle>
      </CardHeader>
      <CardContent className="flex-1">
        <ul className={cn("grid gap-4 sm:grid-cols-2 xl:flex xl:h-full xl:flex-col", items.length >= 4 && "xl:justify-between")}>
          {items.map((h) => {
            const { icon: Icon, className } = highlightIcon[h.kind];
            return (
              <li key={h.kind} className="flex items-start gap-3">
                <span aria-hidden className={cn("print-exact flex h-9 w-9 shrink-0 items-center justify-center rounded-xl", className)}>
                  <Icon className="h-4 w-4" />
                </span>
                <div className="min-w-0">
                  <p className="text-xs text-muted-foreground">{h.label}</p>
                  <p className="truncate text-sm font-semibold" title={h.value}>
                    {h.value}
                  </p>
                  <p className="text-xs text-muted-foreground">{h.detail}</p>
                </div>
              </li>
            );
          })}
        </ul>
      </CardContent>
    </Card>
  );
}

/* ----------------------------- Daily log ----------------------------- */

function Dash({ label = "No time tracked" }: { label?: string }) {
  return (
    <>
      <span aria-hidden>—</span>
      <span className="sr-only">{label}</span>
    </>
  );
}

function DateChip({ row, worked, today }: { row: DayRow; worked: boolean; today: boolean }) {
  return (
    <span
      aria-hidden
      className={cn(
        "print-exact hidden h-10 w-10 shrink-0 flex-col items-center justify-center rounded-xl leading-none sm:flex print:flex",
        today ? "bg-primary text-primary-foreground" : worked ? "bg-primary/10 text-primary-strong" : "bg-muted text-muted-foreground",
      )}
    >
      <span className="text-[9px] font-semibold uppercase tracking-wider">{row.weekday}</span>
      <span className="mt-0.5 text-sm font-bold">{Number(row.date.slice(8, 10))}</span>
    </span>
  );
}

function DailyLog({
  log,
  summary,
  period,
  zone,
  scale,
  team,
  perMember,
  today,
  openDay,
  onToggleDay,
  hideEmpty,
  onHideEmpty,
  format,
  fmt,
  renderDay,
}: {
  log: DayRow[];
  summary: WorkSummary;
  period: string;
  zone: string;
  scale: number;
  team: boolean;
  perMember: { minutes: number; memberDays: number } | null;
  today: string | null;
  openDay: string | null;
  onToggleDay: (date: string) => void;
  /** Leave days without tracked time out of the table (the totals still cover every day). */
  hideEmpty: boolean;
  onHideEmpty: (hide: boolean) => void;
  format: HoursFormat;
  fmt: (minutes: number) => string;
  /** What a worked day shows when opened: who worked, when, and the day's timeline. */
  renderDay: (row: DayRow) => React.ReactNode;
}) {
  const emptyDays = log.filter((r) => r.minutes <= 0).length;
  const rows = hideEmpty ? log.filter((r) => r.minutes > 0) : log;
  const daysWorked = `${summary.workedDays} ${summary.workedDays === 1 ? "day" : "days"} worked`;
  const footNote = perMember
    ? `${daysWorked} · ${perMember.memberDays} member-${perMember.memberDays === 1 ? "day" : "days"}`
    : `${daysWorked} · avg ${fmt(summary.averagePerWorkedDay)} per day`;
  const idleKnown = summary.idleMinutes !== null;
  const columns = 7;
  return (
    <Card>
      <CardHeader>
        <div>
          <CardTitle>Daily log</CardTitle>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {summary.workedDays} of {summary.dayCount} {summary.dayCount === 1 ? "day" : "days"} with tracked time
            <span className="no-print"> · open a day to see who worked, and when</span>
            {hideEmpty && emptyDays > 0 && (
              <span>
                {" "}
                · {emptyDays} empty {emptyDays === 1 ? "day" : "days"} hidden
              </span>
            )}
          </p>
        </div>
        {emptyDays > 0 && (
          <button
            type="button"
            aria-pressed={hideEmpty}
            onClick={() => onHideEmpty(!hideEmpty)}
            className="no-print inline-flex h-8 items-center gap-1.5 rounded-lg border border-border px-3 text-xs font-medium text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {hideEmpty ? "Show empty days" : `Hide empty days (${emptyDays})`}
          </button>
        )}
      </CardHeader>
      <CardContent className="pt-3">
        <div className="wr-scroll relative overflow-x-auto overscroll-x-contain">
          <table className="wr-table w-full border-collapse text-sm">
            <caption className="sr-only">
              Hours worked per day, {period}. Days are calendar days in {zone}.
            </caption>
            <thead>
              <tr className="border-b border-border text-[11px] uppercase tracking-wider text-muted-foreground">
                <th scope="col" className="py-2 pr-2 text-left font-semibold sm:pr-3">Day</th>
                <th scope="col" className="px-2 py-2 text-right font-semibold sm:px-3">{team ? "Team hours" : "Hours"}</th>
                <th scope="col" className="hidden w-[28%] px-2 py-2 text-left font-semibold sm:table-cell sm:px-3 print:table-cell">
                  <span className="sr-only">Hours, as a bar</span>
                </th>
                <th scope="col" className="hidden px-2 py-2 text-right font-semibold sm:table-cell sm:px-3 print:table-cell">Idle</th>
                <th scope="col" className="px-2 py-2 text-right font-semibold sm:px-3">Activity</th>
                <th scope="col" className="hidden py-2 pl-2 text-right font-semibold sm:table-cell sm:pl-3 print:table-cell">Sessions</th>
                <th scope="col" className="w-10 py-2 pl-2 print:hidden">
                  <span className="sr-only">Details</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => {
                const worked = r.minutes > 0;
                const isToday = r.date === today;
                const open = worked && openDay === r.date;
                const detailId = `wr-day-${r.date}`;
                return (
                  <Fragment key={r.date}>
                    <tr
                      className={cn(
                        "print-exact border-b border-border/60 transition-colors hover:bg-muted/40",
                        r.weekend && "bg-muted/35",
                        open && "bg-accent/40 hover:bg-accent/40",
                        !worked && "text-muted-foreground",
                      )}
                    >
                      <th scope="row" className="py-2 pr-2 text-left font-medium sm:pr-3">
                        <span className="flex items-center gap-3">
                          <DateChip row={r} worked={worked} today={isToday} />
                          <span className="min-w-0">
                            <span className="block whitespace-nowrap">{r.label}</span>
                            {(r.weekend || isToday) && (
                              <span className="block text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                                {isToday ? "Today" : "Weekend"}
                              </span>
                            )}
                          </span>
                        </span>
                      </th>
                      <td className={cn("whitespace-nowrap px-2 py-2 text-right tabular-nums sm:px-3", worked && "font-semibold text-foreground")}>
                        {worked ? fmt(r.minutes) : <Dash />}
                      </td>
                      <td className="hidden px-2 py-2 sm:table-cell sm:px-3 print:table-cell">
                        <div aria-hidden className="print-exact h-2 w-full overflow-hidden rounded-full bg-muted">
                          {worked && (
                            <div
                              className={cn(
                                "print-exact h-full rounded-full bg-gradient-to-r",
                                r.weekend ? "from-primary/30 to-primary/50" : "from-primary/70 to-primary",
                              )}
                              style={{ width: `${barPercent(r.minutes, scale)}%` }}
                            />
                          )}
                        </div>
                      </td>
                      <td className="hidden whitespace-nowrap px-2 py-2 text-right tabular-nums sm:table-cell sm:px-3 print:table-cell">
                        {worked && r.idleMinutes !== null ? fmt(r.idleMinutes) : <Dash label={worked ? "Not measured" : "No time tracked"} />}
                      </td>
                      <td className="px-2 py-2 text-right sm:px-3">
                        {worked && r.activity !== null ? (
                          <Badge tone={levelTone[r.level]} className="print-exact tabular-nums">
                            {r.activity}%
                          </Badge>
                        ) : (
                          <Dash label={worked ? "Not measured" : "No activity"} />
                        )}
                      </td>
                      <td className="hidden py-2 pl-2 text-right tabular-nums sm:table-cell sm:pl-3 print:table-cell">
                        {worked || r.sessions > 0 ? r.sessions : <Dash label="No sessions" />}
                      </td>
                      <td className="py-2 pl-2 text-right print:hidden">
                        {worked && (
                          <button
                            type="button"
                            aria-expanded={open}
                            aria-controls={detailId}
                            aria-label={`${open ? "Hide" : "Show"} who worked on ${r.label}`}
                            onClick={() => onToggleDay(r.date)}
                            className="inline-flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                          >
                            <ChevronDown className={cn("h-4 w-4 transition-transform motion-reduce:transition-none", open && "rotate-180")} />
                          </button>
                        )}
                      </td>
                    </tr>
                    {open && (
                      <tr id={detailId} className="border-b border-border/60 bg-accent/20 print:hidden">
                        <td colSpan={columns} className="px-2 py-3 sm:px-3">
                          {renderDay(r)}
                        </td>
                      </tr>
                    )}
                  </Fragment>
                );
              })}
            </tbody>
            <tfoot>
              <tr className="border-t-2 border-foreground/70 font-bold text-foreground">
                <th scope="row" className="py-3 pr-2 text-left sm:pr-3">Total</th>
                <td className="whitespace-nowrap px-2 py-3 text-right tabular-nums sm:px-3">
                  {formatTotal(
                    log.map((r) => r.minutes),
                    format,
                  )}
                </td>
                <td className="hidden px-2 py-3 text-xs font-medium text-muted-foreground sm:table-cell sm:px-3 print:table-cell">
                  {footNote}
                </td>
                <td className="hidden whitespace-nowrap px-2 py-3 text-right tabular-nums sm:table-cell sm:px-3 print:table-cell">
                  {idleKnown ? (
                    formatTotal(
                      log.map((r) => r.idleMinutes ?? 0),
                      format,
                    )
                  ) : (
                    <Dash label="Not measured" />
                  )}
                </td>
                <td className="px-2 py-3 text-right sm:px-3">
                  {summary.totalMinutes > 0 && summary.averageActivity !== null ? (
                    <Badge tone={levelTone[summary.level]} className="print-exact tabular-nums">
                      {summary.averageActivity}%
                    </Badge>
                  ) : (
                    <Dash label="Not measured" />
                  )}
                </td>
                <td className="hidden py-3 pl-2 text-right tabular-nums sm:table-cell sm:pl-3 print:table-cell">{summary.sessions}</td>
                <td className="print:hidden" />
              </tr>
            </tfoot>
          </table>
        </div>
      </CardContent>
    </Card>
  );
}

/* ----------------------------- A day, opened ----------------------------- */

/** The gap kinds present in a day's lanes (for its legend). */
function gapKindsOf(lanes: ReturnType<typeof dayTimeline>["lanes"]): GapKind[] {
  const kinds = new Set<GapKind>();
  for (const lane of lanes) for (const g of lane.gaps) kinds.add(gapKind(g.reason).kind);
  return (["break", "idle", "lock", "sleep", "off", "untracked"] as const).filter((k) => kinds.has(k));
}

function DayDetail({
  row,
  team,
  members,
  blocks,
  blocksState,
  covered,
  calendar,
  notes,
  nameOf,
  localText,
  fmt,
}: {
  row: DayRow;
  team: boolean;
  members: MemberDay[];
  blocks: Block[];
  blocksState: BlocksState;
  /** The loaded blocks include all of this day. */
  covered: boolean;
  calendar: ZoneCalendar;
  notes: NoteEntry[];
  nameOf: (userId: string, hint: string | null) => string;
  localText: (userId: string, firstStart: number | null, lastEnd: number | null) => string | null;
  fmt: (minutes: number) => string;
}) {
  const timeline = useMemo(() => (blocksState === "ready" && covered ? dayTimeline(blocks, row.date, calendar) : null), [blocks, blocksState, covered, row.date, calendar]);
  const top = members[0]?.minutes ?? 0;
  const byUser = new Map(members.map((m) => [m.userId, m]));
  return (
    <div className="space-y-4">
      {members.length > 0 && (
        <ul aria-label={`Who worked on ${row.label}`} className={cn("grid grid-cols-1 gap-2", team && "sm:grid-cols-2")}>
          {members.map((m) => {
            const when = localText(m.userId, m.firstStart, m.lastEnd);
            return (
              <li key={m.userId} className="flex items-center gap-3 rounded-xl bg-card px-3 py-2 card-elev">
                <Avatar name={m.name} size="sm" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-medium">{m.name}</span>
                  <span className="block truncate text-[11px] text-muted-foreground tabular-nums">
                    {[when, m.idleMinutes !== null ? `idle ${fmt(m.idleMinutes)}` : null].filter(Boolean).join(" · ") || " "}
                  </span>
                  {team && <MiniBar percent={barPercent(m.minutes, top)} className="bg-primary" />}
                </span>
                <span className="shrink-0 text-sm font-semibold tabular-nums">{fmt(m.minutes)}</span>
              </li>
            );
          })}
        </ul>
      )}

      <div>
        <p className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">Timeline</p>
        {blocksState === "loading" ? (
          <p role="status" className="text-xs text-muted-foreground">
            Loading the day&apos;s timeline…
          </p>
        ) : blocksState === "error" ? (
          <p className="text-xs text-muted-foreground">The day&apos;s timeline couldn&apos;t be loaded. The hours above are complete.</p>
        ) : !covered || !timeline ? (
          <p className="text-xs text-muted-foreground">
            This day is older than the newest {MAX_BLOCKS.toLocaleString("en")} blocks of the period, so its timeline isn&apos;t shown.
          </p>
        ) : timeline.lanes.length === 0 ? (
          <p className="text-xs text-muted-foreground">No blocks to draw for this day.</p>
        ) : (
          <div className="space-y-2">
            <DayTimelineChart
              compact
              timeline={timeline}
              meta={(userId) => {
                const m = byUser.get(userId);
                return { name: m?.name ?? nameOf(userId, null), local: m ? localText(userId, m.firstStart, m.lastEnd) : null };
              }}
              colorOf={projectColor}
              projectNameOf={(id) => projectLabel(id, projectById(id))}
              clockOf={(ms) => clockLabel(calendar.minuteOfDay(ms))}
              fmt={fmt}
            />
            <TimelineLegend kinds={gapKindsOf(timeline.lanes)} />
          </div>
        )}
      </div>

      {blocksState === "ready" && notes.length > 0 && (
        <div>
          <p className="mb-1.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">Worked on</p>
          <ul aria-label={`What was worked on, ${row.label}`} className="divide-y divide-border/60 text-sm">
            {notes.slice(0, 6).map((n) => (
              <li key={n.key} className="flex items-baseline justify-between gap-3 py-1.5">
                <span className={cn("min-w-0 [overflow-wrap:anywhere]", !n.note && "italic text-muted-foreground")}>{n.note ?? NO_NOTE}</span>
                <span className="shrink-0 font-semibold tabular-nums">{fmt(n.minutes)}</span>
              </li>
            ))}
          </ul>
          {notes.length > 6 && <p className="mt-1 text-xs text-muted-foreground">+{notes.length - 6} more in “What was worked on” below</p>}
        </div>
      )}
    </div>
  );
}

/* ----------------------------- Projects ----------------------------- */

function ProjectBreakdown({ rows, format, fmt }: { rows: ProjectShare[]; format: HoursFormat; fmt: (minutes: number) => string }) {
  const recharts = useRecharts();
  const max = rows.reduce((m, p) => Math.max(m, p.minutes), 0);
  const colored = rows.map((p) => ({ ...p, fill: p.color ?? projectColor(p.projectId) }));
  const total = formatTotal(
    rows.map((r) => r.minutes),
    format,
  );
  return (
    <Card className="wr-keep">
      <CardHeader>
        <div>
          <CardTitle>Project breakdown</CardTitle>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {rows.length} {rows.length === 1 ? "project" : "projects"} · {total} in total
          </p>
        </div>
      </CardHeader>
      <CardContent>
        <div className="relative mx-auto h-[168px] w-[168px]" aria-hidden>
          {recharts ? (
            <recharts.PieChart width={168} height={168}>
              <recharts.Pie
                data={colored}
                dataKey="minutes"
                nameKey="name"
                innerRadius={56}
                outerRadius={80}
                paddingAngle={colored.length > 1 ? 2 : 0}
                stroke="var(--card)"
                strokeWidth={2}
                isAnimationActive={false}
              >
                {colored.map((p) => (
                  <recharts.Cell key={p.projectId} fill={p.fill} />
                ))}
              </recharts.Pie>
            </recharts.PieChart>
          ) : (
            <ChartLoading className="rounded-full" />
          )}
          <div className="absolute inset-0 flex flex-col items-center justify-center">
            <span className="font-display text-lg font-bold tabular-nums">{total}</span>
            <span className="text-[11px] text-muted-foreground">total</span>
          </div>
        </div>
        <ul className="mt-4 space-y-3">
          {colored.map((p) => (
            <li key={p.projectId} className="wr-keep">
              <div className="flex items-baseline justify-between gap-3 text-sm">
                <span className="flex min-w-0 items-center gap-2 font-medium">
                  <span aria-hidden className="print-exact h-2.5 w-2.5 shrink-0 rounded-full" style={{ background: p.fill }} />
                  <span className="truncate" title={p.name}>
                    {p.name}
                  </span>
                  {p.archived && (
                    <Badge tone="muted" className="shrink-0 px-1.5 py-0 text-[10px]">
                      Archived
                    </Badge>
                  )}
                </span>
                <span className="shrink-0 tabular-nums">
                  <span className="font-semibold">{fmt(p.minutes)}</span>
                  <span className="text-muted-foreground"> · {p.share}%</span>
                </span>
              </div>
              <div aria-hidden className="print-exact mt-1.5 h-1.5 overflow-hidden rounded-full bg-muted">
                <div className="print-exact h-full rounded-full" style={{ width: `${barPercent(p.minutes, max)}%`, background: p.fill }} />
              </div>
            </li>
          ))}
        </ul>
      </CardContent>
    </Card>
  );
}

/* ----------------------------- What was worked on ----------------------------- */

type NotesView = "notes" | "days" | "members";

/** Notes listed before "Show all" (the printed document lists every one). */
const NOTES_PREVIEW = 8;

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

function NotesCard({
  rows,
  byDay,
  byMember,
  team,
  state,
  sampleNote,
  onRetry,
  nameOf,
  fmt,
}: {
  rows: NoteRow[];
  byDay: NoteSection[];
  byMember: NoteSection[];
  team: boolean;
  state: BlocksState;
  /** Set when the notes come from the newest blocks only. */
  sampleNote: string | null;
  onRetry: () => void;
  nameOf: (userId: string, hint: string | null) => string;
  fmt: (minutes: number) => string;
}) {
  const [view, setView] = useState<NotesView>("notes");
  const [showAll, setShowAll] = useState(false);
  const shown: NotesView = !team && view === "members" ? "notes" : view;
  const noted = rows.filter((r) => r.note).length;
  const max = rows.reduce((m, r) => Math.max(m, r.minutes), 0);
  const options = [
    { key: "notes" as const, label: "By note" },
    { key: "days" as const, label: "By day" },
    ...(team ? [{ key: "members" as const, label: "By member" }] : []),
  ];
  return (
    <Card className="wr-notes">
      <CardHeader>
        <div className="min-w-0">
          <CardTitle className="flex items-center gap-2">
            <ListChecks aria-hidden className="h-4 w-4 text-primary" /> What was worked on
          </CardTitle>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {state === "ready"
              ? noted
                ? `${plural(noted, "task note")} from the desktop app, with the time behind each`
                : "Time by task note, from the desktop app"
              : "Time by task note, from the desktop app"}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {sampleNote && state === "ready" && <SampleBadge detail={sampleNote} />}
          {state === "ready" && rows.length > 0 && (
            <Segmented label="Group the notes" className="no-print" value={shown} onChange={setView} options={options} />
          )}
        </div>
      </CardHeader>
      <CardContent>
        {state === "loading" ? (
          <div aria-hidden className="no-print space-y-3">
            {[0, 1, 2].map((i) => (
              <div key={i} className="h-10 rounded-xl bg-muted/60 motion-safe:animate-pulse" />
            ))}
          </div>
        ) : state === "error" ? (
          <div className="flex flex-wrap items-center justify-between gap-3 text-sm text-muted-foreground">
            <span>The task notes couldn&apos;t be loaded. The hours above are complete.</span>
            <Button variant="outline" size="sm" onClick={() => void onRetry()} className="no-print">
              <RotateCw className="h-3.5 w-3.5" /> Retry
            </Button>
          </div>
        ) : rows.length === 0 || noted === 0 ? (
          <p className="text-sm text-muted-foreground">
            No task notes were added to this period&apos;s tracked time. Notes come from the desktop app&apos;s “What are you working on?” field.
          </p>
        ) : shown === "notes" ? (
          <>
            <ol aria-label="Time per task note" className="space-y-3.5">
              {rows.map((r, i) => (
                <li key={r.key || "none"} className={cn("wr-keep", i >= NOTES_PREVIEW && !showAll && "hidden print:block")}>
                  <div className="flex items-start justify-between gap-3">
                    <p
                      className={cn("min-w-0 text-sm font-medium [overflow-wrap:anywhere] line-clamp-3 print:line-clamp-none", !r.note && "italic text-muted-foreground")}
                      title={r.note ?? undefined}
                    >
                      {r.note ?? NO_NOTE}
                    </p>
                    <p className="shrink-0 text-sm tabular-nums">
                      <span className="font-semibold">{fmt(r.minutes)}</span>
                      <span className="text-muted-foreground"> · {r.share}%</span>
                    </p>
                  </div>
                  <div aria-hidden className="print-exact mt-1.5 h-1.5 overflow-hidden rounded-full bg-muted">
                    <div className={cn("print-exact h-full rounded-full", r.note ? "bg-primary" : "bg-muted-foreground/60")} style={{ width: `${barPercent(r.minutes, max)}%` }} />
                  </div>
                  <p className="mt-1 text-xs text-muted-foreground">
                    {plural(r.days.length, "day")}
                    {team && ` · ${r.members.map((id) => nameOf(id, null)).join(", ")}`}
                  </p>
                </li>
              ))}
            </ol>
            {rows.length > NOTES_PREVIEW && (
              <button
                type="button"
                aria-expanded={showAll}
                onClick={() => setShowAll((v) => !v)}
                className="no-print mt-4 text-xs font-medium text-primary-strong hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                {showAll ? "Show fewer" : `Show all ${rows.length} notes`}
              </button>
            )}
          </>
        ) : (
          <div className="grid grid-cols-1 gap-x-8 gap-y-5 md:grid-cols-2">
            {(shown === "days" ? byDay : byMember).map((section) => (
              <section key={section.id} className="wr-keep min-w-0" aria-label={shown === "days" ? formatDayLabel(section.id) : nameOf(section.id, null)}>
                <h3 className="flex items-center justify-between gap-3 border-b border-border pb-1.5 text-xs font-semibold">
                  <span className="flex min-w-0 items-center gap-2">
                    {shown === "members" && <Avatar name={nameOf(section.id, null)} size="sm" className="print:hidden" />}
                    <span className="truncate">{shown === "days" ? formatDayLabel(section.id) : nameOf(section.id, null)}</span>
                  </span>
                  <span className="shrink-0 tabular-nums">{fmt(section.minutes)}</span>
                </h3>
                <ul className="divide-y divide-border/60 text-sm">
                  {section.notes.map((n) => (
                    <li key={n.key || "none"} className="flex items-baseline justify-between gap-3 py-1.5">
                      <span className={cn("min-w-0 [overflow-wrap:anywhere]", !n.note && "italic text-muted-foreground")}>{n.note ?? NO_NOTE}</span>
                      <span className="shrink-0 tabular-nums">{fmt(n.minutes)}</span>
                    </li>
                  ))}
                </ul>
              </section>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

/* ----------------------------- Team ----------------------------- */

const TEAM_PREVIEW = 6;

/** The team at a glance on screen; the timesheet below has everyone, so this is not printed. */
function TeamCard({ matrix, format, fmt }: { matrix: TimesheetMatrix; format: HoursFormat; fmt: (minutes: number) => string }) {
  const top = matrix.rows[0]?.total ?? 0;
  const shown = matrix.rows.slice(0, TEAM_PREVIEW);
  return (
    <Card className="wr-keep print:hidden">
      <CardHeader>
        <div>
          <CardTitle>Team</CardTitle>
          <p className="mt-0.5 text-xs text-muted-foreground">{matrix.rows.length} members with tracked time</p>
        </div>
      </CardHeader>
      <CardContent>
        <ul aria-label="Hours per member" className="space-y-3">
          {shown.map((r, i) => (
            <li key={r.userId} className="flex items-center gap-3">
              <span aria-hidden className="w-4 shrink-0 text-xs font-semibold tabular-nums text-muted-foreground">
                {i + 1}
              </span>
              <Avatar name={r.name} size="sm" className="print:hidden" />
              <span className="min-w-0 flex-1">
                <span className="flex items-baseline justify-between gap-2">
                  <span className="truncate text-sm font-medium">{r.name}</span>
                  <span className="shrink-0 text-sm font-semibold tabular-nums">{formatTotal(r.cells, format)}</span>
                </span>
                <span className="mt-1 flex items-center gap-2">
                  <MiniBar percent={barPercent(r.total, top)} className="bg-primary" />
                  <span className="shrink-0 text-[11px] text-muted-foreground">
                    {r.workedDays} {r.workedDays === 1 ? "day" : "days"}
                    {r.idleMinutes !== null && ` · idle ${fmt(r.idleMinutes)}`}
                  </span>
                </span>
              </span>
            </li>
          ))}
        </ul>
        {matrix.rows.length > TEAM_PREVIEW && (
          <a href="#team-timesheet" className="no-print mt-4 inline-flex text-xs font-medium text-primary-strong hover:underline">
            See all {matrix.rows.length} in the timesheet
          </a>
        )}
      </CardContent>
    </Card>
  );
}

function TeamTimesheet({
  matrix,
  period,
  zone,
  scale,
  format,
  fmt,
}: {
  matrix: TimesheetMatrix;
  period: string;
  zone: string;
  scale: number;
  format: HoursFormat;
  fmt: (minutes: number) => string;
}) {
  const showIdle = matrix.idleTotal !== null;
  const minWidth = 160 + matrix.days.length * 58 + 76 + (showIdle ? 70 : 0);
  return (
    <Card id="team-timesheet" className="scroll-mt-24">
      <CardHeader>
        <div>
          <CardTitle>Team timesheet</CardTitle>
          <p className="mt-0.5 text-xs text-muted-foreground">Hours per member per day · {matrix.rows.length} members</p>
        </div>
        <div aria-hidden className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <span>Less</span>
          {([1, 2, 3, 4] as const).map((l) => (
            <span key={l} className={cn("print-exact h-3 w-4 rounded-sm", heatClass[l])} />
          ))}
          <span>More</span>
        </div>
      </CardHeader>
      <CardContent className="pt-3">
        <div className="wr-scroll relative overflow-x-auto overscroll-x-contain">
          <table
            className="wr-matrix w-full border-separate border-spacing-0 text-xs"
            style={{ minWidth }}
            data-dense={matrix.days.length > 14 ? "" : undefined}
          >
            <caption className="sr-only">
              Hours per member per day, {period}, with totals per member and per day. Days are calendar days in {zone}.
            </caption>
            <thead>
              <tr>
                <th scope="col" className="sticky left-0 z-10 bg-card py-2 pr-3 text-left text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                  Member
                </th>
                {matrix.days.map((d) => (
                  <th key={d} scope="col" className="px-0.5 py-2 text-center font-medium text-muted-foreground">
                    <abbr title={formatDayLabel(d, { year: true })} className="no-underline">
                      <span className="block text-[11px]">{weekdayShort(d)}</span>
                      <span className="block text-[10px] font-normal">{Number(d.slice(8, 10))}</span>
                    </abbr>
                  </th>
                ))}
                <th scope="col" className="py-2 pl-3 text-right text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                  Total
                </th>
                {showIdle && (
                  <th scope="col" className="py-2 pl-3 text-right text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                    Idle
                  </th>
                )}
              </tr>
            </thead>
            <tbody>
              {matrix.rows.map((r) => (
                <tr key={r.userId}>
                  <th scope="row" className="sticky left-0 z-10 border-t border-border/60 bg-card py-1.5 pr-3 text-left font-medium">
                    <span className="flex min-w-0 items-center gap-2">
                      <span className="hidden sm:inline-flex">
                        <Avatar name={r.name} size="sm" className="print:hidden" />
                      </span>
                      <span className="block max-w-[8.5rem] truncate text-sm sm:max-w-[14rem]" title={r.name}>
                        {r.name}
                      </span>
                    </span>
                  </th>
                  {r.cells.map((c, i) => (
                    <td key={matrix.days[i]} className="border-t border-border/60 p-0.5">
                      <span
                        className={cn(
                          "print-exact flex h-8 min-w-[52px] items-center justify-center whitespace-nowrap rounded-lg px-1 tabular-nums",
                          heatClass[heatLevel(c, scale)],
                        )}
                      >
                        {c > 0 ? fmt(c) : <Dash />}
                      </span>
                    </td>
                  ))}
                  <td className="border-t border-border/60 py-1.5 pl-3 text-right text-sm font-semibold tabular-nums">{formatTotal(r.cells, format)}</td>
                  {showIdle && (
                    <td className="whitespace-nowrap border-t border-border/60 py-1.5 pl-3 text-right text-sm tabular-nums text-muted-foreground">
                      {r.idleMinutes === null ? <Dash label="Not measured" /> : fmt(r.idleMinutes)}
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
            <tfoot>
              <tr className="font-bold">
                <th scope="row" className="sticky left-0 z-10 border-t-2 border-foreground/70 bg-card py-2.5 pr-3 text-left text-sm">
                  Total
                </th>
                {matrix.dayTotals.map((t, i) => (
                  <td key={matrix.days[i]} className="whitespace-nowrap border-t-2 border-foreground/70 px-0.5 py-2.5 text-center tabular-nums">
                    {t > 0
                      ? formatTotal(
                          matrix.rows.map((r) => r.cells[i]),
                          format,
                        )
                      : <Dash />}
                  </td>
                ))}
                <td className="border-t-2 border-foreground/70 py-2.5 pl-3 text-right text-sm tabular-nums">
                  {formatTotal(
                    matrix.rows.flatMap((r) => r.cells),
                    format,
                  )}
                </td>
                {showIdle && (
                  <td className="whitespace-nowrap border-t-2 border-foreground/70 py-2.5 pl-3 text-right text-sm tabular-nums">
                    {formatTotal(
                      matrix.rows.map((r) => r.idleMinutes ?? 0),
                      format,
                    )}
                  </td>
                )}
              </tr>
            </tfoot>
          </table>
        </div>
      </CardContent>
    </Card>
  );
}

/* ----------------------------- Loading & footnote ----------------------------- */

function LoadingSkeleton() {
  return (
    <div aria-hidden className="no-print grid grid-cols-1 gap-5 xl:grid-cols-3">
      <div className="h-[340px] rounded-2xl border border-border bg-muted/50 motion-safe:animate-pulse xl:col-span-2" />
      <div className="h-[340px] rounded-2xl border border-border bg-muted/50 motion-safe:animate-pulse" />
      <div className="h-[420px] rounded-2xl border border-border bg-muted/50 motion-safe:animate-pulse xl:col-span-2" />
      <div className="h-[420px] rounded-2xl border border-border bg-muted/50 motion-safe:animate-pulse" />
    </div>
  );
}

function Footnote({ zone }: { zone: string }) {
  return (
    <div className="wr-keep flex flex-col gap-3 border-t border-border pt-4 text-xs leading-relaxed text-muted-foreground sm:flex-row sm:items-start sm:justify-between">
      <p className="flex max-w-3xl gap-2">
        <Info aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0" />
        <span>
          Hours are the sum of tracked activity blocks recorded by the desktop app; sessions are those blocks, each counted on
          the day it started. Days are calendar days in {zone}; start and finish times are each member&apos;s own local time
          when their app reports its time zone. Activity % is the share of tracked minutes with keyboard or mouse input
          (“—” where input isn&apos;t measured); idle time is tracked time without any. Task notes come from the desktop
          app&apos;s “What are you working on?” field.
        </span>
      </p>
      <p className="shrink-0 font-medium text-foreground">Prepared with Dosi Tracker</p>
    </div>
  );
}
