"use client";

import { useCallback, useMemo } from "react";
import { Clock, Gauge, Hourglass, Users, Zap } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { DataTable, type Column } from "@/components/ui/data-table";
import { getApi } from "@/hooks/useApi";
import { reportingRoute } from "@/lib/api-contract";
import { downloadCSV } from "@/lib/export";
import {
  ALL,
  activityBreakdown,
  dayBars,
  decimalHoursTotal,
  displayedTotal,
  memberName,
  readDaily,
  readSummary,
  safePercent,
  summarizeBlocks,
  topProjectByMember,
  type DailyFigures,
  type ReportSummary,
} from "@/lib/report-math";
import { projectById, userById } from "@/lib/tenant-data";
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
  demoActivities,
  useHoursFormat,
  usePeriodReport,
  useReportPeriod,
  useReportScope,
} from "./report-shell";

/* ============================================================================
 * TIME & ACTIVITY — hours per day and per member, with how much of that time
 * had keyboard or mouse input (activity %, active time) and how much had none
 * (idle time). Live: /reporting/summary and /reporting/daily-series for the
 * period, in the reporting zone, filtered server-side; demo: the same shapes
 * derived from the seeded activity.
 * ========================================================================== */

interface Figures {
  summary: ReportSummary;
  daily: DailyFigures[];
}

interface Row {
  userId: string;
  name: string;
  designation: string | null;
  tracked: number;
  active: number | null;
  idle: number | null;
  activity: number | null;
  sessions: number;
  topProject: string | null;
}

export function TimeActivityReport() {
  const scope = useReportScope();
  const { isLive, projectId, memberId } = scope;
  const range = useReportPeriod("7d", isLive);
  const { period, timeZone } = range;
  const { format, setFormat, fmt } = useHoursFormat();

  const load = useCallback(async (): Promise<Figures> => {
    const query = { from: period.from, to: period.to, projectId, userId: memberId, timeZone };
    const [summary, daily] = await Promise.all([getApi(reportingRoute("summary", query)), getApi(reportingRoute("daily-series", query))]);
    return { summary: readSummary(summary), daily: readDaily(daily) };
  }, [period, projectId, memberId, timeZone]);
  const live = usePeriodReport(isLive, range.ready, load);

  const demo = useMemo<Figures | null>(() => {
    if (isLive) return null;
    const { summary, daily } = summarizeBlocks(demoActivities(period, projectId, memberId), timeZone);
    return { summary, daily };
  }, [isLive, period, projectId, memberId, timeZone]);

  // A live session shows only real figures (or nothing) — never the demo team.
  const figures = isLive ? live.data : demo;
  const pending = live.pending;

  const rows = useMemo<Row[]>(() => {
    if (!figures) return [];
    const top = figures.summary.perUserProject ? topProjectByMember(figures.summary.perUserProject) : null;
    return figures.summary.perUser
      .filter((u) => memberId === ALL || u.userId === memberId)
      .map((u) => {
        const member = userById(u.userId);
        const split = activityBreakdown(u.trackedMinutes, u.averageProductivity, u.idleMinutes);
        const topId = top?.get(u.userId)?.projectId ?? null;
        return {
          userId: u.userId,
          name: memberName(u.userId, member ?? (u.name ? { name: u.name } : null)),
          designation: member?.designation ?? null,
          tracked: split.tracked,
          active: split.active,
          idle: split.idle,
          activity: split.activity,
          sessions: u.activityCount,
          topProject: topId ? projectLabel(topId, projectById(topId)) : null,
        };
      });
  }, [figures, memberId]);

  const bars = useMemo(() => dayBars(period.days, figures?.daily ?? []), [period.days, figures]);
  const idleKnown = rows.some((r) => r.idle !== null);
  const activeKnown = rows.some((r) => r.active !== null);
  const topKnown = !!figures?.summary.perUserProject && rows.some((r) => r.topProject !== null);
  const hasTime = (figures?.summary.trackedMinutes ?? 0) > 0;

  const tracked = rows.map((r) => r.tracked);
  const totalTracked = displayedTotal(tracked, format);
  const totalIdle = displayedTotal(rows.map((r) => r.idle ?? 0), format);
  const totalActive = displayedTotal(rows.map((r) => r.active ?? 0), format);
  const sessions = rows.reduce((s, r) => s + r.sessions, 0);
  const average = figures?.summary.averageProductivity ?? null;
  const averagePercent = average === null ? null : Math.round(average);
  const dash = "—";

  const columns: Column<Row>[] = [
    { key: "name", header: "Member", sortValue: (r) => r.name, render: (r) => <MemberCell name={r.name} sub={r.designation} status={userById(r.userId)?.status} /> },
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
    { key: "sessions", header: "Sessions", align: "right", sortValue: (r) => r.sessions, render: (r) => <span className="tabular-nums">{r.sessions}</span> },
    ...(topKnown
      ? [
          {
            key: "topProject",
            header: "Top project",
            sortValue: (r: Row) => r.topProject ?? "",
            render: (r: Row) => (r.topProject ? <span className="text-muted-foreground">{r.topProject}</span> : <Dash label="No project" />),
          },
        ]
      : []),
  ];

  const footer = (
    <>
      <td className="sticky left-0 z-10 bg-card px-3 py-2.5">Total</td>
      <td className="px-3 py-2.5 text-right tabular-nums">{fmt(totalTracked)}</td>
      <td className="px-3 py-2.5 text-right tabular-nums">{averagePercent === null ? dash : `${averagePercent}%`}</td>
      <td className="px-3 py-2.5 text-right tabular-nums">{activeKnown ? fmt(totalActive) : dash}</td>
      {idleKnown && <td className="px-3 py-2.5 text-right tabular-nums">{fmt(totalIdle)}</td>}
      <td className="px-3 py-2.5 text-right tabular-nums">{sessions}</td>
      {topKnown && <td className="px-3 py-2.5" />}
    </>
  );

  function exportCsv() {
    const headers = [
      "Member",
      "Hours (h:mm)",
      "Hours (decimal)",
      "Activity %",
      "Active hours (decimal)",
      ...(idleKnown ? ["Idle hours (decimal)"] : []),
      "Sessions",
      ...(topKnown ? ["Top project"] : []),
    ];
    const body: (string | number)[][] = rows.map((r) => [
      r.name,
      formatHM(r.tracked),
      decimalHours(r.tracked),
      r.activity ?? "",
      r.active === null ? "" : decimalHours(r.active),
      ...(idleKnown ? [r.idle === null ? "" : decimalHours(r.idle)] : []),
      r.sessions,
      ...(topKnown ? [r.topProject ?? ""] : []),
    ]);
    body.push([
      "Total",
      formatHM(displayedTotal(tracked, "clock")),
      decimalHoursTotal(tracked),
      averagePercent ?? "",
      activeKnown ? decimalHoursTotal(rows.map((r) => r.active ?? 0)) : "",
      ...(idleKnown ? [decimalHoursTotal(rows.map((r) => r.idle ?? 0))] : []),
      sessions,
      ...(topKnown ? [""] : []),
    ]);
    downloadCSV(`time-activity-${period.fromKey}-to-${period.toKey}`, headers, body);
  }

  return (
    <ReportShell
      title="Time & Activity"
      description="Hours, activity (keyboard and mouse) and idle time per member, day by day."
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
        <Kpi label="Total tracked" icon={Clock} accent="blue" value={pending ? dash : fmt(totalTracked)} sub={pending ? undefined : `${sessions} ${sessions === 1 ? "session" : "sessions"}`} />
        <Kpi
          label="Average activity"
          icon={Gauge}
          accent="green"
          value={pending || averagePercent === null ? dash : `${averagePercent}%`}
          sub={pending ? undefined : averagePercent === null ? "keyboard & mouse not measured" : "keyboard & mouse input"}
        >
          {!pending && averagePercent !== null && <MiniBar percent={averagePercent} className="bg-success" />}
        </Kpi>
        {idleKnown || pending ? (
          <Kpi
            label="Idle time"
            icon={Hourglass}
            accent="amber"
            value={pending ? dash : fmt(totalIdle)}
            sub={pending ? undefined : `${safePercent(totalIdle, totalTracked)}% of tracked time`}
          />
        ) : (
          <Kpi
            label="Active time"
            icon={Zap}
            accent="amber"
            value={activeKnown ? fmt(totalActive) : dash}
            sub={activeKnown ? "tracked time × activity" : "keyboard & mouse not measured"}
          />
        )}
        <Kpi label="Members" icon={Users} accent="violet" value={pending ? dash : String(rows.length)} sub={pending ? undefined : "with tracked time"} />
      </KpiGrid>

      {pending ? (
        live.loading ? <ReportSkeleton /> : null
      ) : !hasTime ? (
        <NoTimeState period={period} onWiden={period.custom || range.toolbar.preset !== "30d" ? () => range.choosePreset("30d") : undefined} />
      ) : (
        <>
          <Card>
            <SectionHeading title="Hours per day" sub={`${period.label} · active, other tracked and idle time`}>
              <ActivityLegend idle={idleKnown} other={bars.some((b) => b.other > 0)} />
            </SectionHeading>
            <CardContent>
              <DailyHoursChart bars={bars} fmt={fmt} label={`Hours tracked per day, ${period.label}. The table below lists each member.`} />
            </CardContent>
          </Card>

          <Card>
            <SectionHeading title="By member" sub={`${rows.length} ${rows.length === 1 ? "member" : "members"} with tracked time`} />
            <CardContent>
              <DataTable columns={columns} rows={rows} initialSort={{ key: "tracked", dir: "desc" }} footer={footer} emptyText="No tracked time for these filters." />
            </CardContent>
          </Card>
        </>
      )}

      <ReportFootnote>
        Hours are the tracked blocks recorded by the desktop app, on calendar days of the reporting time zone shown above. Activity % is
        the share of tracked minutes with keyboard or mouse input (blocks recorded without input capture aren&apos;t counted); active time
        is tracked time × activity %, idle time is the minutes without input, and other tracked time is the rest.
        {!idleKnown && " This server doesn't report idle time yet."}
      </ReportFootnote>
    </ReportShell>
  );
}
