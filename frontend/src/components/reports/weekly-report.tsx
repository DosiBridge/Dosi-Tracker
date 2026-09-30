"use client";

import { useCallback, useMemo, useState } from "react";
import { CalendarDays, Clock, Gauge, Users } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Ring } from "@/components/ui/ring";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ActivityTrendChart } from "@/components/dashboard/charts";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid, LiveReportNotice, useLiveReport } from "./report-shell";
import { activities, projects, userById, users } from "@/lib/tenant-data";
import { trackedMembers } from "@/lib/roles";
import { cn } from "@/lib/utils";
import {
  averageProductivity,
  dayKey,
  distinctActiveDays,
  durationMinutes,
  minutesByProject,
  minutesForUser,
} from "@/lib/metrics";
import { filterActivitiesByRange, type RangeKey } from "@/lib/reports-data";
import { activeDaysByUser, asArray, memberName, reportQuery, type UserDayRow } from "@/lib/report-math";
import { exportRecords } from "@/lib/export";
import { formatDuration } from "@/lib/utils";
import { getApi } from "@/hooks/useApi";
import { useSession } from "@/components/session-provider";
import { resolveRange, type ResolvedRange } from "./date-range-picker";

interface Row {
  userId: string;
  name: string;
  tracked: number;
  activity: number;
  productive: number;
  /** null in live mode when the per-member day series couldn't be read. */
  daysActive: number | null;
  topProject: string;
}

/* ---- Live (API) shapes: /api/app/reporting/* ---- */

interface LivePerUser {
  userId: string;
  activityCount: number;
  trackedMinutes: number;
  averageProductivity: number;
}

interface LiveSummary {
  totalTrackedMinutes?: number;
  averageProductivity?: number;
  perUser?: LivePerUser[];
}

interface LiveDailyPoint {
  date: string;
  trackedMinutes: number;
  averageProductivity: number;
}

interface LiveWeekly {
  summary: LiveSummary;
  series: LiveDailyPoint[];
  /** userId → distinct days with activity; null when the per-member series is unavailable. */
  activeDays: Map<string, number> | null;
}

export function WeeklyReport() {
  const { isLive } = useSession();
  const [rangeKey, setRangeKey] = useState<RangeKey>("7d");
  const [range, setRange] = useState<ResolvedRange>(() => resolveRange("7d"));
  const [viewMode, setViewMode] = useState<"table" | "grid">("table");

  const loadLive = useCallback(async (): Promise<LiveWeekly> => {
    const qs = reportQuery({ from: range.from, to: range.to });
    const [summary, series, userDays] = await Promise.all([
      getApi(`/api/app/reporting/summary?${qs}`),
      getApi(`/api/app/reporting/daily-series?${qs}`),
      // Days-active per member is a detail column: without it the column reads "—".
      getApi(`/api/app/reporting/user-daily-series?${qs}`).catch(() => null),
    ]);
    return {
      summary: summary && typeof summary === "object" ? (summary as LiveSummary) : {},
      series: asArray<LiveDailyPoint>(series),
      activeDays: Array.isArray(userDays) ? activeDaysByUser(userDays as UserDayRow[]) : null,
    };
  }, [range]);

  const live = useLiveReport(isLive, loadLive);

  const members = trackedMembers(users);

  const acts = useMemo(() => filterActivitiesByRange(activities, range.from, range.to), [range]);

  const dynamicTrend = useMemo(() => {
    const days: Record<string, { day: string; tracked: number; productive: number }> = {};
    const DAY_MS = 24 * 60 * 60 * 1000;
    const count = Math.round((range.to.getTime() - range.from.getTime()) / DAY_MS) + 1;
    
    for (let i = 0; i < count; i++) {
      const date = new Date(range.from.getTime() + i * DAY_MS);
      const iso = date.toISOString().slice(0, 10);
      const label = date.toLocaleDateString("en", { weekday: "short", timeZone: "UTC" });
      days[iso] = { day: label, tracked: 0, productive: 0 };
    }
    
    acts.forEach((a) => {
      const iso = dayKey(a);
      if (days[iso]) {
        const mins = durationMinutes(a);
        days[iso].tracked += mins;
        days[iso].productive += Math.round((mins * a.productivity) / 100);
      }
    });

    return Object.values(days);
  }, [acts, range]);

  const mockRows = useMemo<Row[]>(() => {
    return members.map((u) => {
      const ua = acts.filter((a) => a.userId === u.id);
      const tracked = minutesForUser(acts, u.id);
      const activity = ua.length ? averageProductivity(ua) : u.productivity;

      const projectTimes = minutesByProject(ua);
      let topProjId = "";
      let maxTime = -1;
      projectTimes.forEach((v, k) => {
        if (v > maxTime) {
          maxTime = v;
          topProjId = k;
        }
      });
      const topProj = projects.find((p) => p.id === topProjId)?.title ?? "—";

      return {
        userId: u.id,
        name: u.name,
        tracked,
        activity,
        productive: Math.round((tracked * activity) / 100),
        daysActive: distinctActiveDays(ua, u.id),
        topProject: topProj,
      };
    });
  }, [members, acts]);

  const liveTrend = useMemo(
    () =>
      (live.data?.series ?? []).map((d) => ({
        day: new Date(d.date).toLocaleDateString("en", { weekday: "short", timeZone: "UTC" }),
        tracked: Math.round(d.trackedMinutes),
        productive: Math.round((d.trackedMinutes * d.averageProductivity) / 100),
      })),
    [live.data],
  );

  const liveRows = useMemo<Row[]>(() => {
    const activeDays = live.data?.activeDays ?? null;
    return asArray<LivePerUser>(live.data?.summary.perUser).map((u) => {
      const tracked = Math.round(u.trackedMinutes);
      const activity = Math.round(u.averageProductivity);
      return {
        userId: u.userId,
        name: memberName(u.userId, userById(u.userId)),
        tracked,
        activity,
        productive: Math.round((tracked * activity) / 100),
        daysActive: activeDays ? (activeDays.get(u.userId) ?? 0) : null,
        topProject: "—",
      };
    });
  }, [live.data]);

  // A live session shows only real rows (or nothing) — never the demo team.
  const rows = isLive ? liveRows : mockRows;
  const trend = isLive ? liveTrend : dynamicTrend;

  const totalTracked = rows.reduce((s, r) => s + r.tracked, 0);
  // Live: the backend's duration-weighted team average; demo: the mean of member scores.
  const avgActivity = isLive
    ? Math.round(Number(live.data?.summary.averageProductivity) || 0)
    : rows.length
      ? Math.round(rows.reduce((s, r) => s + r.activity, 0) / rows.length)
      : 0;
  // A day with no productive time is not a "best day" (an empty range has none).
  const bestDay = [...trend].sort((a, b) => b.productive - a.productive).find((d) => d.productive > 0);

  const columns: Column<Row>[] = [
    { key: "name", header: "Member", sortValue: (r) => r.name, render: (r) => (
      <div className="flex items-center gap-2.5">
        <Avatar name={r.name} size="sm" status={userById(r.userId)?.status} />
        <span className="font-medium">{r.name}</span>
      </div>
    )},
    { key: "tracked", header: "Tracked", align: "right", sortValue: (r) => r.tracked, render: (r) => <span className="font-medium">{formatDuration(r.tracked)}</span> },
    { key: "productive", header: "Productive", align: "right", sortValue: (r) => r.productive, render: (r) => formatDuration(r.productive) },
    { key: "activity", header: "Activity", align: "right", sortValue: (r) => r.activity, render: (r) => <Badge tone={r.activity >= 75 ? "success" : r.activity >= 55 ? "warning" : "danger"}>{r.activity}%</Badge> },
    // Demo keeps its "n/5" work-week read; live shows the real count of active days in the range.
    { key: "daysActive", header: "Days", align: "right", sortValue: (r) => r.daysActive ?? -1, render: (r) => (r.daysActive == null ? "—" : isLive ? String(r.daysActive) : `${r.daysActive}/5`) },
    { key: "topProject", header: "Top project", sortValue: (r) => r.topProject, render: (r) => <span className="text-muted-foreground">{r.topProject}</span> },
  ];

  function onRange(r: ResolvedRange) { setRange(r); setRangeKey(r.key); }
  function doExport() {
    exportRecords(`weekly-summary-${range.key}`, rows, [
      { header: "Member", value: (r) => r.name },
      { header: "Tracked (min)", value: (r) => r.tracked },
      { header: "Productive (min)", value: (r) => r.productive },
      { header: "Activity %", value: (r) => r.activity },
      { header: "Days active", value: (r) => r.daysActive },
      { header: "Top project", value: (r) => r.topProject },
    ]);
  }

  return (
    <ReportShell
      title="Weekly Summary"
      description="A rolled-up weekly digest of team activity and productivity."
      actions={<ExportMenu onExportCSV={doExport} />}
    >
      <FilterBar rangeKey={rangeKey} onRange={onRange} />

      {isLive && <LiveReportNotice loading={live.loading} error={live.error} onRetry={live.retry} />}

      <KpiGrid>
        <Kpi label="Team hours" value={formatDuration(totalTracked)} icon={Clock} tone="#6d5efc" sub={range.label} />
        <Kpi label="Avg activity" value={`${avgActivity}%`} icon={Gauge} tone="#22c55e" />
        <Kpi label="Best day" value={bestDay?.day ?? "—"} icon={CalendarDays} tone="#0ea5e9" sub={bestDay ? formatDuration(bestDay.productive) + " productive" : ""} />
        <Kpi label="Active members" value={String(rows.length)} icon={Users} tone="#ec4899" />
      </KpiGrid>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <CardHeader><CardTitle>Daily activity this week</CardTitle></CardHeader>
          <CardContent><ActivityTrendChart data={trend} /></CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>Team focus</CardTitle></CardHeader>
          <CardContent className="flex flex-col items-center justify-center gap-3 py-6">
            <Ring value={avgActivity} size={132} />
            <p className="text-center text-sm text-muted-foreground">Average activity across {rows.length} members this week.</p>
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <div>
            <CardTitle>Member digest</CardTitle>
            <span className="text-xs text-muted-foreground">{rows.length} members</span>
          </div>
          <div className="flex items-center gap-1 rounded-lg bg-muted p-0.5 text-xs">
            <button
              onClick={() => setViewMode("table")}
              className={cn("px-2.5 py-1.5 rounded-md transition-all", viewMode === "table" ? "bg-card text-foreground shadow-sm font-semibold" : "text-muted-foreground hover:text-foreground")}
            >
              Table
            </button>
            <button
              onClick={() => setViewMode("grid")}
              className={cn("px-2.5 py-1.5 rounded-md transition-all", viewMode === "grid" ? "bg-card text-foreground shadow-sm font-semibold" : "text-muted-foreground hover:text-foreground")}
            >
              Grid
            </button>
          </div>
        </CardHeader>
        <CardContent>
          {viewMode === "table" ? (
            <DataTable
              columns={columns}
              rows={rows}
              initialSort={{ key: "tracked", dir: "desc" }}
              emptyText={isLive ? "No tracked time in this range." : undefined}
            />
          ) : (
            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {rows.map((r) => {
                const u = userById(r.userId);
                return (
                  <Card key={r.userId} className="p-4 hover:border-primary/40 transition-colors">
                    <div className="flex items-center gap-3">
                      <Avatar name={r.name} size="md" status={u?.status} />
                      <div className="min-w-0 flex-1">
                        <h4 className="font-semibold truncate text-sm">{r.name}</h4>
                        <p className="text-xs text-muted-foreground truncate">{u?.designation}</p>
                      </div>
                    </div>
                    <div className="mt-4 grid grid-cols-2 gap-2 border-t border-border/60 pt-3 text-xs">
                      <div>
                        <span className="text-muted-foreground">Tracked</span>
                        <div className="text-sm font-bold text-foreground mt-0.5">{formatDuration(r.tracked)}</div>
                      </div>
                      <div>
                        <span className="text-muted-foreground">Productive</span>
                        <div className="text-sm font-bold text-success mt-0.5">{formatDuration(r.productive)}</div>
                      </div>
                    </div>
                    <div className="mt-4 flex items-center justify-between text-xs text-muted-foreground border-t border-border/60 pt-3">
                      <span>Activity: <strong className="text-foreground">{r.activity}%</strong></span>
                      <span>Days Active: <strong className="text-foreground">{r.daysActive ?? "—"}</strong></span>
                    </div>
                    <div className="mt-2 text-xs text-muted-foreground">
                      Top Project: <Badge tone="muted" className="ml-1 font-semibold">{r.topProject}</Badge>
                    </div>
                  </Card>
                );
              })}
            </div>
          )}
        </CardContent>
      </Card>
    </ReportShell>
  );
}
