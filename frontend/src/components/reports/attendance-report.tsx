"use client";

import { useCallback, useMemo } from "react";
import { CalendarCheck, CalendarX, Sun, Sunrise, Sheet } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { DataTable, type Column } from "@/components/ui/data-table";
import { getApi } from "@/hooks/useApi";
import { reportingRoute } from "@/lib/api-contract";
import { downloadCSV } from "@/lib/export";
import {
  ALL,
  attendanceFromBlocks,
  attendanceRows,
  averageClock,
  decimalHoursTotal,
  displayedTotal,
  elapsedDays,
  formatClock,
  memberName,
  readAttendance,
  safePercent,
  withRoster,
  type AttendanceLine,
  type AttendanceRow,
} from "@/lib/report-math";
import { trackedMembers } from "@/lib/roles";
import { projectById, userById, users } from "@/lib/tenant-data";
import { formatDayKey } from "@/lib/time-zone";
import { decimalHours, formatHM, isWeekend, weekdayShort } from "@/lib/work-report";
import { cn } from "@/lib/utils";
import {
  Dash,
  ExportMenu,
  HoursFormatToggle,
  Kpi,
  KpiGrid,
  LiveReportNotice,
  MemberCell,
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
 * ATTENDANCE — presence derived honestly from tracked activity: a member is
 * present on a working day (Mon–Fri of the reporting zone) on which they
 * tracked anything, and absent on one they didn't; weekend work is counted
 * apart, never as an absence. Every member of the scoped projects is listed,
 * including those who tracked nothing at all. Days still ahead are not shown;
 * present + absent always equals the working days in the grid.
 *
 * The tracker records no shifts or clock-ins, so "start" and "finish" are the
 * first and last tracked minute of each present day, averaged. Live:
 * GET /api/app/reporting/attendance in the reporting zone (an older server
 * leaves out members without time; they're added from the roster).
 * ========================================================================== */

interface Loaded {
  lines: AttendanceLine[];
  /** The source lists every member and reports start/finish (contract v2). */
  detailed: boolean;
}

interface Row extends AttendanceRow {
  label: string;
}

export function AttendanceReport() {
  const scope = useReportScope();
  const { isLive, projectId, memberId, access, user } = scope;
  const range = useReportPeriod("week", isLive);
  const { period, timeZone } = range;
  const { format, setFormat, fmt } = useHoursFormat();

  const load = useCallback(async (): Promise<Loaded> => {
    const report = await getApi(reportingRoute("attendance", { from: period.from, to: period.to, projectId, userId: memberId, timeZone }));
    return readAttendance(report);
  }, [period, projectId, memberId, timeZone]);
  const live = usePeriodReport(isLive, range.ready, load);

  const demo = useMemo<Loaded | null>(
    () => (isLive ? null : { lines: attendanceFromBlocks(demoActivities(period, projectId, memberId), timeZone), detailed: false }),
    [isLive, period, projectId, memberId, timeZone],
  );

  // Who should be on the sheet: the tracked members of the scoped projects (the server's roster rule),
  // or only the viewer when the server reports only about them.
  const roster = useMemo(() => {
    if (isLive && !access.viewAll) return [{ userId: user.id, name: user.name }];
    const onProject = projectId === ALL ? null : new Set(projectById(projectId)?.memberIds ?? []);
    return trackedMembers(users)
      .filter((u) => (memberId === ALL || u.id === memberId) && (!onProject || onProject.has(u.id)))
      .map((u) => ({ userId: u.id, name: u.name }));
  }, [isLive, access.viewAll, user.id, user.name, projectId, memberId]);

  // A live session shows only real presence (or nothing) — never the demo team.
  const loaded = isLive ? live.data : demo;
  const pending = live.pending;
  const days = useMemo(() => elapsedDays(period), [period]);
  const workdays = days.filter((d) => !isWeekend(d));

  const rows = useMemo<Row[]>(() => {
    if (!loaded) return [];
    const lines = loaded.detailed ? loaded.lines : withRoster(loaded.lines, roster);
    return attendanceRows(lines, days)
      .filter((r) => memberId === ALL || r.userId === memberId)
      .map((r) => ({ ...r, label: memberName(r.userId, userById(r.userId) ?? (r.name ? { name: r.name } : null)) }))
      .sort((a, b) => a.label.localeCompare(b.label));
  }, [loaded, roster, days, memberId]);

  const present = rows.reduce((s, r) => s + r.present, 0);
  const absent = rows.reduce((s, r) => s + r.absent, 0);
  const weekendDays = rows.reduce((s, r) => s + r.weekendDays, 0);
  const clock = averageClock(rows);
  const clockKnown = rows.some((r) => r.averageStart !== null || r.averageEnd !== null);
  const idleKnown = rows.some((r) => r.idleMinutes !== null);
  const worked = rows.map((r) => r.workedMinutes);
  const dash = "—";

  const columns: Column<Row>[] = [
    { key: "name", header: "Member", sortValue: (r) => r.label, render: (r) => <MemberCell name={r.label} status={userById(r.userId)?.status} /> },
    { key: "present", header: "Present", align: "right", sortValue: (r) => r.present, render: (r) => <span className="tabular-nums text-success-on-tint">{r.present}</span> },
    { key: "absent", header: "Absent", align: "right", sortValue: (r) => r.absent, render: (r) => <span className="tabular-nums text-danger-on-tint">{r.absent}</span> },
    { key: "weekend", header: "Weekend days", align: "right", sortValue: (r) => r.weekendDays, render: (r) => <span className="tabular-nums">{r.weekendDays}</span> },
    ...(clockKnown
      ? [
          {
            key: "start",
            header: "Avg start",
            align: "right" as const,
            sortValue: (r: Row) => r.averageStart ?? Number.MAX_SAFE_INTEGER,
            render: (r: Row) => (r.averageStart === null ? <Dash label="Not present" /> : <span className="tabular-nums">{formatClock(r.averageStart)}</span>),
          },
          {
            key: "end",
            header: "Avg finish",
            align: "right" as const,
            sortValue: (r: Row) => r.averageEnd ?? -1,
            render: (r: Row) => (r.averageEnd === null ? <Dash label="Not present" /> : <span className="tabular-nums">{formatClock(r.averageEnd)}</span>),
          },
        ]
      : []),
    { key: "worked", header: "Worked", align: "right", sortValue: (r) => r.workedMinutes, render: (r) => <span className="font-semibold tabular-nums">{fmt(r.workedMinutes)}</span> },
    ...(idleKnown
      ? [
          {
            key: "idle",
            header: "Idle",
            align: "right" as const,
            sortValue: (r: Row) => r.idleMinutes ?? -1,
            render: (r: Row) => (r.idleMinutes === null ? <Dash label="Not reported" /> : <span className="tabular-nums">{fmt(r.idleMinutes)}</span>),
          },
        ]
      : []),
  ];

  const footer = (
    <>
      <td className="sticky left-0 z-10 bg-card px-3 py-2.5">Total</td>
      <td className="px-3 py-2.5 text-right tabular-nums">{present}</td>
      <td className="px-3 py-2.5 text-right tabular-nums">{absent}</td>
      <td className="px-3 py-2.5 text-right tabular-nums">{weekendDays}</td>
      {clockKnown && <td className="px-3 py-2.5 text-right tabular-nums">{formatClock(clock.start)}</td>}
      {clockKnown && <td className="px-3 py-2.5 text-right tabular-nums">{formatClock(clock.end)}</td>}
      <td className="px-3 py-2.5 text-right tabular-nums">{fmt(displayedTotal(worked, format))}</td>
      {idleKnown && <td className="px-3 py-2.5 text-right tabular-nums">{fmt(displayedTotal(rows.map((r) => r.idleMinutes ?? 0), format))}</td>}
    </>
  );

  const fileBase = `attendance-${period.fromKey}-to-${period.toKey}`;
  function exportSummary() {
    const headers = [
      "Member",
      "Present days",
      "Absent days",
      "Weekend days worked",
      ...(clockKnown ? ["Average start", "Average finish"] : []),
      "Worked hours (decimal)",
      "Worked (h:mm)",
      ...(idleKnown ? ["Idle hours (decimal)"] : []),
    ];
    const body: (string | number)[][] = rows.map((r) => [
      r.label,
      r.present,
      r.absent,
      r.weekendDays,
      ...(clockKnown ? [r.averageStart === null ? "" : formatClock(r.averageStart), r.averageEnd === null ? "" : formatClock(r.averageEnd)] : []),
      decimalHours(r.workedMinutes),
      formatHM(r.workedMinutes),
      ...(idleKnown ? [r.idleMinutes === null ? "" : decimalHours(r.idleMinutes)] : []),
    ]);
    body.push([
      "Total",
      present,
      absent,
      weekendDays,
      ...(clockKnown ? [clock.start === null ? "" : formatClock(clock.start), clock.end === null ? "" : formatClock(clock.end)] : []),
      decimalHoursTotal(worked),
      formatHM(displayedTotal(worked, "clock")),
      ...(idleKnown ? [decimalHoursTotal(rows.map((r) => r.idleMinutes ?? 0))] : []),
    ]);
    downloadCSV(`${fileBase}-summary`, headers, body);
  }
  function exportDays() {
    const headers = ["Member", ...days.map((d) => `${d} (${weekdayShort(d)})`)];
    const body = rows.map((r) => [r.label, ...days.map((d) => (r.presentDays.has(d) ? "Present" : isWeekend(d) ? "" : "Absent"))]);
    downloadCSV(`${fileBase}-days`, headers, body);
  }

  return (
    <ReportShell
      aiExplain={{ slug: "attendance", query: { from: period.from, to: period.to, projectId, userId: memberId, timeZone } }}
      title="Attendance"
      description="Days present and absent, typical start and finish times, and weekend work per member."
      actions={
        <ExportMenu
          onExportCSV={exportSummary}
          csvLabel="Summary (CSV)"
          extraItems={[{ label: "Day by day (CSV)", onSelect: exportDays, icon: Sheet }]}
          disabled={pending || rows.length === 0}
        />
      }
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
          label="Attendance rate"
          icon={CalendarCheck}
          accent="green"
          value={pending || present + absent === 0 ? dash : `${safePercent(present, present + absent)}%`}
          sub={pending ? undefined : `${present} of ${present + absent} working days present`}
        />
        <Kpi label="Absent days" icon={CalendarX} accent="pink" value={pending ? dash : String(absent)} sub={pending ? undefined : `across ${rows.length} ${rows.length === 1 ? "member" : "members"}`} />
        <Kpi
          label="Typical day"
          icon={Sunrise}
          accent="blue"
          value={pending || clock.start === null || clock.end === null ? dash : `${formatClock(clock.start)} – ${formatClock(clock.end)}`}
          sub={pending ? undefined : clockKnown ? "average start and finish" : "start and finish not reported"}
        />
        <Kpi label="Weekend days worked" icon={Sun} accent="amber" value={pending ? dash : String(weekendDays)} sub={pending ? undefined : "Saturdays and Sundays with tracked time"} />
      </KpiGrid>

      {pending ? (
        live.loading ? <ReportSkeleton /> : null
      ) : rows.length === 0 ? (
        <NoTimeState period={period} what="Nobody to report on" onWiden={period.custom || range.toolbar.preset !== "30d" ? () => range.choosePreset("30d") : undefined} />
      ) : (
        <>
          <Card>
            <SectionHeading title="Daily presence" sub={`${workdays.length} working ${workdays.length === 1 ? "day" : "days"} · ${period.label}`}>
              <PresenceLegend />
            </SectionHeading>
            <CardContent>
              {workdays.length === 0 ? (
                <p className="py-8 text-center text-sm text-muted-foreground">No working days in this period yet.</p>
              ) : (
                <>
                  <PresenceTable rows={rows} days={workdays} today={period.today} />
                  <PresenceCards rows={rows} days={workdays} today={period.today} />
                </>
              )}
            </CardContent>
          </Card>

          <Card>
            <SectionHeading title="Summary" sub={`${rows.length} ${rows.length === 1 ? "member" : "members"}`} />
            <CardContent>
              <DataTable columns={columns} rows={rows} initialSort={{ key: "worked", dir: "desc" }} footer={footer} emptyText="No members in this scope." />
            </CardContent>
          </Card>
        </>
      )}

      <ReportFootnote>
        Present means at least one tracked block that day; absent means a working day (Monday to Friday) with none, counted only for days
        that have begun — today included. Weekend work is counted apart and never as an absence. Start and finish are the first and last
        tracked minute of each present day, averaged. Days are calendar days of the reporting time zone shown above.
        {!clockKnown && isLive && " This server doesn't report start and finish times yet."}
      </ReportFootnote>
    </ReportShell>
  );
}

/* ----------------------------- Presence grid ----------------------------- */

function PresenceLegend() {
  return (
    <div aria-hidden className="flex flex-wrap items-center gap-x-3 gap-y-1.5 text-xs text-muted-foreground">
      <span className="flex items-center gap-1.5">
        <span className="print-exact h-2.5 w-2.5 rounded bg-success/40" /> Present
      </span>
      <span className="flex items-center gap-1.5">
        <span className="print-exact h-2.5 w-2.5 rounded bg-danger/25" /> Absent
      </span>
      <span className="flex items-center gap-1.5">
        <span className="h-2.5 w-2.5 rounded ring-2 ring-primary" /> Today
      </span>
    </div>
  );
}

/** One member-day: a tick when present, a dash when absent — never colour alone. */
function DayCell({ present, today, label }: { present: boolean; today: boolean; label: string }) {
  return (
    <span
      title={label}
      className={cn(
        "print-exact flex h-9 min-w-[2.5rem] items-center justify-center rounded-lg text-xs font-semibold",
        // On-tint text keeps ≥ 4.5:1 on its own tint in both themes.
        present ? "bg-success/15 text-success-on-tint" : "bg-danger/10 text-danger-on-tint",
        today && "ring-2 ring-primary ring-offset-1 ring-offset-card",
      )}
    >
      <span aria-hidden>{present ? "✓" : "—"}</span>
      <span className="sr-only">{present ? "Present" : "Absent"}</span>
    </span>
  );
}

/**
 * Members × working days, from md up. A real table, so screen readers announce
 * "Rina, Wed 15: Present" from its headers. The scroll box is `relative`: the
 * visually hidden labels are absolutely positioned and would otherwise escape
 * it and widen the page.
 */
function PresenceTable({ rows, days, today }: { rows: Row[]; days: string[]; today: string }) {
  return (
    <div className="relative hidden overflow-x-auto overscroll-x-contain md:block">
      {/* Sized to its days (a short week doesn't stretch into wide bars); a long period scrolls in its box. */}
      <table className="border-separate border-spacing-1 text-left" style={{ minWidth: 180 + days.length * 48 }}>
        <caption className="sr-only">Daily presence by member: whether each member tracked time on each working day.</caption>
        <thead>
          <tr>
            <th scope="col" className="sticky left-0 z-10 w-[180px] bg-card text-xs font-medium text-muted-foreground">
              Member
            </th>
            {days.map((d) => (
              <th key={d} scope="col" className={cn("text-center text-xs font-medium text-muted-foreground", d === today && "text-primary-strong")}>
                <abbr className="no-underline" title={formatDayKey(d, { weekday: true, year: true })}>
                  <span className="block">{d === today ? "Today" : weekdayShort(d)}</span>
                  <span className="block text-[10px] font-normal">{Number(d.slice(8, 10))}</span>
                </abbr>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.userId}>
              <th scope="row" className="sticky left-0 z-10 bg-card py-1 font-normal">
                <span className="block max-w-[11rem] truncate text-sm font-medium" title={r.label}>
                  {r.label}
                </span>
              </th>
              {days.map((d) => (
                <td key={d} className="p-0">
                  <DayCell
                    present={r.presentDays.has(d)}
                    today={d === today}
                    label={`${r.label} — ${r.presentDays.has(d) ? "present" : "absent"} on ${formatDayKey(d, { weekday: true, year: true })}`}
                  />
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Below md: a card per member whose days wrap onto as many rows as they need — every day visible, nothing scrolls sideways. */
function PresenceCards({ rows, days, today }: { rows: Row[]; days: string[]; today: string }) {
  return (
    <ul aria-label="Daily presence by member" className="space-y-3 md:hidden">
      {rows.map((r) => (
        <li key={r.userId} className="rounded-xl border border-border bg-card/50 p-3.5">
          <div className="flex items-center justify-between gap-3">
            <span className="min-w-0 truncate text-sm font-medium">{r.label}</span>
            <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
              {r.present} of {r.present + r.absent} present
            </span>
          </div>
          <ul aria-label={`${r.label}, day by day`} className="mt-3 grid grid-cols-5 gap-1.5">
            {days.map((d) => (
              <li key={d} className="flex flex-col items-center gap-1">
                <span aria-hidden className={cn("text-[10px] font-medium text-muted-foreground", d === today && "text-primary-strong")}>
                  {weekdayShort(d)} {Number(d.slice(8, 10))}
                </span>
                <DayCell
                  present={r.presentDays.has(d)}
                  today={d === today}
                  label={`${formatDayKey(d, { weekday: true })}: ${r.presentDays.has(d) ? "present" : "absent"}`}
                />
              </li>
            ))}
          </ul>
        </li>
      ))}
    </ul>
  );
}
