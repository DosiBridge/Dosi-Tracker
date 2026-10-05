"use client";

import { useCallback, useMemo } from "react";
import { Clock, DollarSign, FolderKanban, Trophy } from "lucide-react";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { DataTable, type Column } from "@/components/ui/data-table";
import { getApi } from "@/hooks/useApi";
import { downloadCSV } from "@/lib/export";
import { reportingRoute } from "@/lib/api-contract";
import {
  ALL,
  asArray,
  decimalHoursTotal,
  displayedTotal,
  formatMoney,
  payrollLinesFromBlocks,
  projectCosts,
  readSummary,
  sumMoney,
  summarizeBlocks,
  type ReportSummary,
} from "@/lib/report-math";
import { billing, projectById, projects as sessionProjects, userById } from "@/lib/tenant-data";
import { apportionPercent, decimalHours, formatHM, projectLabel } from "@/lib/work-report";
import { fetchPayroll } from "./payroll-report";
import {
  ActivityBadge,
  Dash,
  ExportMenu,
  HoursFormatToggle,
  Kpi,
  KpiGrid,
  LiveReportNotice,
  NoTimeState,
  RankedBars,
  ReportFootnote,
  ReportShell,
  ReportSkeleton,
  ReportToolbar,
  SectionHeading,
  demoActivities,
  useHoursFormat,
  usePeriodReport,
  useReportPeriod,
  useReportScope,
} from "./report-shell";

/* ============================================================================
 * PROJECT BREAKDOWN — where the period's hours went, per project: time, share,
 * activity and labour cost. Every project with time in the period is listed,
 * archived ones included (and labelled) so the total matches the other
 * reports; active projects without time are listed too. Labour cost is the
 * effective-dated pay of /reporting/payroll grouped by project (one request,
 * not one per project), shown only to Tracker.Team.Manage holders.
 * ========================================================================== */

/** GET /api/app/project item, as far as this report reads it. */
interface ApiProject {
  id: string;
  title?: string | null;
  color?: string | null;
  isArchived?: boolean | null;
}

interface Loaded {
  summary: ReportSummary;
  /** A fresh project list; null when it couldn't be read (the session's list is used). */
  projects: ApiProject[] | null;
  /** projectId → labour cost; null without permission or when pay couldn't be read. */
  costs: Map<string, number> | null;
}

interface ProjectInfo {
  title: string | null;
  color: string | null;
  archived: boolean;
  memberIds: string[];
}

interface Row {
  projectId: string;
  title: string;
  color: string;
  archived: boolean;
  minutes: number;
  share: number;
  activity: number | null;
  cost: number | null;
  memberIds: string[];
}

const FALLBACK_COLOR = "#768390";

export function ProjectsReport() {
  const scope = useReportScope();
  const { isLive, projectId, memberId, access } = scope;
  const range = useReportPeriod("30d", isLive);
  const { period, timeZone } = range;
  const { format, setFormat, fmt } = useHoursFormat();
  const showCost = access.manageTeam;

  const load = useCallback(async (): Promise<Loaded> => {
    const query = { from: period.from, to: period.to, projectId, userId: memberId, timeZone };
    const [summary, list, pay] = await Promise.all([
      getApi(reportingRoute("summary", query)),
      // The project list only adds names and colours (the session has them too).
      getApi("/api/app/project?MaxResultCount=1000").catch(() => null),
      // Labour cost is secondary to the hours: without it the report still stands.
      showCost ? fetchPayroll(query).catch(() => null) : Promise.resolve(null),
    ]);
    return {
      summary: readSummary(summary),
      projects: Array.isArray(list) ? asArray<ApiProject>(list).filter((p) => p && typeof p.id === "string") : null,
      costs: pay ? projectCosts(pay.lines) : null,
    };
  }, [period, projectId, memberId, timeZone, showCost]);
  const live = usePeriodReport(isLive, range.ready && access.ready, load);

  const demo = useMemo<Loaded | null>(() => {
    if (isLive) return null;
    const list = demoActivities(period, projectId, memberId);
    const rates = new Map(billing.map((b) => [b.userId, b.rate]));
    return {
      summary: summarizeBlocks(list, timeZone).summary,
      projects: null,
      costs: projectCosts(payrollLinesFromBlocks(list, (userId) => rates.get(userId) ?? 0)),
    };
  }, [isLive, period, projectId, memberId, timeZone]);

  // A live session shows only real projects (or nothing) — never the demo portfolio.
  const loaded = isLive ? live.data : demo;
  const pending = isLive && (!access.ready || live.pending);

  const rows = useMemo<Row[]>(() => {
    if (!loaded) return [];
    const fresh = new Map((loaded.projects ?? []).map((p) => [p.id, p]));
    const info = (id: string, fallback?: { title: string | null; isArchived: boolean | null }): ProjectInfo => {
      const session = projectById(id);
      const api = fresh.get(id);
      return {
        title: api?.title?.trim() || session?.title || fallback?.title || null,
        color: api?.color || session?.color || null,
        archived: api?.isArchived ?? session?.archived ?? fallback?.isArchived ?? false,
        memberIds: session?.memberIds ?? [],
      };
    };
    const withTime = loaded.summary.perProject.filter((p) => p.trackedMinutes > 0);
    const seen = new Set(withTime.map((p) => p.projectId));
    // Active projects without time in the period are listed too (a filter narrows them like the figures).
    const candidates = loaded.projects ? loaded.projects.map((p) => p.id) : sessionProjects.map((p) => p.id);
    const idle = candidates.filter((id) => {
      if (seen.has(id) || (projectId !== ALL && id !== projectId)) return false;
      const p = info(id);
      return !p.archived && (memberId === ALL || p.memberIds.includes(memberId));
    });
    const list = [
      ...withTime.map((p) => ({ id: p.projectId, minutes: Math.round(p.trackedMinutes), activity: p.averageProductivity, p: info(p.projectId, p) })),
      ...idle.map((id) => ({ id, minutes: 0, activity: null, p: info(id) })),
    ];
    const shares = apportionPercent(list.map((r) => r.minutes));
    return list.map((r, i) => ({
      projectId: r.id,
      title: projectLabel(r.id, r.p.title ? { title: r.p.title } : null),
      color: r.p.color ?? FALLBACK_COLOR,
      archived: r.p.archived,
      minutes: r.minutes,
      share: shares[i],
      activity: r.activity === null ? null : Math.round(r.activity),
      cost: loaded.costs ? (loaded.costs.get(r.id) ?? 0) : null,
      memberIds: r.p.memberIds,
    }));
  }, [loaded, projectId, memberId]);

  const tracked = rows.filter((r) => r.minutes > 0);
  const totalMinutes = displayedTotal(rows.map((r) => r.minutes), format);
  const totalCost = sumMoney(rows.map((r) => r.cost ?? 0));
  const costKnown = showCost && rows.some((r) => r.cost !== null);
  const activeCount = rows.filter((r) => !r.archived).length;
  const top = [...tracked].sort((a, b) => b.minutes - a.minutes)[0] ?? null;
  const archivedWithTime = tracked.filter((r) => r.archived).length;
  const hasTime = tracked.length > 0;
  const dash = "—";

  const columns: Column<Row>[] = [
    {
      key: "title",
      header: "Project",
      sortValue: (r) => r.title,
      render: (r) => (
        <span className="flex min-w-0 flex-wrap items-center gap-2 font-medium">
          <span aria-hidden className="h-3 w-3 shrink-0 rounded" style={{ background: r.color }} />
          <span className="min-w-0 truncate" title={r.title}>
            {r.title}
          </span>
          {r.archived && <Badge tone="muted">Archived</Badge>}
        </span>
      ),
    },
    {
      key: "team",
      header: "Team",
      hideOnMobile: true,
      render: (r) =>
        r.memberIds.length ? (
          <div className="flex -space-x-2">
            {r.memberIds.slice(0, 4).map((id) => {
              const u = userById(id);
              return u ? <Avatar key={id} name={u.name} size="sm" /> : null;
            })}
            {r.memberIds.length > 4 && (
              <span className="flex h-8 w-8 items-center justify-center rounded-full bg-muted text-xs ring-2 ring-card">+{r.memberIds.length - 4}</span>
            )}
          </div>
        ) : (
          <Dash label="No members" />
        ),
    },
    {
      key: "time",
      header: "Time",
      align: "right",
      sortValue: (r) => r.minutes,
      render: (r) => (r.minutes > 0 ? <span className="font-semibold tabular-nums">{fmt(r.minutes)}</span> : <Dash label="No time" />),
    },
    { key: "share", header: "Share", align: "right", sortValue: (r) => r.minutes, render: (r) => <span className="tabular-nums text-muted-foreground">{r.share}%</span> },
    { key: "activity", header: "Activity", align: "right", sortValue: (r) => r.activity ?? -1, render: (r) => (r.minutes > 0 ? <ActivityBadge percent={r.activity} /> : <Dash label="No time" />) },
    ...(costKnown
      ? [
          {
            key: "cost",
            header: "Labour cost",
            align: "right" as const,
            sortValue: (r: Row) => r.cost ?? -1,
            render: (r: Row) => (r.cost === null ? <Dash /> : <span className="tabular-nums">{formatMoney(r.cost)}</span>),
          },
        ]
      : []),
  ];

  const footer = (
    <>
      <td className="sticky left-0 z-10 bg-card px-3 py-2.5">Total</td>
      <td className="hidden px-3 py-2.5 md:table-cell" />
      <td className="px-3 py-2.5 text-right tabular-nums">{fmt(totalMinutes)}</td>
      <td className="px-3 py-2.5 text-right tabular-nums">{hasTime ? "100%" : dash}</td>
      <td className="px-3 py-2.5 text-right tabular-nums">
        {loaded?.summary.averageProductivity === null || loaded?.summary.averageProductivity === undefined ? dash : `${Math.round(loaded.summary.averageProductivity)}%`}
      </td>
      {costKnown && <td className="px-3 py-2.5 text-right tabular-nums">{formatMoney(totalCost)}</td>}
    </>
  );

  function exportCsv() {
    const headers = ["Project", "Archived", "Hours (decimal)", "Hours (h:mm)", "Share %", "Activity %", ...(costKnown ? ["Labour cost (USD)"] : [])];
    const body: (string | number)[][] = rows.map((r) => [
      r.title,
      r.archived ? "Yes" : "No",
      decimalHours(r.minutes),
      formatHM(r.minutes),
      r.share,
      r.activity ?? "",
      ...(costKnown ? [r.cost === null ? "" : r.cost.toFixed(2)] : []),
    ]);
    body.push([
      "Total",
      "",
      decimalHoursTotal(rows.map((r) => r.minutes)),
      formatHM(displayedTotal(rows.map((r) => r.minutes), "clock")),
      hasTime ? 100 : 0,
      "",
      ...(costKnown ? [totalCost.toFixed(2)] : []),
    ]);
    downloadCSV(`projects-${period.fromKey}-to-${period.toKey}`, headers, body);
  }

  return (
    <ReportShell
      title="Project Breakdown"
      description="Time, share and labour cost per project, archived projects included."
      actions={<ExportMenu onExportCSV={exportCsv} disabled={pending || !hasTime} />}
    >
      <ReportToolbar
        period={range}
        projectId={projectId}
        onProject={scope.setProjectId}
        memberId={memberId}
        onMember={scope.setMemberId}
        liveWorkerId={scope.liveWorkerId}
        extra={<HoursFormatToggle value={format} onChange={setFormat} />}
      />

      {isLive && (
        <div className="no-print">
          <LiveReportNotice loading={live.loading} error={live.error} onRetry={live.retry} />
        </div>
      )}

      <KpiGrid>
        <Kpi label="Total time" icon={Clock} accent="blue" value={pending ? dash : fmt(totalMinutes)} sub={pending ? undefined : period.label} />
        <Kpi
          label="Projects with time"
          icon={FolderKanban}
          accent="cyan"
          value={pending ? dash : String(tracked.length)}
          sub={pending ? undefined : `${activeCount} active${archivedWithTime ? ` · ${archivedWithTime} archived with time` : ""}`}
        />
        <Kpi label="Most active" icon={Trophy} accent="violet" value={pending || !top ? dash : top.title} sub={pending || !top ? undefined : `${fmt(top.minutes)} · ${top.share}%`} />
        <Kpi
          label="Labour cost"
          icon={DollarSign}
          accent="green"
          value={pending || !costKnown ? dash : formatMoney(totalCost)}
          sub={pending ? undefined : !showCost ? "needs permission to manage the team" : costKnown ? "at the rates in effect" : "pay couldn't be loaded"}
        />
      </KpiGrid>

      {pending ? (
        live.loading ? <ReportSkeleton /> : null
      ) : !hasTime ? (
        <NoTimeState period={period} onWiden={period.custom || range.toolbar.preset !== "30d" ? () => range.choosePreset("30d") : undefined} />
      ) : (
        <>
          <Card>
            <SectionHeading title="Time by project" sub={period.label} />
            <CardContent>
              <RankedBars
                label="Time by project"
                items={[...tracked]
                  .sort((a, b) => b.minutes - a.minutes)
                  .map((r) => ({
                    key: r.projectId,
                    label: r.title,
                    value: r.minutes,
                    display: fmt(r.minutes),
                    detail: `· ${r.share}%`,
                    color: r.color,
                    badge: r.archived ? (
                      <Badge tone="muted" className="shrink-0">
                        Archived
                      </Badge>
                    ) : undefined,
                  }))}
              />
            </CardContent>
          </Card>

          <Card>
            <SectionHeading title="All projects" sub={`${rows.length} ${rows.length === 1 ? "project" : "projects"}`} />
            <CardContent>
              <DataTable columns={columns} rows={rows} initialSort={{ key: "time", dir: "desc" }} footer={footer} emptyText="No projects." />
            </CardContent>
          </Card>
        </>
      )}

      <ReportFootnote>
        Every project with time in the period is listed, archived projects included, so the total matches the other reports; archived
        projects without time are left out. Labour cost prices each block at the member&apos;s hourly rate in effect when it was worked
        (US dollars). Days are calendar days of the reporting time zone shown above.
      </ReportFootnote>
    </ReportShell>
  );
}
