"use client";

import { useCallback, useMemo } from "react";
import { CalendarDays, Clock, Gauge, Hourglass, Users } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { DataTable, type Column } from "@/components/ui/data-table";
import { getApi } from "@/hooks/useApi";
import { reportingRoute } from "@/lib/api-contract";
import { downloadCSV } from "@/lib/export";
import {
  ALL,
  activeDaysByUser,
  activityBreakdown,
  dayBars,
  decimalHoursTotal,
  displayedTotal,
  elapsedDays,
  memberName,
  readDaily,
  readMemberDays,
  readSummary,
  safePercent,
  summarizeBlocks,
  topProjectByMember,
  type DailyFigures,
  type MemberDayFigures,
  type ReportSummary,
} from "@/lib/report-math";
import { projectById, userById } from "@/lib/tenant-data";
import { formatDayKey } from "@/lib/time-zone";
import { decimalHours, formatHM, projectLabel } from "@/lib/work-report";
import {
  ActivityBadge,
  ActivityLegend,
  DailyHoursChart,
  Dash,
  ExportMenu,
  HoursFormatToggle,
  Kpi,
  KpiGrid,
  LiveReportNotice,
  MemberCell,
  MiniBar,
  NoTimeState,
  ReportFootnote,
  ReportShell,
  ReportSkeleton,
  ReportToolbar,
  SectionHeading,
  ShareDonut,
  demoActivities,
  useHoursFormat,
  usePeriodReport,
  useReportPeriod,
  useReportScope,
} from "./report-shell";

/* ============================================================================
 * WEEKLY SUMMARY — the week at a glance: the team's hours per day, how they
 * split into active, other and idle time, the busiest day, and a digest per
 * member (hours, activity, idle time, days worked, main project). It opens on
 * this Monday–Sunday week and steps back week by week; any other period works
 * too. Live: /reporting/summary, /daily-series and /user-daily-series in the
 * reporting zone (days worked is a detail: without the per-member series that
 * column reads "—"); demo: the same shapes from the seeded activity.
 * ========================================================================== */

interface Figures {
  summary: ReportSummary;
  daily: DailyFigures[];
  /** null when the per-member day series couldn't be read. */
  memberDays: MemberDayFigures[] | null;
}

interface Row {
  userId: string;
  name: string;
  tracked: number;
  active: number | null;
  idle: number | null;
  activity: number | null;
  daysWorked: number | null;
  topProject: string | null;
}

const SPLIT_COLORS = {
  active: "var(--primary)",
  other: "color-mix(in srgb, var(--primary) 30%, transparent)",
  idle: "var(--warning)",
};

export function WeeklyReport() {
  const scope = useReportScope();
  const { isLive, projectId, memberId } = scope;
  const range = useReportPeriod("week", isLive);
  const { period, timeZone } = range;
  const { format, setFormat, fmt } = useHoursFormat();

  const load = useCallback(async (): Promise<Figures> => {
    const query = { from: period.from, to: period.to, projectId, userId: memberId, timeZone };
    const [summary, daily, memberDays] = await Promise.all([
      getApi(reportingRoute("summary", query)),
      getApi(reportingRoute("daily-series", query)),
      getApi(reportingRoute("user-daily-series", query)).catch(() => null),
    ]);
    return { summary: readSummary(summary), daily: readDaily(daily), memberDays: Array.isArray(memberDays) ? readMemberDays(memberDays) : null };
  }, [period, projectId, memberId, timeZone]);
  const live = usePeriodReport(isLive, range.ready, load);

  const demo = useMemo<Figures | null>(() => (isLive ? null : summarizeBlocks(demoActivities(period, projectId, memberId), timeZone)), [
    isLive,
    period,
    projectId,
    memberId,
    timeZone,
  ]);

  // A live session shows only real figures (or nothing) — never the demo team.
  const figures = isLive ? live.data : demo;
  const pending = live.pending;
  const dayCount = elapsedDays(period).length;

  const rows = useMemo<Row[]>(() => {
    if (!figures) return [];
    const days = new Set(period.days);
    const worked = figures.memberDays ? activeDaysByUser(figures.memberDays.filter((d) => days.has(d.date))) : null;
    const top = figures.summary.perUserProject ? topProjectByMember(figures.summary.perUserProject) : null;
    return figures.summary.perUser
      .filter((u) => memberId === ALL || u.userId === memberId)
      .map((u) => {
        const split = activityBreakdown(u.trackedMinutes, u.averageProductivity, u.idleMinutes);
        const topId = top?.get(u.userId)?.projectId ?? null;
        return {
          userId: u.userId,
          name: memberName(u.userId, userById(u.userId) ?? (u.name ? { name: u.name } : null)),
          tracked: split.tracked,
          active: split.active,
          idle: split.idle,
          activity: split.activity,
          daysWorked: worked ? (worked.get(u.userId) ?? 0) : null,
          topProject: topId ? projectLabel(topId, projectById(topId)) : null,
        };
      });
  }, [figures, memberId, period.days]);

  const bars = useMemo(() => dayBars(period.days, figures?.daily ?? []), [period.days, figures]);
  const idleKnown = rows.some((r) => r.idle !== null);
  const topKnown = rows.some((r) => r.topProject !== null);
  const daysKnown = rows.some((r) => r.daysWorked !== null);
  const hasTime = (figures?.summary.trackedMinutes ?? 0) > 0;

  const tracked = rows.map((r) => r.tracked);
  const totalTracked = displayedTotal(tracked, format);
  const totalActive = displayedTotal(rows.map((r) => r.active ?? 0), format);
  const totalIdle = displayedTotal(rows.map((r) => r.idle ?? 0), format);
  const totalOther = Math.max(0, displayedTotal(rows.map((r) => r.tracked - (r.active ?? 0) - (r.idle ?? 0)), format));
  const average = figures?.summary.averageProductivity ?? null;
  const averagePercent = average === null ? null : Math.round(average);
  // The busiest day of the period (the earliest on a tie); a day without time is never "busiest".
  const busiest = bars.reduce<(typeof bars)[number] | null>((best, d) => (d.minutes > 0 && (!best || d.minutes > best.minutes) ? d : best), null);
  const dash = "—";

  const columns: Column<Row>[] = [
    { key: "name", header: "Member", sortValue: (r) => r.name, render: (r) => <MemberCell name={r.name} status={userById(r.userId)?.status} /> },
    { key: "tracked", header: "Tracked", align: "right", sortValue: (r) => r.tracked, render: (r) => <span className="font-semibold tabular-nums">{fmt(r.tracked)}</span> },
    { key: "activity", header: "Activity", align: "right", sortValue: (r) => r.activity ?? -1, render: (r) => <ActivityBadge percent={r.activity} /> },
    {
      key: "active",
      header: "Active",
      align: "right",
      sortValue: (r) => r.active ?? -1,
      render: (r) => (r.active === null ? <Dash label="Not measured" /> : <span className="tabular-nums">{fmt(r.active)}</span>),
    },
    ...(idleKnown
      ? [
          {
            key: "idle",
            header: "Idle",
            align: "right" as const,
            sortValue: (r: Row) => r.idle ?? -1,
            render: (r: Row) => (r.idle === null ? <Dash label="Not reported" /> : <span className="tabular-nums">{fmt(r.idle)}</span>),
          },
        ]
      : []),
    ...(daysKnown
      ? [
          {
            key: "daysWorked",
            header: "Days worked",
            align: "right" as const,
            sortValue: (r: Row) => r.daysWorked ?? -1,
            render: (r: Row) =>
              r.daysWorked === null ? (
                <Dash />
              ) : (
                <span className="tabular-nums">
                  {r.daysWorked}
                  <span className="text-muted-foreground"> of {dayCount}</span>
                </span>
              ),
          },
        ]
      : []),
    ...(topKnown
      ? [
          {
            key: "topProject",
            header: "Main project",
            sortValue: (r: Row) => r.topProject ?? "",
            render: (r: Row) => (r.topProject ? <span className="text-muted-foreground">{r.topProject}</span> : <Dash label="No project" />),
          },
        ]
      : []),
  ];

  function exportCsv() {
    const headers = [
      "Member",
      "Hours (h:mm)",
      "Hours (decimal)",
      "Activity %",
      "Active hours (decimal)",
      ...(idleKnown ? ["Idle hours (decimal)"] : []),
      ...(daysKnown ? [`Days worked (of ${dayCount})`] : []),
      ...(topKnown ? ["Main project"] : []),
    ];
    const body: (string | number)[][] = rows.map((r) => [
      r.name,
      formatHM(r.tracked),
      decimalHours(r.tracked),
      r.activity ?? "",
      r.active === null ? "" : decimalHours(r.active),
      ...(idleKnown ? [r.idle === null ? "" : decimalHours(r.idle)] : []),
      ...(daysKnown ? [r.daysWorked ?? ""] : []),
      ...(topKnown ? [r.topProject ?? ""] : []),
    ]);
    body.push([
      "Total",
      formatHM(displayedTotal(tracked, "clock")),
      decimalHoursTotal(tracked),
      averagePercent ?? "",
      rows.some((r) => r.active !== null) ? decimalHoursTotal(rows.map((r) => r.active ?? 0)) : "",
      ...(idleKnown ? [decimalHoursTotal(rows.map((r) => r.idle ?? 0))] : []),
      ...(daysKnown ? [""] : []),
      ...(topKnown ? [""] : []),
    ]);
    downloadCSV(`weekly-summary-${period.fromKey}-to-${period.toKey}`, headers, body);
  }

  return (
    <ReportShell
      title="Weekly Summary"
      description="The week at a glance per member: hours, activity, idle time, days worked and the busiest day."
      actions={<ExportMenu onExportCSV={exportCsv} disabled={pending || rows.length === 0} />}
    >
      <ReportToolbar
        period={range}
        projectId={projectId}
        onProject={scope.setProjectId}
        memberId={memberId}
        onMember={scope.setMemberId}
        liveWorkerId={scope.liveWorkerId}
        extra={<HoursFormatToggle value={format} onChange={setFormat} />}
      />

      {isLive && (
        <div className="no-print">
          <LiveReportNotice loading={live.loading} error={live.error} onRetry={live.retry} />
        </div>
      )}

      <KpiGrid>
        <Kpi
          label="Team hours"
          icon={Clock}
          accent="blue"
          value={pending ? dash : fmt(totalTracked)}
          sub={pending ? undefined : `${rows.length} ${rows.length === 1 ? "member" : "members"} with tracked time`}
        />
        <Kpi
          label="Average activity"
          icon={Gauge}
          accent="green"
          value={pending || averagePercent === null ? dash : `${averagePercent}%`}
          sub={pending ? undefined : averagePercent === null ? "keyboard & mouse not measured" : "keyboard & mouse input"}
        >
          {!pending && averagePercent !== null && <MiniBar percent={averagePercent} className="bg-success" />}
        </Kpi>
        <Kpi
          label="Busiest day"
          icon={CalendarDays}
          accent="violet"
          value={pending || !busiest ? dash : formatDayKey(busiest.date, { weekday: true })}
          sub={pending || !busiest ? undefined : `${fmt(busiest.minutes)} tracked`}
        />
        <Kpi
          label={idleKnown || pending ? "Idle time" : "Members"}
          icon={idleKnown || pending ? Hourglass : Users}
          accent="amber"
          value={pending ? dash : idleKnown ? fmt(totalIdle) : String(rows.length)}
          sub={pending ? undefined : idleKnown ? `${safePercent(totalIdle, totalTracked)}% of tracked time` : "with tracked time"}
        />
      </KpiGrid>

      {pending ? (
        live.loading ? <ReportSkeleton /> : null
      ) : !hasTime ? (
        <NoTimeState period={period} onWiden={period.custom || range.toolbar.preset !== "30d" ? () => range.choosePreset("30d") : undefined} />
      ) : (
        <>
          <div className="grid grid-cols-1 gap-5 xl:grid-cols-3">
            <Card className="xl:col-span-2">
              <SectionHeading title="Team hours per day" sub={period.label}>
                <ActivityLegend idle={idleKnown} other={bars.some((b) => b.other > 0)} />
              </SectionHeading>
              <CardContent>
                <DailyHoursChart bars={bars} fmt={fmt} label={`Team hours per day, ${period.label}. The member digest below lists each member.`} />
              </CardContent>
            </Card>
            <Card>
              <SectionHeading title="How the time was spent" sub="Keyboard and mouse input across the team" />
              <CardContent>
                <ShareDonut
                  label="Time by kind"
                  fmt={fmt}
                  slices={[
                    { key: "active", label: "Active", minutes: totalActive, color: SPLIT_COLORS.active },
                    ...(totalOther > 0 ? [{ key: "other", label: "Other tracked", minutes: totalOther, color: SPLIT_COLORS.other }] : []),
                    ...(idleKnown ? [{ key: "idle", label: "Idle", minutes: totalIdle, color: SPLIT_COLORS.idle }] : []),
                  ]}
                />
              </CardContent>
            </Card>
          </div>

          <Card>
            <SectionHeading title="Member digest" sub={`${rows.length} ${rows.length === 1 ? "member" : "members"} · ${period.label}`} />
            <CardContent>
              <DataTable columns={columns} rows={rows} initialSort={{ key: "tracked", dir: "desc" }} emptyText="No tracked time in this period." />
            </CardContent>
          </Card>
        </>
      )}

      <ReportFootnote>
        A week runs Monday to Sunday in the reporting time zone shown above; the current week runs up to today. Activity % is the share of
        tracked minutes with keyboard or mouse input; active time is tracked time × activity %, idle time is the minutes without input, and
        days worked counts the days with any tracked time.
        {!idleKnown && " This server doesn't report idle time yet."}
      </ReportFootnote>
    </ReportShell>
  );
}
