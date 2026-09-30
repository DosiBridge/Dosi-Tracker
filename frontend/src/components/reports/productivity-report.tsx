"use client";

import { useCallback, useMemo, useState } from "react";
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
import { Clock, Gauge, TrendingUp, Minus, TrendingDown } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid, LiveReportNotice, useLiveReport } from "./report-shell";
import { activities, userById, users } from "@/lib/tenant-data";
import { durationMinutes } from "@/lib/metrics";
import { categoryColor, filterActivitiesByRange, type RangeKey } from "@/lib/reports-data";
import { activitySplit, asArray, memberName, reportQuery, safePercent } from "@/lib/report-math";
import { exportRecords } from "@/lib/export";
import { cn, formatDuration } from "@/lib/utils";
import { getApi } from "@/hooks/useApi";
import { useSession } from "@/components/session-provider";
import { resolveRange, type ResolvedRange } from "./date-range-picker";

/* ---- Live (API) shape: /api/app/reporting/summary ---- */

interface LivePerUser {
  userId: string;
  activityCount: number;
  trackedMinutes: number;
  averageProductivity: number;
}

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
  const { isLive } = useSession();
  const [rangeKey, setRangeKey] = useState<RangeKey>("7d");
  const [range, setRange] = useState<ResolvedRange>(() => resolveRange("7d"));
  const [memberId, setMemberId] = useState("all");
  const [viewMode, setViewMode] = useState<"table" | "grid">("table");

  const loadLive = useCallback(async (): Promise<LivePerUser[]> => {
    const summary = (await getApi(
      `/api/app/reporting/summary?${reportQuery({ from: range.from, to: range.to, userId: memberId })}`,
    )) as { perUser?: LivePerUser[] } | null;
    return asArray<LivePerUser>(summary?.perUser);
  }, [range, memberId]);

  const live = useLiveReport(isLive, loadLive);

  const mockRows = useMemo<Row[]>(() => {
    const acts = filterActivitiesByRange(activities, range.from, range.to);
    const members = users.filter((u) => u.role !== "client" && (memberId === "all" || u.id === memberId));
    return members.map((u) => {
      const ua = acts.filter((a) => a.userId === u.id);
      let productive = 0;
      let neutral = 0;
      let unproductive = 0;
      let idle = 0;

      ua.forEach((a) => {
        const mins = durationMinutes(a);
        const prod = Math.round((mins * a.productivity) / 100);
        const rem = mins - prod;
        const unprod = Math.round(rem * 0.35);
        const neut = rem - unprod;

        productive += prod;
        neutral += neut;
        unproductive += unprod;
        idle += Math.round(mins * 0.08);
      });

      const total = productive + neutral + unproductive;
      const focus = total ? Math.round((productive / total) * 100) : 0;
      return {
        userId: u.id,
        name: u.name,
        productive,
        neutral,
        unproductive,
        idle,
        focus,
      };
    });
  }, [range, memberId]);

  /*
   * LIVE: the backend reports each member's tracked minutes and their
   * duration-weighted activity score — nothing classifies time as neutral or
   * unproductive and nothing measures idle time. So a live row carries the
   * active ("productive") share and the rest as "other tracked"; the demo-only
   * unproductive/idle split is never invented for a real person.
   */
  const liveRows = useMemo<Row[]>(
    () =>
      (live.data ?? [])
        .filter((u) => memberId === "all" || u.userId === memberId)
        .map((u) => {
          const split = activitySplit(u.trackedMinutes, u.averageProductivity);
          return {
            userId: u.userId,
            name: memberName(u.userId, userById(u.userId)),
            productive: split.productive,
            neutral: split.other,
            unproductive: 0,
            idle: 0,
            focus: split.focus,
          };
        }),
    [live.data, memberId],
  );

  const rows = isLive ? liveRows : mockRows;
  const neutralLabel = isLive ? "Other tracked" : "Neutral";

  const totals = useMemo(() => {
    const t = { productive: 0, neutral: 0, unproductive: 0, idle: 0 };
    rows.forEach((r) => {
      t.productive += r.productive;
      t.neutral += r.neutral;
      t.unproductive += r.unproductive;
      t.idle += r.idle;
    });
    return t;
  }, [rows]);

  const grand = totals.productive + totals.neutral + totals.unproductive;
  const focusPct = safePercent(totals.productive, grand);

  const donutData = isLive
    ? [
        { name: "Productive", value: totals.productive, color: categoryColor.productive },
        { name: neutralLabel, value: totals.neutral, color: categoryColor.neutral },
      ]
    : [
        { name: "Productive", value: totals.productive, color: categoryColor.productive },
        { name: "Neutral", value: totals.neutral, color: categoryColor.neutral },
        { name: "Unproductive", value: totals.unproductive, color: categoryColor.unproductive },
      ];

  const barData = rows.map((r) => ({ ...r, name: r.name.split(" ")[0] }));

  const allColumns: Column<Row>[] = [
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
    { key: "neutral", header: neutralLabel, align: "right", sortValue: (r) => r.neutral, render: (r) => <span className="text-info">{formatDuration(r.neutral)}</span> },
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
  // Live data has no unproductive/idle measurement: those columns would only ever read 0.
  const columns = isLive ? allColumns.filter((c) => c.key !== "unproductive" && c.key !== "idle") : allColumns;

  function onRange(r: ResolvedRange) {
    setRange(r);
    setRangeKey(r.key);
  }
  function doExport() {
    if (isLive) {
      exportRecords(`productivity-${range.key}`, rows, [
        { header: "Member", value: (r) => r.name },
        { header: "Productive (min)", value: (r) => r.productive },
        { header: "Other tracked (min)", value: (r) => r.neutral },
        { header: "Focus %", value: (r) => r.focus },
      ]);
      return;
    }
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

      {isLive && <LiveReportNotice loading={live.loading} error={live.error} onRetry={live.retry} />}

      <KpiGrid>
        <Kpi label="Team focus" value={`${focusPct}%`} icon={Gauge} tone="#6d5efc" sub={range.label} />
        <Kpi label="Productive" value={formatDuration(totals.productive)} icon={TrendingUp} tone="#22c55e" />
        <Kpi label={neutralLabel} value={formatDuration(totals.neutral)} icon={Minus} tone="#0ea5e9" />
        {isLive ? (
          <Kpi label="Total tracked" value={formatDuration(grand)} icon={Clock} tone="#ec4899" />
        ) : (
          <Kpi label="Unproductive" value={formatDuration(totals.unproductive)} icon={TrendingDown} tone="#ef4444" />
        )}
      </KpiGrid>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <Card>
          <CardHeader><CardTitle>Time split</CardTitle></CardHeader>
          <CardContent>
            <div className="mx-auto h-[160px] max-w-[240px] sm:h-[200px] sm:max-w-none">
              <ResponsiveContainer width="100%" height="100%">
                <PieChart>
                  <Pie data={donutData} dataKey="value" innerRadius={44} outerRadius={68} paddingAngle={2} strokeWidth={0}>
                    {donutData.map((d) => <Cell key={d.name} fill={d.color} />)}
                  </Pie>
                  <Tooltip contentStyle={tooltipStyle} formatter={(v) => formatDuration(Number(v))} />
                </PieChart>
              </ResponsiveContainer>
            </div>
            <div className="mt-2 space-y-1.5">
              {donutData.map((d) => (
                <div key={d.name} className="flex items-center justify-between text-sm">
                  <span className="flex items-center gap-2">
                    <span className="h-2.5 w-2.5 rounded-full" style={{ background: d.color }} /> {d.name}
                  </span>
                  <span className="font-medium">{safePercent(d.value, grand)}%</span>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>

        <Card className="lg:col-span-2">
          <CardHeader><CardTitle>Per-member breakdown</CardTitle></CardHeader>
          <CardContent>
            <div className="h-[200px] w-full min-w-0 sm:h-[260px]">
              <ResponsiveContainer width="100%" height="100%">
                <BarChart data={barData} margin={{ top: 8, right: 8, left: -8, bottom: 0 }}>
                  <XAxis dataKey="name" tickLine={false} axisLine={false} tick={{ fill: "#94a3b8", fontSize: 11 }} interval={0} angle={0} />
                  <YAxis tickLine={false} axisLine={false} tick={{ fill: "#94a3b8", fontSize: 11 }} width={36} />
                  <Tooltip contentStyle={tooltipStyle} cursor={{ fill: "var(--muted)" }} formatter={(v) => formatDuration(Number(v))} />
                  <Bar dataKey="productive" stackId="a" fill={categoryColor.productive} radius={[0, 0, 0, 0]} barSize={22} />
                  <Bar
                    dataKey="neutral"
                    name={isLive ? "other tracked" : "neutral"}
                    stackId="a"
                    fill={categoryColor.neutral}
                    radius={isLive ? [4, 4, 0, 0] : [0, 0, 0, 0]}
                    barSize={22}
                  />
                  {!isLive && (
                    <Bar dataKey="unproductive" stackId="a" fill={categoryColor.unproductive} radius={[4, 4, 0, 0]} barSize={22} />
                  )}
                </BarChart>
              </ResponsiveContainer>
            </div>
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <div>
            <CardTitle>Details</CardTitle>
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
              initialSort={{ key: "focus", dir: "desc" }}
              emptyText={isLive ? "No tracked time in this range." : undefined}
            />
          ) : (
            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {rows.map((r) => {
                const u = userById(r.userId);
                const sumTotal = r.productive + r.neutral + r.unproductive;
                return (
                  <Card key={r.userId} className="p-4 hover:border-primary/40 transition-colors">
                    <div className="flex items-center gap-3">
                      <Avatar name={r.name} size="md" status={u?.status} />
                      <div className="min-w-0 flex-1">
                        <h4 className="font-semibold truncate text-sm">{r.name}</h4>
                        <p className="text-xs text-muted-foreground truncate">{u?.designation ?? "Team Member"}</p>
                      </div>
                    </div>

                    <div className="mt-4 space-y-2 border-t border-border/60 pt-3">
                      <div className="flex justify-between text-xs">
                        <span className="text-muted-foreground">Focus Score</span>
                        <Badge tone={r.focus >= 70 ? "success" : r.focus >= 50 ? "warning" : "danger"}>{r.focus}%</Badge>
                      </div>

                      <div className="space-y-1">
                        <div className="flex justify-between text-[11px] text-muted-foreground">
                          <span>Productivity Split</span>
                          <span>{formatDuration(r.productive)} / {formatDuration(sumTotal)}</span>
                        </div>
                        <div className="flex h-2 w-full overflow-hidden rounded-full bg-muted">
                          <div className="bg-success transition-all" style={{ width: `${safePercent(r.productive, sumTotal)}%` }} title={`Productive: ${formatDuration(r.productive)}`} />
                          <div className="bg-info transition-all" style={{ width: `${safePercent(r.neutral, sumTotal)}%` }} title={`${neutralLabel}: ${formatDuration(r.neutral)}`} />
                          {!isLive && (
                            <div className="bg-danger transition-all" style={{ width: `${safePercent(r.unproductive, sumTotal)}%` }} title={`Unproductive: ${formatDuration(r.unproductive)}`} />
                          )}
                        </div>
                      </div>
                    </div>

                    {isLive ? (
                      <div className="mt-4 grid grid-cols-2 gap-2 border-t border-border/60 pt-3 text-[11px] text-muted-foreground">
                        <div>
                          <span>Productive:</span>
                          <div className="font-bold text-success text-xs mt-0.5">{formatDuration(r.productive)}</div>
                        </div>
                        <div>
                          <span>Other tracked:</span>
                          <div className="font-bold text-info text-xs mt-0.5">{formatDuration(r.neutral)}</div>
                        </div>
                      </div>
                    ) : (
                      <div className="mt-4 grid grid-cols-2 gap-2 border-t border-border/60 pt-3 text-[11px] text-muted-foreground">
                        <div>
                          <span>Productive:</span>
                          <div className="font-bold text-success text-xs mt-0.5">{formatDuration(r.productive)}</div>
                        </div>
                        <div>
                          <span>Unproductive:</span>
                          <div className="font-bold text-danger text-xs mt-0.5">{formatDuration(r.unproductive)}</div>
                        </div>
                        <div className="mt-1">
                          <span>Neutral:</span>
                          <div className="font-bold text-info text-xs mt-0.5">{formatDuration(r.neutral)}</div>
                        </div>
                        <div className="mt-1">
                          <span>Idle:</span>
                          <div className="font-bold text-foreground text-xs mt-0.5">{formatDuration(r.idle)}</div>
                        </div>
                      </div>
                    )}
                  </Card>
                );
              })}
            </div>
          )}
        </CardContent>
      </Card>

      {isLive && (
        <p className="text-xs text-muted-foreground">
          Productive time is each member&apos;s tracked time weighted by their activity score. Time isn&apos;t classified
          as neutral or unproductive, and idle time isn&apos;t measured, so neither is shown.
        </p>
      )}
    </ReportShell>
  );
}
