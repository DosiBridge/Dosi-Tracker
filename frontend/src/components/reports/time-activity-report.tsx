"use client";

import { useCallback, useMemo, useState } from "react";
import { Clock, Gauge, Camera, CalendarCheck, Users } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar } from "@/components/ui/avatar";
import { Progress } from "@/components/ui/progress";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ActivityTrendChart } from "@/components/dashboard/charts";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid, LiveReportNotice, useLiveReport } from "./report-shell";
import { activities, userById, users } from "@/lib/tenant-data";
import { trackedMembers } from "@/lib/roles";
import { averageProductivity, dayKey, durationMinutes, minutesForUser } from "@/lib/metrics";
import { filterActivitiesByRange, type RangeKey } from "@/lib/reports-data";
import { asArray, memberName, reportQuery } from "@/lib/report-math";
import { exportRecords } from "@/lib/export";
import { cn, formatDuration } from "@/lib/utils";
import { getApi } from "@/hooks/useApi";
import { useSession } from "@/components/session-provider";
import { resolveRange, type ResolvedRange } from "./date-range-picker";

interface Row {
  userId: string;
  name: string;
  designation: string;
  tracked: number;
  activity: number;
  sessions: number;
  productive: number;
  topApp: string;
}

/* ---- Live (API) shapes: /api/app/reporting/* ---- */

interface LivePerUser {
  userId: string;
  activityCount: number;
  trackedMinutes: number;
  averageProductivity: number;
}

interface LiveSummary {
  totalActivities?: number;
  totalTrackedMinutes?: number;
  averageProductivity?: number;
  perUser?: LivePerUser[];
}

interface LiveDailyPoint {
  date: string;
  activityCount: number;
  trackedMinutes: number;
  averageProductivity: number;
}

interface LiveTimeActivity {
  summary: LiveSummary;
  series: LiveDailyPoint[];
}

export function TimeActivityReport() {
  const { isLive } = useSession();
  const [rangeKey, setRangeKey] = useState<RangeKey>("7d");
  const [range, setRange] = useState<ResolvedRange>(() => resolveRange("7d"));
  const [projectId, setProjectId] = useState("all");
  const [memberId, setMemberId] = useState("all");
  const [viewMode, setViewMode] = useState<"table" | "grid">("table");

  // LIVE: the project and member filters are real ids, applied server-side.
  const loadLive = useCallback(async (): Promise<LiveTimeActivity> => {
    const qs = reportQuery({ from: range.from, to: range.to, projectId, userId: memberId });
    const [summary, series] = await Promise.all([
      getApi(`/api/app/reporting/summary?${qs}`),
      getApi(`/api/app/reporting/daily-series?${qs}`),
    ]);
    return {
      summary: summary && typeof summary === "object" ? (summary as LiveSummary) : {},
      series: asArray<LiveDailyPoint>(series),
    };
  }, [range, projectId, memberId]);

  const live = useLiveReport(isLive, loadLive);

  const acts = useMemo(() => {
    let list = filterActivitiesByRange(activities, range.from, range.to);
    if (projectId !== "all") list = list.filter((a) => a.projectId === projectId);
    if (memberId !== "all") list = list.filter((a) => a.userId === memberId);
    return list;
  }, [range, projectId, memberId]);

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
    const members = trackedMembers(users).filter(
      (u) => memberId === "all" || u.id === memberId
    );

    return members.map((u) => {
      const ua = acts.filter((a) => a.userId === u.id);
      const tracked = minutesForUser(acts, u.id);
      const activity = ua.length ? averageProductivity(ua) : u.productivity;
      const appCount = new Map<string, number>();
      ua.forEach((a) => appCount.set(a.screen.app, (appCount.get(a.screen.app) ?? 0) + 1));
      const topApp = [...appCount.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? "—";
      return {
        userId: u.id,
        name: u.name,
        designation: u.designation,
        tracked,
        activity,
        sessions: ua.length,
        productive: Math.round((tracked * activity) / 100),
        topApp,
      };
    });
  }, [acts, memberId]);

  const liveTrend = useMemo(
    () =>
      (live.data?.series ?? []).map((d) => ({
        day: new Date(d.date).toLocaleDateString("en", { weekday: "short", timeZone: "UTC" }),
        tracked: Math.round(d.trackedMinutes),
        productive: Math.round((d.trackedMinutes * d.averageProductivity) / 100),
      })),
    [live.data],
  );

  const liveRows = useMemo<Row[]>(
    () =>
      asArray<LivePerUser>(live.data?.summary.perUser)
        .filter((u) => memberId === "all" || u.userId === memberId)
        .map((u) => {
          const member = userById(u.userId);
          const tracked = Math.round(u.trackedMinutes);
          const activity = Math.round(u.averageProductivity);
          return {
            userId: u.userId,
            name: memberName(u.userId, member),
            designation: member?.designation ?? "Member",
            tracked,
            activity,
            sessions: u.activityCount,
            productive: Math.round((tracked * activity) / 100),
            topApp: "—",
          };
        }),
    [live.data, memberId],
  );

  // A live session shows only real rows (or nothing) — never the demo team.
  const rows = isLive ? liveRows : mockRows;
  const trendData = isLive ? liveTrend : dynamicTrend;

  let totalTracked: number;
  let avgActivity: number;
  let totalSessions: number;
  if (isLive) {
    // The summary is already scoped by the ProjectId/UserId filters.
    const s = live.data?.summary;
    totalTracked = Math.round(Number(s?.totalTrackedMinutes) || 0);
    avgActivity = Math.round(Number(s?.averageProductivity) || 0);
    totalSessions = Number(s?.totalActivities) || 0;
  } else {
    totalTracked = rows.reduce((s, r) => s + r.tracked, 0);
    avgActivity = rows.length ? Math.round(rows.reduce((s, r) => s + r.activity, 0) / rows.length) : 0;
    totalSessions = rows.reduce((s, r) => s + r.sessions, 0);
  }

  const columns: Column<Row>[] = [
    {
      key: "name",
      header: "Member",
      sortValue: (r) => r.name,
      render: (r) => {
        const u = userById(r.userId);
        return (
          <div className="flex items-center gap-2.5">
            <Avatar name={r.name} size="sm" status={u?.status} />
            <div className="min-w-0">
              <div className="truncate font-medium">{r.name}</div>
              <div className="truncate text-xs text-muted-foreground">{r.designation}</div>
            </div>
          </div>
        );
      },
    },
    { key: "tracked", header: "Tracked", align: "right", sortValue: (r) => r.tracked, render: (r) => <span className="font-medium">{formatDuration(r.tracked)}</span> },
    {
      key: "activity",
      header: "Activity",
      align: "right",
      sortValue: (r) => r.activity,
      render: (r) => (
        <div className="ml-auto flex w-28 items-center gap-2">
          <Progress value={r.activity} />
          <span className="w-9 text-right text-xs font-medium">{r.activity}%</span>
        </div>
      ),
    },
    { key: "productive", header: "Productive", align: "right", sortValue: (r) => r.productive, render: (r) => formatDuration(r.productive) },
    { key: "sessions", header: "Sessions", align: "right", sortValue: (r) => r.sessions, render: (r) => r.sessions },
    { key: "topApp", header: "Top app", sortValue: (r) => r.topApp, render: (r) => <span className="text-muted-foreground">{r.topApp}</span> },
  ];

  function onRange(r: ResolvedRange) {
    setRange(r);
    setRangeKey(r.key);
  }

  function doExport() {
    exportRecords(`time-activity-${range.key}`, rows, [
      { header: "Member", value: (r) => r.name },
      { header: "Designation", value: (r) => r.designation },
      { header: "Tracked (min)", value: (r) => r.tracked },
      { header: "Activity %", value: (r) => r.activity },
      { header: "Productive (min)", value: (r) => r.productive },
      { header: "Sessions", value: (r) => r.sessions },
      { header: "Top app", value: (r) => r.topApp },
    ]);
  }

  return (
    <ReportShell
      title="Time & Activity"
      description="Time worked, activity level, and app usage per member."
      actions={<ExportMenu onExportCSV={doExport} />}
    >
      <FilterBar
        rangeKey={rangeKey}
        onRange={onRange}
        projectId={projectId}
        onProject={setProjectId}
        memberId={memberId}
        onMember={setMemberId}
      />

      {isLive && <LiveReportNotice loading={live.loading} error={live.error} onRetry={live.retry} />}

      <KpiGrid>
        <Kpi label="Total tracked" value={formatDuration(totalTracked)} icon={Clock} tone="#6d5efc" />
        <Kpi label="Avg activity" value={`${avgActivity}%`} icon={Gauge} tone="#22c55e" />
        <Kpi label="Sessions" value={String(totalSessions)} icon={CalendarCheck} tone="#0ea5e9" />
        {isLive ? (
          // The summary carries no screenshot count; don't restate sessions as screenshots.
          <Kpi label="Members tracked" value={String(rows.length)} icon={Users} tone="#ec4899" />
        ) : (
          <Kpi label="Screenshots" value={String(totalSessions)} icon={Camera} tone="#ec4899" sub="auto-captured" />
        )}
      </KpiGrid>

      <Card>
        <CardHeader>
          <CardTitle>Tracked vs productive time</CardTitle>
          <span className="text-xs text-muted-foreground">{range.label}</span>
        </CardHeader>
        <CardContent>
          <ActivityTrendChart data={trendData} />
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <div>
            <CardTitle>Breakdown by member</CardTitle>
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
              emptyText={isLive ? "No tracked time for these filters." : undefined}
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
                        <p className="text-xs text-muted-foreground truncate">{r.designation}</p>
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
                    <div className="mt-4 space-y-1.5">
                      <div className="flex justify-between text-xs text-muted-foreground">
                        <span>Activity Level</span>
                        <span className="font-semibold text-foreground">{r.activity}%</span>
                      </div>
                      <Progress value={r.activity} />
                    </div>
                    <div className="mt-4 flex items-center justify-between text-xs text-muted-foreground border-t border-border/60 pt-3">
                      <span>Sessions: <strong className="text-foreground">{r.sessions}</strong></span>
                      <span className="truncate max-w-[140px]">Top App: <strong className="text-foreground">{r.topApp}</strong></span>
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
