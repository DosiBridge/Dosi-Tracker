"use client";

import { useMemo, useState } from "react";
import {
  Bar,
  BarChart,
  Cell,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { Gauge, TrendingUp, Minus, TrendingDown } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid } from "./report-shell";
import { categoryTotals, productivitySplit, userById, users } from "@/lib/tenant-data";
import { categoryColor, rangeForKey, type RangeKey } from "@/lib/reports-data";
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

interface Row {
  userId: string;
  name: string;
  productive: number;
  neutral: number;
  unproductive: number;
  idle: number;
  focus: number;
}

export function ProductivityReport() {
  const [rangeKey, setRangeKey] = useState<RangeKey>("7d");
  const [range, setRange] = useState<ResolvedRange>(() => ({ key: "7d", ...rangeForKey("7d") }));
  const [memberId, setMemberId] = useState("all");

  const rows = useMemo<Row[]>(() => {
    return productivitySplit
      .filter((s) => memberId === "all" || s.userId === memberId)
      .map((s) => {
        const total = s.productive + s.neutral + s.unproductive;
        const focus = total ? Math.round((s.productive / total) * 100) : 0;
        return {
          userId: s.userId,
          name: userById(s.userId)?.name ?? "",
          productive: s.productive,
          neutral: s.neutral,
          unproductive: s.unproductive,
          idle: s.idle,
          focus,
        };
      });
  }, [memberId]);

  const totals = categoryTotals();
  const grand = totals.productive + totals.neutral + totals.unproductive;
  const focusPct = grand ? Math.round((totals.productive / grand) * 100) : 0;

  const donutData = [
    { name: "Productive", value: totals.productive, color: categoryColor.productive },
    { name: "Neutral", value: totals.neutral, color: categoryColor.neutral },
    { name: "Unproductive", value: totals.unproductive, color: categoryColor.unproductive },
  ];

  const barData = rows.map((r) => ({ ...r, name: r.name.split(" ")[0] }));

  const columns: Column<Row>[] = [
    {
      key: "name",
      header: "Member",
      sortValue: (r) => r.name,
      render: (r) => (
        <div className="flex items-center gap-2.5">
          <Avatar name={r.name} size="sm" status={userById(r.userId)?.status} />
          <span className="font-medium">{r.name}</span>
        </div>
      ),
    },
    { key: "productive", header: "Productive", align: "right", sortValue: (r) => r.productive, render: (r) => <span className="text-success">{formatDuration(r.productive)}</span> },
    { key: "neutral", header: "Neutral", align: "right", sortValue: (r) => r.neutral, render: (r) => <span className="text-info">{formatDuration(r.neutral)}</span> },
    { key: "unproductive", header: "Unproductive", align: "right", sortValue: (r) => r.unproductive, render: (r) => <span className="text-danger">{formatDuration(r.unproductive)}</span> },
    { key: "idle", header: "Idle", align: "right", sortValue: (r) => r.idle, render: (r) => <span className="text-muted-foreground">{formatDuration(r.idle)}</span> },
    {
      key: "focus",
      header: "Focus",
      align: "right",
      sortValue: (r) => r.focus,
      render: (r) => <Badge tone={r.focus >= 70 ? "success" : r.focus >= 50 ? "warning" : "danger"}>{r.focus}%</Badge>,
    },
  ];

  function onRange(r: ResolvedRange) {
    setRange(r);
    setRangeKey(r.key);
  }
  function doExport() {
    exportRecords(`productivity-${range.key}`, rows, [
      { header: "Member", value: (r) => r.name },
      { header: "Productive (min)", value: (r) => r.productive },
      { header: "Neutral (min)", value: (r) => r.neutral },
      { header: "Unproductive (min)", value: (r) => r.unproductive },
      { header: "Idle (min)", value: (r) => r.idle },
      { header: "Focus %", value: (r) => r.focus },
    ]);
  }

  return (
    <ReportShell
      title="Productivity"
      description="How time splits across productive, neutral, and unproductive apps."
      actions={<ExportMenu onExportCSV={doExport} />}
    >
      <FilterBar rangeKey={rangeKey} onRange={onRange} memberId={memberId} onMember={setMemberId} />

      <KpiGrid>
        <Kpi label="Team focus" value={`${focusPct}%`} icon={Gauge} tone="#6d5efc" sub={range.label} />
        <Kpi label="Productive" value={formatDuration(totals.productive)} icon={TrendingUp} tone="#22c55e" />
        <Kpi label="Neutral" value={formatDuration(totals.neutral)} icon={Minus} tone="#0ea5e9" />
        <Kpi label="Unproductive" value={formatDuration(totals.unproductive)} icon={TrendingDown} tone="#ef4444" />
      </KpiGrid>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <Card>
          <CardHeader><CardTitle>Time split</CardTitle></CardHeader>
          <CardContent>
            <ResponsiveContainer width="100%" height={200}>
              <PieChart>
                <Pie data={donutData} dataKey="value" innerRadius={58} outerRadius={86} paddingAngle={2} strokeWidth={0}>
                  {donutData.map((d) => <Cell key={d.name} fill={d.color} />)}
                </Pie>
                <Tooltip contentStyle={tooltipStyle} formatter={(v) => formatDuration(Number(v))} />
              </PieChart>
            </ResponsiveContainer>
            <div className="mt-2 space-y-1.5">
              {donutData.map((d) => (
                <div key={d.name} className="flex items-center justify-between text-sm">
                  <span className="flex items-center gap-2">
                    <span className="h-2.5 w-2.5 rounded-full" style={{ background: d.color }} /> {d.name}
                  </span>
                  <span className="font-medium">{Math.round((d.value / grand) * 100)}%</span>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>

        <Card className="lg:col-span-2">
          <CardHeader><CardTitle>Per-member breakdown</CardTitle></CardHeader>
          <CardContent>
            <ResponsiveContainer width="100%" height={260}>
              <BarChart data={barData} margin={{ top: 8, right: 8, left: -18, bottom: 0 }}>
                <XAxis dataKey="name" tickLine={false} axisLine={false} tick={{ fill: "#94a3b8", fontSize: 12 }} />
                <YAxis tickLine={false} axisLine={false} tick={{ fill: "#94a3b8", fontSize: 12 }} />
                <Tooltip contentStyle={tooltipStyle} cursor={{ fill: "var(--muted)" }} formatter={(v) => formatDuration(Number(v))} />
                <Bar dataKey="productive" stackId="a" fill={categoryColor.productive} radius={[0, 0, 0, 0]} barSize={26} />
                <Bar dataKey="neutral" stackId="a" fill={categoryColor.neutral} barSize={26} />
                <Bar dataKey="unproductive" stackId="a" fill={categoryColor.unproductive} radius={[4, 4, 0, 0]} barSize={26} />
              </BarChart>
            </ResponsiveContainer>
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader><CardTitle>Details</CardTitle><span className="text-xs text-muted-foreground">{rows.length} members</span></CardHeader>
        <CardContent>
          <DataTable columns={columns} rows={rows} initialSort={{ key: "focus", dir: "desc" }} />
        </CardContent>
      </Card>
    </ReportShell>
  );
}
