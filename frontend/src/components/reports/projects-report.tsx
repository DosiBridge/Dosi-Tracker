"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Bar, BarChart, Cell, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { FolderKanban, Clock, Users, DollarSign } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid } from "./report-shell";
import { activities, projects, userById } from "@/lib/tenant-data";
import { filterActivitiesByRange, rangeForKey, type RangeKey } from "@/lib/reports-data";
import { exportRecords } from "@/lib/export";
import { formatDuration } from "@/lib/utils";
import { getApi } from "@/hooks/useApi";
import type { Project } from "@/lib/types";
import type { ResolvedRange } from "./date-range-picker";

const BLENDED_RATE = 48; // $/hr for cost estimate

/* ---- Live (API) shapes ---- */

interface LivePerProject {
  projectId: string;
  trackedMinutes: number;
}

interface LiveSummary {
  perProject: LivePerProject[];
}

interface ApiProject {
  id: string;
  title: string;
  color?: string | null;
  intervalMinutes?: number;
  isArchived?: boolean;
}

/** Map a real backend project onto the demo Project view-model the report markup expects.
 *  Members and all-time totals aren't carried by these endpoints, so they degrade to empty in
 *  live mode (loggedTotal reflects the queried window). */
function toProjectView(p: ApiProject, trackedMinutes: number): Project {
  return {
    id: p.id,
    title: p.title,
    description: "",
    color: p.color ?? "#0d9488",
    archived: !!p.isArchived,
    intervalMinutes: p.intervalMinutes ?? 10,
    permissions: {
      screenshot: false,
      webcam: false,
      keyboard: false,
      mouse: false,
      activeWindow: false,
      runningPrograms: false,
    },
    memberIds: [],
    createdAt: "",
    loggedThisWeek: 0,
    loggedThisMonth: 0,
    loggedTotal: trackedMinutes,
  };
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
    case "7d":
      from.setDate(from.getDate() - 7);
      break;
    case "month":
      from.setDate(1);
      from.setHours(0, 0, 0, 0);
      break;
    default: // "30d" and "custom" fall back to the last 30 days
      from.setDate(from.getDate() - 30);
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

export function ProjectsReport() {
  const [rangeKey, setRangeKey] = useState<RangeKey>("30d");
  const [range, setRange] = useState<ResolvedRange>(() => ({ key: "30d", ...rangeForKey("30d") }));

  const [liveSummary, setLiveSummary] = useState<LiveSummary | null>(null);
  const [liveProjects, setLiveProjects] = useState<ApiProject[] | null>(null);
  const [liveLoading, setLiveLoading] = useState(false);
  const [liveError, setLiveError] = useState(false);

  const loadLive = useCallback(async () => {
    setLiveLoading(true);
    setLiveError(false);
    try {
      const { from, to } = liveRangeFor(range.key);
      const qs = new URLSearchParams({ From: from.toISOString(), To: to.toISOString() });
      const [summary, projectList] = await Promise.all([
        getApi(`/api/app/reporting/summary?${qs.toString()}`),
        getApi("/api/app/project"),
      ]);
      setLiveSummary(summary && typeof summary === "object" ? (summary as LiveSummary) : null);
      setLiveProjects(Array.isArray(projectList) ? (projectList as ApiProject[]) : []);
    } catch {
      setLiveSummary(null); // fall back to the demo dataset below
      setLiveProjects(null);
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

  const mockRows = useMemo(() => projects.filter((p) => !p.archived), []);

  const mockMinutes = useMemo(() => {
    const acts = filterActivitiesByRange(activities, range.from, range.to);
    const map = new Map<string, number>();
    acts.forEach((a) => {
      const p = projects.find((pr) => pr.id === a.projectId);
      const mins = p?.intervalMinutes ?? 10;
      map.set(a.projectId, (map.get(a.projectId) ?? 0) + mins);
    });
    return map;
  }, [range]);

  const liveMinutes = useMemo<Map<string, number> | null>(() => {
    if (!liveSummary) return null;
    const map = new Map<string, number>();
    (Array.isArray(liveSummary.perProject) ? liveSummary.perProject : []).forEach((p) => {
      map.set(p.projectId, Math.round(p.trackedMinutes));
    });
    return map;
  }, [liveSummary]);

  const liveRows = useMemo<Project[] | null>(() => {
    if (!liveProjects || !liveMinutes) return null;
    return liveProjects
      .filter((p) => !p.isArchived)
      .map((p) => toProjectView(p, liveMinutes.get(p.id) ?? 0));
  }, [liveProjects, liveMinutes]);

  const rows = liveRows ?? mockRows;
  const projectMinutes = liveMinutes ?? mockMinutes;
  const showLiveLoading = liveLoading && liveRows === null;
  const showLiveError = liveError && liveRows === null;

  const minutesOf = (p: Project) => projectMinutes.get(p.id) ?? 0;

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
    { key: "time", header: range.label, align: "right", sortValue: (r) => minutesOf(r), render: (r) => <span className="font-medium">{formatDuration(minutesOf(r))}</span> },
    { key: "total", header: "Total logged", align: "right", sortValue: (r) => r.loggedTotal, render: (r) => formatDuration(r.loggedTotal) },
    { key: "share", header: "Share", align: "right", sortValue: (r) => minutesOf(r), render: (r) => <Badge tone="muted">{totalMinutes ? Math.round((minutesOf(r) / totalMinutes) * 100) : 0}%</Badge> },
    { key: "cost", header: "Est. cost", align: "right", sortValue: (r) => minutesOf(r), render: (r) => <span className="font-medium">${Math.round((minutesOf(r) / 60) * BLENDED_RATE).toLocaleString()}</span> },
  ];

  function onRange(r: ResolvedRange) { setRange(r); setRangeKey(r.key); }
  function doExport() {
    exportRecords(`projects-${range.key}`, rows, [
      { header: "Project", value: (r) => r.title },
      { header: "Members", value: (r) => r.memberIds.length },
      { header: `${range.label} (min)`, value: (r) => minutesOf(r) },
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
        <Kpi label="Total time" value={formatDuration(totalMinutes)} icon={Clock} tone="#6d5efc" sub={range.label} />
        <Kpi label="Active projects" value={String(rows.length)} icon={FolderKanban} tone="#0ea5e9" />
        <Kpi label="Most active" value={topProject?.title ?? "—"} icon={Users} tone="#ec4899" sub={topProject ? formatDuration(minutesOf(topProject)) : ""} />
        <Kpi label="Est. labor cost" value={`$${totalCost.toLocaleString()}`} icon={DollarSign} tone="#22c55e" sub={`@ $${BLENDED_RATE}/hr blended`} />
      </KpiGrid>

      <Card>
        <CardHeader><CardTitle>Time by project</CardTitle><span className="text-xs text-muted-foreground">{range.label}</span></CardHeader>
        <CardContent>
          <div className="h-[200px] w-full min-w-0 sm:h-[260px]">
            <ResponsiveContainer width="100%" height="100%">
              <BarChart data={chartData} margin={{ top: 8, right: 8, left: -8, bottom: 0 }}>
                <XAxis dataKey="name" tickLine={false} axisLine={false} tick={{ fill: "#94a3b8", fontSize: 10 }} interval={0} />
                <YAxis tickLine={false} axisLine={false} tick={{ fill: "#94a3b8", fontSize: 11 }} width={36} />
                <Tooltip contentStyle={tooltipStyle} cursor={{ fill: "var(--muted)" }} formatter={(v) => formatDuration(Number(v))} />
                <Bar dataKey="minutes" radius={[6, 6, 0, 0]} barSize={36}>
                  {chartData.map((d) => <Cell key={d.name} fill={d.color} />)}
                </Bar>
              </BarChart>
            </ResponsiveContainer>
          </div>
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
