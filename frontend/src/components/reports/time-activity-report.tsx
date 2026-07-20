"use client";

import { useMemo, useState } from "react";
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

export function TimeActivityReport() {
  const [rangeKey, setRangeKey] = useState<RangeKey>("7d");
  const [range, setRange] = useState<ResolvedRange>(() => ({ key: "7d", ...rangeForKey("7d") }));
  const [projectId, setProjectId] = useState("all");
  const [memberId, setMemberId] = useState("all");
  const [viewMode, setViewMode] = useState<"table" | "grid">("table");

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

  const rows = useMemo<Row[]>(() => {
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

  const totalTracked = rows.reduce((s, r) => s + r.tracked, 0);
  const avgActivity = rows.length ? Math.round(rows.reduce((s, r) => s + r.activity, 0) / rows.length) : 0;
  const totalSessions = rows.reduce((s, r) => s + r.sessions, 0);

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
          <ActivityTrendChart data={dynamicTrend} />
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
    </ReportShell>
  );
}
