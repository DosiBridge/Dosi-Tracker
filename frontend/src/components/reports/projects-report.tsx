"use client";

import { useCallback, useMemo, useState } from "react";
import { Bar, BarChart, Cell, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { FolderKanban, Clock, Users, DollarSign } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid, LiveReportNotice, useLiveReport } from "./report-shell";
import { activities, projectById, projects, userById } from "@/lib/tenant-data";
import { minutesByProject } from "@/lib/metrics";
import { filterActivitiesByRange, type RangeKey } from "@/lib/reports-data";
import {
  asArray,
  laborCost,
  ratesByMember,
  reportQuery,
  safePercent,
  type MemberRate,
  type MinutesRow,
} from "@/lib/report-math";
import { exportRecords } from "@/lib/export";
import { formatDuration } from "@/lib/utils";
import { getApi } from "@/hooks/useApi";
import { useSession } from "@/components/session-provider";
import type { Project } from "@/lib/types";
import { resolveRange, type ResolvedRange } from "./date-range-picker";

/** DEMO ONLY: the seeded dataset has no per-project rates, so its cost is a flat illustrative rate. */
const DEMO_BLENDED_RATE = 48; // $/hr

/* ---- Live (API) shapes ---- */

interface LiveSummary {
  perProject?: { projectId: string; trackedMinutes: number }[];
  perUser?: MinutesRow[];
}

interface ApiProject {
  id: string;
  title: string;
  color?: string | null;
  intervalMinutes?: number;
  isArchived?: boolean;
}

interface LiveProjectsReport {
  projects: ApiProject[];
  /** projectId → minutes tracked in the range. */
  minutes: Map<string, number>;
  /** projectId → labour cost from real rates; null when the rates couldn't be read. */
  costs: Map<string, number> | null;
}

/** Map a real backend project onto the Project view-model the report markup expects.
 *  Team avatars come from the session roster; all-time totals aren't carried by these
 *  endpoints, so that column is not shown in live mode. */
function toProjectView(p: ApiProject): Project {
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
    memberIds: projectById(p.id)?.memberIds ?? [],
    createdAt: "",
    loggedThisWeek: 0,
    loggedThisMonth: 0,
    loggedTotal: 0,
  };
}

const tooltipStyle = {
  borderRadius: 12,
  border: "1px solid var(--border)",
  background: "var(--card)",
  fontSize: 12,
  boxShadow: "var(--elev-lg)",
} as const;

export function ProjectsReport() {
  const { isLive } = useSession();
  const [rangeKey, setRangeKey] = useState<RangeKey>("30d");
  const [range, setRange] = useState<ResolvedRange>(() => resolveRange("30d"));

  const loadLive = useCallback(async (): Promise<LiveProjectsReport> => {
    const span = { from: range.from, to: range.to };
    const [summary, projectList] = await Promise.all([
      getApi(`/api/app/reporting/summary?${reportQuery(span)}`) as Promise<LiveSummary | null>,
      getApi("/api/app/project?MaxResultCount=1000"),
    ]);
    const minutes = new Map<string, number>();
    for (const p of asArray<{ projectId: string; trackedMinutes: number }>(summary?.perProject)) {
      minutes.set(p.projectId, Math.round(Number(p.trackedMinutes) || 0));
    }

    // Labour cost only from REAL rates: per tracked project, its memberships'
    // hourly rates × each member's minutes on it. Rates are secondary to the
    // hours, so a failure here hides the cost instead of failing the report.
    let costs: Map<string, number> | null;
    try {
      const entries = await Promise.all(
        [...minutes].filter(([, m]) => m > 0).map(async ([projectId]) => {
          const [members, projectSummary] = await Promise.all([
            getApi(`/api/app/team/project-members/${encodeURIComponent(projectId)}`),
            getApi(`/api/app/reporting/summary?${reportQuery({ ...span, projectId })}`) as Promise<LiveSummary | null>,
          ]);
          const { cost } = laborCost(asArray<MinutesRow>(projectSummary?.perUser), ratesByMember(asArray<MemberRate>(members)));
          return [projectId, cost] as const;
        }),
      );
      costs = new Map(entries);
    } catch {
      costs = null;
    }
    return { projects: asArray<ApiProject>(projectList), minutes, costs };
  }, [range]);

  const live = useLiveReport(isLive, loadLive);

  const mockRows = useMemo(() => projects.filter((p) => !p.archived), []);

  const mockMinutes = useMemo(
    () => minutesByProject(filterActivitiesByRange(activities, range.from, range.to)),
    [range],
  );

  const liveRows = useMemo<Project[]>(
    () => (live.data?.projects ?? []).filter((p) => !p.isArchived).map(toProjectView),
    [live.data],
  );

  // A live session shows only real projects (or nothing) — never the demo portfolio.
  const rows = isLive ? liveRows : mockRows;
  const projectMinutes = isLive ? (live.data?.minutes ?? new Map<string, number>()) : mockMinutes;
  const liveCosts = live.data?.costs ?? null;
  // Money is shown in live mode only when real hourly rates produced some.
  const hasRates = !!liveCosts && [...liveCosts.values()].some((c) => c > 0);
  const showCost = !isLive || hasRates;

  const minutesOf = (p: Project) => projectMinutes.get(p.id) ?? 0;
  const costOf = (p: Project) =>
    isLive ? (liveCosts?.get(p.id) ?? 0) : Math.round((minutesOf(p) / 60) * DEMO_BLENDED_RATE);

  const totalMinutes = rows.reduce((s, p) => s + minutesOf(p), 0);
  const totalCost = isLive
    ? Math.round(rows.reduce((s, p) => s + costOf(p), 0) * 100) / 100
    : Math.round((totalMinutes / 60) * DEMO_BLENDED_RATE);
  const topProject = [...rows].sort((a, b) => minutesOf(b) - minutesOf(a))[0];

  const chartData = [...rows]
    .sort((a, b) => minutesOf(b) - minutesOf(a))
    .map((p) => ({ id: p.id, name: p.title, minutes: minutesOf(p), color: p.color }));

  const allColumns: Column<Project>[] = [
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
    { key: "share", header: "Share", align: "right", sortValue: (r) => minutesOf(r), render: (r) => <Badge tone="muted">{safePercent(minutesOf(r), totalMinutes)}%</Badge> },
    { key: "cost", header: isLive ? "Labor cost" : "Est. cost", align: "right", sortValue: (r) => costOf(r), render: (r) => <span className="font-medium">${costOf(r).toLocaleString()}</span> },
  ];
  // Live: no all-time totals from these endpoints, and no cost column without real rates.
  const columns = allColumns.filter((c) => !(isLive && c.key === "total") && !(c.key === "cost" && !showCost));

  function onRange(r: ResolvedRange) { setRange(r); setRangeKey(r.key); }
  function doExport() {
    if (isLive) {
      exportRecords(`projects-${range.key}`, rows, [
        { header: "Project", value: (r) => r.title },
        { header: "Members", value: (r) => r.memberIds.length },
        { header: `${range.label} (min)`, value: (r) => minutesOf(r) },
        ...(hasRates ? [{ header: "Labor cost USD", value: (r: Project) => costOf(r) }] : []),
      ]);
      return;
    }
    exportRecords(`projects-${range.key}`, rows, [
      { header: "Project", value: (r) => r.title },
      { header: "Members", value: (r) => r.memberIds.length },
      { header: `${range.label} (min)`, value: (r) => minutesOf(r) },
      { header: "Total (min)", value: (r) => r.loggedTotal },
      { header: "Est. cost USD", value: (r) => costOf(r) },
    ]);
  }

  const liveCostSub = hasRates
    ? "from members' hourly rates"
    : liveCosts === null && live.data
      ? "Rates couldn't be loaded"
      : "No hourly rates set";

  return (
    <ReportShell
      title="Project Breakdown"
      description="Time and estimated cost distribution across active projects."
      actions={<ExportMenu onExportCSV={doExport} />}
    >
      <FilterBar rangeKey={rangeKey} onRange={onRange} />

      {isLive && <LiveReportNotice loading={live.loading} error={live.error} onRetry={live.retry} />}

      <KpiGrid>
        <Kpi label="Total time" value={formatDuration(totalMinutes)} icon={Clock} tone="#6d5efc" sub={range.label} />
        <Kpi label="Active projects" value={String(rows.length)} icon={FolderKanban} tone="#0ea5e9" />
        <Kpi label="Most active" value={topProject?.title ?? "—"} icon={Users} tone="#ec4899" sub={topProject ? formatDuration(minutesOf(topProject)) : ""} />
        {isLive ? (
          <Kpi label="Labor cost" value={hasRates ? `$${totalCost.toLocaleString()}` : "—"} icon={DollarSign} tone="#22c55e" sub={liveCostSub} />
        ) : (
          <Kpi label="Est. labor cost" value={`$${totalCost.toLocaleString()}`} icon={DollarSign} tone="#22c55e" sub={`@ $${DEMO_BLENDED_RATE}/hr blended`} />
        )}
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
                  {chartData.map((d) => <Cell key={d.id} fill={d.color} />)}
                </Bar>
              </BarChart>
            </ResponsiveContainer>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader><CardTitle>Details</CardTitle></CardHeader>
        <CardContent>
          <DataTable
            columns={columns}
            rows={rows}
            initialSort={{ key: "time", dir: "desc" }}
            emptyText={isLive ? "No active projects." : undefined}
          />
        </CardContent>
      </Card>
    </ReportShell>
  );
}
