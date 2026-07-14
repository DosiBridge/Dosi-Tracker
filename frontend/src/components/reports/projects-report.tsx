"use client";

import { useMemo, useState } from "react";
import { Bar, BarChart, Cell, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { FolderKanban, Clock, Users, DollarSign } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid } from "./report-shell";
import { projects, userById } from "@/lib/tenant-data";
import { rangeForKey, type RangeKey } from "@/lib/reports-data";
import { exportRecords } from "@/lib/export";
import { formatDuration } from "@/lib/utils";
import type { Project } from "@/lib/types";
import type { ResolvedRange } from "./date-range-picker";

const BLENDED_RATE = 48; // $/hr for cost estimate

const tooltipStyle = {
  borderRadius: 12,
  border: "1px solid var(--border)",
  background: "var(--card)",
  fontSize: 12,
  boxShadow: "var(--elev-lg)",
} as const;

export function ProjectsReport() {
  const [rangeKey, setRangeKey] = useState<RangeKey>("30d");
  const [range, setRange] = useState<ResolvedRange>(() => ({ key: "30d", ...rangeForKey("30d") }));

  const useMonth = rangeKey === "30d" || rangeKey === "month";
  const rows = useMemo(() => projects.filter((p) => !p.archived), []);
  const minutesOf = (p: Project) => (useMonth ? p.loggedThisMonth : p.loggedThisWeek);

  const totalMinutes = rows.reduce((s, p) => s + minutesOf(p), 0);
  const totalCost = Math.round((totalMinutes / 60) * BLENDED_RATE);
  const topProject = [...rows].sort((a, b) => minutesOf(b) - minutesOf(a))[0];

  const chartData = [...rows]
    .sort((a, b) => minutesOf(b) - minutesOf(a))
    .map((p) => ({ name: p.title, minutes: minutesOf(p), color: p.color }));

  const columns: Column<Project>[] = [
    { key: "title", header: "Project", sortValue: (r) => r.title, render: (r) => (
      <span className="flex items-center gap-2 font-medium">
        <span className="h-3 w-3 rounded" style={{ background: r.color }} /> {r.title}
      </span>
    )},
    { key: "members", header: "Team", render: (r) => (
      <div className="flex -space-x-2">
        {r.memberIds.slice(0, 4).map((id) => {
          const u = userById(id);
          return u ? <Avatar key={id} name={u.name} size="sm" /> : null;
        })}
        {r.memberIds.length > 4 && (
          <span className="flex h-8 w-8 items-center justify-center rounded-full bg-muted text-xs ring-2 ring-card">+{r.memberIds.length - 4}</span>
        )}
      </div>
    )},
    { key: "time", header: useMonth ? "This month" : "This week", align: "right", sortValue: (r) => minutesOf(r), render: (r) => <span className="font-medium">{formatDuration(minutesOf(r))}</span> },
    { key: "total", header: "Total logged", align: "right", sortValue: (r) => r.loggedTotal, render: (r) => formatDuration(r.loggedTotal) },
    { key: "share", header: "Share", align: "right", sortValue: (r) => minutesOf(r), render: (r) => <Badge tone="muted">{totalMinutes ? Math.round((minutesOf(r) / totalMinutes) * 100) : 0}%</Badge> },
    { key: "cost", header: "Est. cost", align: "right", sortValue: (r) => minutesOf(r), render: (r) => <span className="font-medium">${Math.round((minutesOf(r) / 60) * BLENDED_RATE).toLocaleString()}</span> },
  ];

  function onRange(r: ResolvedRange) { setRange(r); setRangeKey(r.key); }
  function doExport() {
    exportRecords(`projects-${range.key}`, rows, [
      { header: "Project", value: (r) => r.title },
      { header: "Members", value: (r) => r.memberIds.length },
      { header: `${useMonth ? "Month" : "Week"} (min)`, value: (r) => minutesOf(r) },
      { header: "Total (min)", value: (r) => r.loggedTotal },
      { header: "Est. cost USD", value: (r) => Math.round((minutesOf(r) / 60) * BLENDED_RATE) },
    ]);
  }

  return (
    <ReportShell
      title="Project Breakdown"
      description="Time and estimated cost distribution across active projects."
      actions={<ExportMenu onExportCSV={doExport} />}
    >
      <FilterBar rangeKey={rangeKey} onRange={onRange} />

      <KpiGrid>
        <Kpi label="Total time" value={formatDuration(totalMinutes)} icon={Clock} tone="#6d5efc" sub={range.label} />
        <Kpi label="Active projects" value={String(rows.length)} icon={FolderKanban} tone="#0ea5e9" />
        <Kpi label="Most active" value={topProject?.title ?? "—"} icon={Users} tone="#ec4899" sub={topProject ? formatDuration(minutesOf(topProject)) : ""} />
        <Kpi label="Est. labor cost" value={`$${totalCost.toLocaleString()}`} icon={DollarSign} tone="#22c55e" sub={`@ $${BLENDED_RATE}/hr blended`} />
      </KpiGrid>

      <Card>
        <CardHeader><CardTitle>Time by project</CardTitle><span className="text-xs text-muted-foreground">{range.label}</span></CardHeader>
        <CardContent>
          <ResponsiveContainer width="100%" height={260}>
            <BarChart data={chartData} margin={{ top: 8, right: 8, left: -18, bottom: 0 }}>
              <XAxis dataKey="name" tickLine={false} axisLine={false} tick={{ fill: "#94a3b8", fontSize: 11 }} />
              <YAxis tickLine={false} axisLine={false} tick={{ fill: "#94a3b8", fontSize: 12 }} />
              <Tooltip contentStyle={tooltipStyle} cursor={{ fill: "var(--muted)" }} formatter={(v) => formatDuration(Number(v))} />
              <Bar dataKey="minutes" radius={[6, 6, 0, 0]} barSize={44}>
                {chartData.map((d) => <Cell key={d.name} fill={d.color} />)}
              </Bar>
            </BarChart>
          </ResponsiveContainer>
        </CardContent>
      </Card>

      <Card>
        <CardHeader><CardTitle>Details</CardTitle></CardHeader>
        <CardContent>
          <DataTable columns={columns} rows={rows} initialSort={{ key: "time", dir: "desc" }} />
        </CardContent>
      </Card>
    </ReportShell>
  );
}
