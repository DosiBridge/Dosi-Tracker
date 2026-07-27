"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Clock, Gauge, Camera, CalendarCheck } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar } from "@/components/ui/avatar";
import { Progress } from "@/components/ui/progress";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ActivityTrendChart } from "@/components/dashboard/charts";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid } from "./report-shell";
import { activities, projects, userById, users } from "@/lib/tenant-data";
import { trackedMembers } from "@/lib/roles";
import { filterActivitiesByRange, rangeForKey, type RangeKey } from "@/lib/reports-data";
import { exportRecords } from "@/lib/export";
import { cn, formatDuration } from "@/lib/utils";
import { getApi } from "@/hooks/useApi";
import type { ResolvedRange } from "./date-range-picker";

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
  totalActivities: number;
  totalTrackedMinutes: number;
  averageProductivity: number;
  perUser: LivePerUser[];
}

interface LiveDailyPoint {
  date: string;
  activityCount: number;
  trackedMinutes: number;
  averageProductivity: number;
}

interface LiveIdentityUser {
  id: string;
  userName: string;
  name?: string | null;
  surname?: string | null;
}

const GUID_RE = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;

/**
 * The shared range presets pivot on the demo NOW constant so mock data stays
 * stable; live queries must pivot on the real clock instead. Mirrors
 * rangeForKey's per-preset logic exactly.
 */
function liveRangeFor(key: RangeKey): { from: Date; to: Date } {
  const to = new Date();
  const from = new Date();
  switch (key) {
    case "today":
      from.setHours(0, 0, 0, 0);
      break;
    case "yesterday":
      from.setDate(from.getDate() - 1);
      from.setHours(0, 0, 0, 0);
      to.setDate(to.getDate() - 1);
      to.setHours(23, 59, 59, 999);
      break;
    case "30d":
      from.setDate(from.getDate() - 30);
      break;
    case "month":
      from.setDate(1);
      from.setHours(0, 0, 0, 0);
      break;
    default: // "7d" and "custom" both fall back to the last 7 days
      from.setDate(from.getDate() - 7);
      break;
  }
  return { from, to };
}

export function TimeActivityReport() {
  const [rangeKey, setRangeKey] = useState<RangeKey>("7d");
  const [range, setRange] = useState<ResolvedRange>(() => ({ key: "7d", ...rangeForKey("7d") }));
  const [projectId, setProjectId] = useState("all");
  const [memberId, setMemberId] = useState("all");
  const [viewMode, setViewMode] = useState<"table" | "grid">("table");

  const [liveSummary, setLiveSummary] = useState<LiveSummary | null>(null);
  const [liveSeries, setLiveSeries] = useState<LiveDailyPoint[] | null>(null);
  const [liveUsers, setLiveUsers] = useState<LiveIdentityUser[]>([]);
  const [liveLoading, setLiveLoading] = useState(false);
  const [liveError, setLiveError] = useState(false);

  // The filter selects list demo entities (non-GUID ids) — the backend can only
  // filter by real GUIDs, so anything else keeps the mock rendering below.
  const liveApplicable =
    (projectId === "all" || GUID_RE.test(projectId)) &&
    (memberId === "all" || GUID_RE.test(memberId));

  const loadLive = useCallback(async () => {
    setLiveLoading(true);
    setLiveError(false);
    try {
      const { from, to } = liveRangeFor(range.key);
      const qs = new URLSearchParams({ From: from.toISOString(), To: to.toISOString() });
      if (projectId !== "all") qs.set("ProjectId", projectId);
      const [summary, series] = await Promise.all([
        getApi(`/api/app/reporting/summary?${qs.toString()}`),
        getApi(`/api/app/reporting/daily-series?${qs.toString()}`),
      ]);
      setLiveSummary(summary && typeof summary === "object" ? (summary as LiveSummary) : null);
      setLiveSeries(Array.isArray(series) ? (series as LiveDailyPoint[]) : []);
      // Best effort: admins resolve member names, workers get a 403 -> keep ids.
      const identityUsers = await getApi("/api/identity/users?MaxResultCount=100").catch(() => []);
      setLiveUsers(Array.isArray(identityUsers) ? (identityUsers as LiveIdentityUser[]) : []);
    } catch {
      setLiveSummary(null); // fall back to the demo dataset below
      setLiveSeries(null);
      setLiveError(true);
    } finally {
      setLiveLoading(false);
    }
  }, [range, projectId]);

  useEffect(() => {
    if (typeof window === "undefined" || !localStorage.getItem("dosi-token")) return;
    if (!liveApplicable) return;
    loadLive();
  }, [loadLive, liveApplicable]);

  // Live data only counts while the backend could honor the current filters;
  // demo-only selections render the mock aggregation below instead.
  const activeSummary = liveApplicable ? liveSummary : null;
  const activeSeries = liveApplicable ? liveSeries : null;
  const live = activeSummary !== null && activeSeries !== null;
  const showLiveLoading = liveApplicable && liveLoading && !live;
  const showLiveError = liveApplicable && liveError;

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
      const iso = a.endedAt.slice(0, 10);
      if (days[iso]) {
        const p = projects.find((pr) => pr.id === a.projectId);
        const mins = p?.intervalMinutes ?? 10;
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
      const tracked = ua.reduce((s, a) => {
        const p = projects.find((pr) => pr.id === a.projectId);
        return s + (p?.intervalMinutes ?? 10);
      }, 0);
      const activity = ua.length ? Math.round(ua.reduce((s, a) => s + a.productivity, 0) / ua.length) : u.productivity;
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

  const liveTrend = useMemo(() => {
    if (!activeSeries) return null;
    return activeSeries.map((d) => ({
      day: new Date(d.date).toLocaleDateString("en", { weekday: "short", timeZone: "UTC" }),
      tracked: Math.round(d.trackedMinutes),
      productive: Math.round((d.trackedMinutes * d.averageProductivity) / 100),
    }));
  }, [activeSeries]);

  const liveRows = useMemo<Row[] | null>(() => {
    if (!activeSummary) return null;
    const perUserList = Array.isArray(activeSummary.perUser) ? activeSummary.perUser : [];
    const filtered = memberId === "all" ? perUserList : perUserList.filter((u) => u.userId === memberId);
    return filtered.map((u) => {
      const identity = liveUsers.find((x) => x.id === u.userId);
      const identityName = identity ? `${identity.name ?? ""} ${identity.surname ?? ""}`.trim() : "";
      const mockUser = userById(u.userId);
      const tracked = Math.round(u.trackedMinutes);
      const activity = Math.round(u.averageProductivity);
      return {
        userId: u.userId,
        name: identityName || identity?.userName || mockUser?.name || u.userId.slice(0, 8),
        designation: mockUser?.designation ?? "Member",
        tracked,
        activity,
        sessions: u.activityCount,
        productive: Math.round((tracked * activity) / 100),
        topApp: "—",
      };
    });
  }, [activeSummary, liveUsers, memberId]);

  const rows = liveRows ?? mockRows;
  const trendData = liveTrend ?? dynamicTrend;

  let totalTracked: number;
  let avgActivity: number;
  let totalSessions: number;
  if (activeSummary && liveRows) {
    if (memberId === "all") {
      totalTracked = Math.round(activeSummary.totalTrackedMinutes);
      avgActivity = Math.round(activeSummary.averageProductivity);
      totalSessions = activeSummary.totalActivities;
    } else {
      totalTracked = liveRows.reduce((s, r) => s + r.tracked, 0);
      avgActivity =
        totalTracked > 0
          ? Math.round(liveRows.reduce((s, r) => s + r.tracked * r.activity, 0) / totalTracked)
          : 0;
      totalSessions = liveRows.reduce((s, r) => s + r.sessions, 0);
    }
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

      {showLiveLoading ? (
        <Card>
          <CardContent className="py-16 text-center text-sm text-muted-foreground">
            Loading live report data…
          </CardContent>
        </Card>
      ) : (
        <>
      {showLiveError && (
        <div className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">
          Live data unavailable right now — showing demo data.
        </div>
      )}

      <KpiGrid>
        <Kpi label="Total tracked" value={formatDuration(totalTracked)} icon={Clock} tone="#6d5efc" />
        <Kpi label="Avg activity" value={`${avgActivity}%`} icon={Gauge} tone="#22c55e" />
        <Kpi label="Sessions" value={String(totalSessions)} icon={CalendarCheck} tone="#0ea5e9" />
        <Kpi label="Screenshots" value={String(totalSessions)} icon={Camera} tone="#ec4899" sub="auto-captured" />
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
            <DataTable columns={columns} rows={rows} initialSort={{ key: "tracked", dir: "desc" }} />
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
        </>
      )}
    </ReportShell>
  );
}
