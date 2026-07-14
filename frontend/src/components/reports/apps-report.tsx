"use client";

import { useMemo, useState } from "react";
import { Bar, BarChart, Cell, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { AppWindow, Clock, TrendingUp, TrendingDown } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Progress } from "@/components/ui/progress";
import { Select } from "@/components/ui/input";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid } from "./report-shell";
import { appCatalog } from "@/lib/tenant-data";
import { categoryColor, rangeForKey, type AppCategory, type AppUsage, type RangeKey } from "@/lib/reports-data";
import { exportRecords } from "@/lib/export";
import { formatDuration } from "@/lib/utils";
import type { ResolvedRange } from "./date-range-picker";

const tooltipStyle = {
  borderRadius: 12,
  border: "1px solid var(--border)",
  background: "var(--card)",
  fontSize: 12,
  boxShadow: "var(--elev-lg)",
} as const;

const catTone: Record<AppCategory, "success" | "info" | "danger"> = {
  productive: "success",
  neutral: "info",
  unproductive: "danger",
};

export function AppsReport() {
  const [rangeKey, setRangeKey] = useState<RangeKey>("7d");
  const [range, setRange] = useState<ResolvedRange>(() => ({ key: "7d", ...rangeForKey("7d") }));
  const [category, setCategory] = useState<AppCategory | "all">("all");

  const rows = useMemo<AppUsage[]>(
    () =>
      [...appCatalog]
        .filter((a) => category === "all" || a.category === category)
        .sort((a, b) => b.minutes - a.minutes),
    [category]
  );

  const total = rows.reduce((s, a) => s + a.minutes, 0);
  const productive = appCatalog.filter((a) => a.category === "productive").reduce((s, a) => s + a.minutes, 0);
  const unproductive = appCatalog.filter((a) => a.category === "unproductive").reduce((s, a) => s + a.minutes, 0);
  const topApp = rows[0];

  const chartData = rows.slice(0, 8).map((a) => ({ app: a.app, minutes: a.minutes, color: a.color }));

  const columns: Column<AppUsage>[] = [
    { key: "app", header: "Application", sortValue: (r) => r.app, render: (r) => (
      <span className="flex items-center gap-2 font-medium">
        <span className="h-2.5 w-2.5 rounded-full" style={{ background: r.color }} /> {r.app}
      </span>
    )},
    { key: "category", header: "Category", sortValue: (r) => r.category, render: (r) => <Badge tone={catTone[r.category]} className="capitalize">{r.category}</Badge> },
    { key: "minutes", header: "Time", align: "right", sortValue: (r) => r.minutes, render: (r) => <span className="font-medium">{formatDuration(r.minutes)}</span> },
    { key: "share", header: "Share", align: "right", sortValue: (r) => r.minutes, render: (r) => (
      <div className="ml-auto flex w-28 items-center gap-2">
        <Progress value={(r.minutes / (topApp?.minutes || 1)) * 100} color={r.color} />
        <span className="w-9 text-right text-xs">{Math.round((r.minutes / total) * 100)}%</span>
      </div>
    )},
    { key: "activeUsers", header: "Users", align: "right", sortValue: (r) => r.activeUsers, render: (r) => r.activeUsers },
  ];

  function onRange(r: ResolvedRange) { setRange(r); setRangeKey(r.key); }
  function doExport() {
    exportRecords(`apps-usage-${range.key}`, rows, [
      { header: "Application", value: (r) => r.app },
      { header: "Category", value: (r) => r.category },
      { header: "Time (min)", value: (r) => r.minutes },
      { header: "Share %", value: (r) => Math.round((r.minutes / total) * 100) },
      { header: "Active users", value: (r) => r.activeUsers },
    ]);
  }

  return (
    <ReportShell
      title="Apps & Websites"
      description="Most-used applications and websites, categorized by productivity."
      actions={<ExportMenu onExportCSV={doExport} />}
    >
      <FilterBar
        rangeKey={rangeKey}
        onRange={onRange}
        extra={
          <Select value={category} onChange={(e) => setCategory(e.target.value as AppCategory | "all")} className="w-auto min-w-40">
            <option value="all">All categories</option>
            <option value="productive">Productive</option>
            <option value="neutral">Neutral</option>
            <option value="unproductive">Unproductive</option>
          </Select>
        }
      />

      <KpiGrid>
        <Kpi label="Total app time" value={formatDuration(total)} icon={Clock} tone="#6d5efc" sub={range.label} />
        <Kpi label="Most used" value={topApp?.app ?? "—"} icon={AppWindow} tone="#0ea5e9" sub={topApp ? formatDuration(topApp.minutes) : ""} />
        <Kpi label="Productive" value={formatDuration(productive)} icon={TrendingUp} tone="#22c55e" />
        <Kpi label="Unproductive" value={formatDuration(unproductive)} icon={TrendingDown} tone="#ef4444" />
      </KpiGrid>

      <Card>
        <CardHeader>
          <CardTitle>Top applications</CardTitle>
          <div className="flex items-center gap-3 text-xs text-muted-foreground">
            {(["productive", "neutral", "unproductive"] as AppCategory[]).map((c) => (
              <span key={c} className="flex items-center gap-1.5 capitalize">
                <span className="h-2 w-2 rounded-full" style={{ background: categoryColor[c] }} /> {c}
              </span>
            ))}
          </div>
        </CardHeader>
        <CardContent>
          <ResponsiveContainer width="100%" height={280}>
            <BarChart data={chartData} layout="vertical" margin={{ top: 0, right: 16, left: 20, bottom: 0 }}>
              <XAxis type="number" hide />
              <YAxis type="category" dataKey="app" tickLine={false} axisLine={false} width={130} tick={{ fill: "#94a3b8", fontSize: 12 }} />
              <Tooltip contentStyle={tooltipStyle} cursor={{ fill: "var(--muted)" }} formatter={(v) => formatDuration(Number(v))} />
              <Bar dataKey="minutes" radius={[0, 6, 6, 0]} barSize={18}>
                {chartData.map((d) => <Cell key={d.app} fill={d.color} />)}
              </Bar>
            </BarChart>
          </ResponsiveContainer>
        </CardContent>
      </Card>

      <Card>
        <CardHeader><CardTitle>All applications</CardTitle><span className="text-xs text-muted-foreground">{rows.length} apps</span></CardHeader>
        <CardContent>
          <DataTable columns={columns} rows={rows} initialSort={{ key: "minutes", dir: "desc" }} />
        </CardContent>
      </Card>
    </ReportShell>
  );
}
