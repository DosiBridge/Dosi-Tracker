"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Bar, BarChart, Cell, Pie, PieChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { AppWindow, Clock, TrendingUp, TrendingDown } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Progress } from "@/components/ui/progress";
import { Select } from "@/components/ui/input";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid } from "./report-shell";
import { activities, appCatalog, projects } from "@/lib/tenant-data";
import { categoryColor, filterActivitiesByRange, rangeForKey, type AppCategory, type AppUsage, type RangeKey } from "@/lib/reports-data";

const APP_PALETTE: { app: string; category: AppCategory; color: string }[] = [
  { app: "Visual Studio Code", category: "productive", color: "#3b82f6" },
  { app: "Terminal", category: "productive", color: "#22c55e" },
  { app: "Figma", category: "productive", color: "#ec4899" },
  { app: "Postman", category: "productive", color: "#f97316" },
  { app: "Linear", category: "productive", color: "#8b7dff" },
  { app: "Docker Desktop", category: "productive", color: "#0ea5e9" },
  { app: "Google Chrome", category: "neutral", color: "#64748b" },
  { app: "Slack", category: "neutral", color: "#a855f7" },
  { app: "Notion", category: "neutral", color: "#94a3b8" },
  { app: "Zoom", category: "neutral", color: "#38bdf8" },
  { app: "YouTube", category: "unproductive", color: "#ef4444" },
  { app: "Spotify", category: "unproductive", color: "#f43f5e" },
  { app: "X (Twitter)", category: "unproductive", color: "#fb7185" },
  { app: "X", category: "unproductive", color: "#fb7185" },
];
import { exportRecords } from "@/lib/export";
import { formatDuration, colorFromString } from "@/lib/utils";
import { getApi } from "@/hooks/useApi";
import { useIsMdUp } from "@/hooks/use-media-query";
import type { ResolvedRange } from "./date-range-picker";

/* ---- Live (API) shape: /api/app/reporting/app-usage ---- */
interface LiveAppUsage {
  appName: string;
  trackedMinutes: number;
  activityCount: number;
  userCount: number;
}

/** Live queries pivot on the real clock; mirrors rangeForKey's per-preset logic. */
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
    default: // "7d" and "custom" fall back to the last 7 days
      from.setDate(from.getDate() - 7);
      break;
  }
  return { from, to };
}

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
  const md = useIsMdUp();
  const [rangeKey, setRangeKey] = useState<RangeKey>("7d");
  const [range, setRange] = useState<ResolvedRange>(() => ({ key: "7d", ...rangeForKey("7d") }));
  const [category, setCategory] = useState<AppCategory | "all">("all");

  const [liveUsage, setLiveUsage] = useState<LiveAppUsage[] | null>(null);
  const [liveLoading, setLiveLoading] = useState(false);
  const [liveError, setLiveError] = useState(false);

  const loadLive = useCallback(async () => {
    setLiveLoading(true);
    setLiveError(false);
    try {
      const { from, to } = liveRangeFor(range.key);
      const qs = new URLSearchParams({ From: from.toISOString(), To: to.toISOString() });
      const usage = await getApi(`/api/app/reporting/app-usage?${qs.toString()}`);
      setLiveUsage(Array.isArray(usage) ? (usage as LiveAppUsage[]) : []);
    } catch {
      setLiveUsage(null); // fall back to the demo dataset below
      setLiveError(true);
    } finally {
      setLiveLoading(false);
    }
  }, [range]);

  useEffect(() => {
    if (typeof window === "undefined" || !localStorage.getItem("dosi-token")) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    loadLive();
  }, [loadLive]);

  const mockAllApps = useMemo<AppUsage[]>(() => {
    const acts = filterActivitiesByRange(activities, range.from, range.to);
    const appMinutes = new Map<string, { app: string; category: AppCategory; minutes: number; color: string; users: Set<string> }>();
    const appMetaMap = new Map(APP_PALETTE.map((a) => [a.app, { category: a.category, color: a.color }]));

    acts.forEach((a) => {
      const appName = a.screen.app;
      const meta = appMetaMap.get(appName) || { category: "neutral" as AppCategory, color: "#64748b" };
      const p = projects.find((pr) => pr.id === a.projectId);
      const mins = p?.intervalMinutes ?? 10;
      
      const cur = appMinutes.get(appName) ?? {
        app: appName,
        category: meta.category,
        minutes: 0,
        color: meta.color,
        users: new Set<string>(),
      };
      cur.minutes += mins;
      cur.users.add(a.userId);
      appMinutes.set(appName, cur);
    });

    return [...appMinutes.values()]
      .map((x) => ({
        app: x.app,
        category: x.category,
        minutes: x.minutes,
        color: x.color,
        activeUsers: x.users.size,
      }));
  }, [range]);

  const liveAllApps = useMemo<AppUsage[] | null>(() => {
    if (!liveUsage) return null;
    const meta = new Map(APP_PALETTE.map((a) => [a.app, { category: a.category, color: a.color }]));
    // The backend tracks usage, not classification, and reports process names (code.exe). Category is
    // a client heuristic: known apps keep their palette tone, everything else reads as neutral.
    return liveUsage.map((u) => {
      const m = meta.get(u.appName);
      return {
        app: u.appName,
        category: m?.category ?? ("neutral" as AppCategory),
        minutes: u.trackedMinutes,
        color: m?.color ?? colorFromString(u.appName),
        activeUsers: u.userCount,
      };
    });
  }, [liveUsage]);

  const allApps = liveAllApps ?? mockAllApps;
  const showLiveLoading = liveLoading && liveAllApps === null;
  const showLiveError = liveError && liveAllApps === null;

  const rows = useMemo<AppUsage[]>(() => {
    return allApps
      .filter((a) => category === "all" || a.category === category)
      .sort((a, b) => b.minutes - a.minutes);
  }, [allApps, category]);

  const total = useMemo(() => allApps.reduce((s, a) => s + a.minutes, 0), [allApps]);
  const productive = useMemo(() => allApps.filter((a) => a.category === "productive").reduce((s, a) => s + a.minutes, 0), [allApps]);
  const unproductive = useMemo(() => allApps.filter((a) => a.category === "unproductive").reduce((s, a) => s + a.minutes, 0), [allApps]);
  const donutData = useMemo(() => {
    const neutral = allApps.filter((a) => a.category === "neutral").reduce((s, a) => s + a.minutes, 0);
    return [
      { name: "Productive", value: productive, color: categoryColor.productive },
      { name: "Neutral", value: neutral, color: categoryColor.neutral },
      { name: "Unproductive", value: unproductive, color: categoryColor.unproductive },
    ];
  }, [allApps, productive, unproductive]);

  const chartData = rows.slice(0, 8).map((a) => ({ app: a.app, minutes: a.minutes, color: a.color }));
  const topApp = rows[0];

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
          <Select value={category} onChange={(e) => setCategory(e.target.value as AppCategory | "all")} className="w-full min-w-0 sm:w-auto sm:min-w-40">
            <option value="all">All categories</option>
            <option value="productive">Productive</option>
            <option value="neutral">Neutral</option>
            <option value="unproductive">Unproductive</option>
          </Select>
        }
      />

      {showLiveLoading && (
        <div className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">
          Loading live report data…
        </div>
      )}
      {showLiveError && (
        <div className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">
          Live data unavailable right now — showing demo data.
        </div>
      )}

      <KpiGrid>
        <Kpi label="Total app time" value={formatDuration(total)} icon={Clock} tone="#6d5efc" sub={range.label} />
        <Kpi label="Most used" value={topApp?.app ?? "—"} icon={AppWindow} tone="#0ea5e9" sub={topApp ? formatDuration(topApp.minutes) : ""} />
        <Kpi label="Productive" value={formatDuration(productive)} icon={TrendingUp} tone="#22c55e" />
        <Kpi label="Unproductive" value={formatDuration(unproductive)} icon={TrendingDown} tone="#ef4444" />
      </KpiGrid>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <Card>
          <CardHeader><CardTitle>Category share</CardTitle></CardHeader>
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
                  <span className="font-medium">{total ? Math.round((d.value / total) * 100) : 0}%</span>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>

        <Card className="lg:col-span-2">
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
            <div className="h-[220px] w-full min-w-0 sm:h-[260px]">
              <ResponsiveContainer width="100%" height="100%">
                <BarChart data={chartData} layout="vertical" margin={{ top: 0, right: 8, left: 4, bottom: 0 }}>
                  <XAxis type="number" hide />
                  <YAxis
                    type="category"
                    dataKey="app"
                    tickLine={false}
                    axisLine={false}
                    width={md ? 120 : 72}
                    tick={{ fill: "#94a3b8", fontSize: md ? 12 : 10 }}
                    tickFormatter={(v) => {
                      const s = String(v);
                      const max = md ? 16 : 10;
                      return s.length > max ? `${s.slice(0, max - 1)}…` : s;
                    }}
                  />
                  <Tooltip contentStyle={tooltipStyle} cursor={{ fill: "var(--muted)" }} formatter={(v) => formatDuration(Number(v))} />
                  <Bar dataKey="minutes" radius={[0, 6, 6, 0]} barSize={md ? 18 : 12}>
                    {chartData.map((d) => <Cell key={d.app} fill={d.color} />)}
                  </Bar>
                </BarChart>
              </ResponsiveContainer>
            </div>
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader><CardTitle>All applications</CardTitle><span className="text-xs text-muted-foreground">{rows.length} apps</span></CardHeader>
        <CardContent>
          <DataTable columns={columns} rows={rows} initialSort={{ key: "minutes", dir: "desc" }} />
        </CardContent>
      </Card>
    </ReportShell>
  );
}
