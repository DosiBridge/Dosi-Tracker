"use client";

import { useMemo, useState } from "react";
import { Clock, Gauge, Camera, CalendarCheck } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar } from "@/components/ui/avatar";
import { Progress } from "@/components/ui/progress";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ActivityTrendChart } from "@/components/dashboard/charts";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid } from "./report-shell";
import { activities, projects, userById, users, weeklyTrend } from "@/lib/tenant-data";
import { filterActivitiesByRange, rangeForKey, type RangeKey } from "@/lib/reports-data";
import { exportRecords } from "@/lib/export";
import { formatDuration } from "@/lib/utils";
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

  const rows = useMemo<Row[]>(() => {
    let acts = filterActivitiesByRange(activities, range.from, range.to);
    if (projectId !== "all") acts = acts.filter((a) => a.projectId === projectId);
    if (memberId !== "all") acts = acts.filter((a) => a.userId === memberId);

    const members = users.filter(
      (u) => (u.role === "worker" || u.role === "owner") && (memberId === "all" || u.id === memberId)
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
  }, [range, projectId, memberId]);

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
          <ActivityTrendChart data={weeklyTrend} />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Breakdown by member</CardTitle>
          <span className="text-xs text-muted-foreground">{rows.length} members</span>
        </CardHeader>
        <CardContent>
          <DataTable columns={columns} rows={rows} initialSort={{ key: "tracked", dir: "desc" }} />
        </CardContent>
      </Card>
    </ReportShell>
  );
}
