"use client";

import { useMemo, useState } from "react";
import { CalendarDays, Clock, Gauge, Users } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Ring } from "@/components/ui/ring";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ActivityTrendChart } from "@/components/dashboard/charts";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid } from "./report-shell";
import { productivitySplit, projects, userById, users, weeklyTrend } from "@/lib/tenant-data";
import { rangeForKey, type RangeKey } from "@/lib/reports-data";
import { exportRecords } from "@/lib/export";
import { formatDuration } from "@/lib/utils";
import type { ResolvedRange } from "./date-range-picker";

interface Row {
  userId: string;
  name: string;
  tracked: number;
  activity: number;
  productive: number;
  daysActive: number;
  topProject: string;
}

export function WeeklyReport() {
  const [rangeKey, setRangeKey] = useState<RangeKey>("7d");
  const [range, setRange] = useState<ResolvedRange>(() => ({ key: "7d", ...rangeForKey("7d") }));

  const members = users.filter((u) => u.role === "worker" || u.role === "owner");

  const rows = useMemo<Row[]>(
    () =>
      members.map((u) => {
        const split = productivitySplit.find((s) => s.userId === u.id);
        const weekTracked = u.trackedToday * 5 + Math.round(u.trackedToday * 0.6);
        const proj = projects.find((p) => p.memberIds.includes(u.id));
        return {
          userId: u.id,
          name: u.name,
          tracked: weekTracked,
          activity: u.productivity,
          productive: split ? split.productive * 5 : Math.round((weekTracked * u.productivity) / 100),
          daysActive: u.status === "offline" ? 4 : 5,
          topProject: proj?.title ?? "—",
        };
      }),
    [members]
  );

  const totalTracked = rows.reduce((s, r) => s + r.tracked, 0);
  const avgActivity = rows.length ? Math.round(rows.reduce((s, r) => s + r.activity, 0) / rows.length) : 0;
  const bestDay = [...weeklyTrend].sort((a, b) => b.productive - a.productive)[0];

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
    { key: "daysActive", header: "Days", align: "right", sortValue: (r) => r.daysActive, render: (r) => `${r.daysActive}/5` },
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

      <KpiGrid>
        <Kpi label="Team hours" value={formatDuration(totalTracked)} icon={Clock} tone="#6d5efc" sub={range.label} />
        <Kpi label="Avg activity" value={`${avgActivity}%`} icon={Gauge} tone="#22c55e" />
        <Kpi label="Best day" value={bestDay?.day ?? "—"} icon={CalendarDays} tone="#0ea5e9" sub={bestDay ? formatDuration(bestDay.productive) + " productive" : ""} />
        <Kpi label="Active members" value={String(rows.length)} icon={Users} tone="#ec4899" />
      </KpiGrid>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <CardHeader><CardTitle>Daily activity this week</CardTitle></CardHeader>
          <CardContent><ActivityTrendChart data={weeklyTrend} /></CardContent>
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
        <CardHeader><CardTitle>Member digest</CardTitle></CardHeader>
        <CardContent>
          <DataTable columns={columns} rows={rows} initialSort={{ key: "tracked", dir: "desc" }} />
        </CardContent>
      </Card>
    </ReportShell>
  );
}
