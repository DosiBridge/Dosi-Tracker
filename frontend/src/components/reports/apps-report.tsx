"use client";

import { useCallback, useMemo, useState } from "react";
import { AppWindow, Clock, TrendingDown, TrendingUp } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { DataTable, type Column } from "@/components/ui/data-table";
import { Select } from "@/components/ui/input";
import { getApi } from "@/hooks/useApi";
import { reportingRoute } from "@/lib/api-contract";
import { APP_CATEGORY_COLORS, OTHER_APP_COLOR } from "@/lib/apps";
import { downloadCSV } from "@/lib/export";
import {
  APP_GROUPS,
  APP_GROUP_LABELS,
  appGroupTotals,
  appUsageFromBlocks,
  decimalHoursTotal,
  displayedTotal,
  readAppUsage,
  safePercent,
  type AppGroup,
  type AppUsageLine,
} from "@/lib/report-math";
import { apportionPercent, decimalHours, formatHM } from "@/lib/work-report";
import {
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
  ShareDonut,
  demoActivities,
  useHoursFormat,
  usePeriodReport,
  useReportPeriod,
  useReportScope,
} from "./report-shell";

/* ============================================================================
 * APPS & WEBSITES — the applications the team works in. Every app reads under
 * ONE name with ONE colour (lib/apps: "chrome.exe" and "Google Chrome" are the
 * same app) and is filed under its app category; apps nobody has classified
 * are shown as unclassified rather than guessed. Apps that were only running
 * in the background (no focused time) are hidden until asked for. Live:
 * /reporting/app-usage for the period, project and member, in the reporting
 * zone; demo: the same crediting rule over the seeded activity.
 * ========================================================================== */

const GROUP_COLORS: Record<AppGroup, string> = { ...APP_CATEGORY_COLORS, unclassified: OTHER_APP_COLOR };

const groupTone: Record<AppGroup, "success" | "info" | "danger" | "muted"> = {
  productive: "success",
  neutral: "info",
  unproductive: "danger",
  unclassified: "muted",
};

/** Under a minute of focused time: the app was only running in the background. */
const isBackground = (l: AppUsageLine) => l.minutes < 1;

interface Row extends AppUsageLine {
  share: number;
}

export function AppsReport() {
  const scope = useReportScope();
  const { isLive, projectId, memberId } = scope;
  const range = useReportPeriod("7d", isLive);
  const { period, timeZone } = range;
  const { format, setFormat, fmt } = useHoursFormat();
  const [category, setCategory] = useState<AppGroup | "all">("all");
  const [showBackground, setShowBackground] = useState(false);

  const load = useCallback(async (): Promise<AppUsageLine[]> => {
    const usage = await getApi(reportingRoute("app-usage", { from: period.from, to: period.to, projectId, userId: memberId, timeZone }));
    return readAppUsage(usage);
  }, [period, projectId, memberId, timeZone]);
  const live = usePeriodReport(isLive, range.ready, load);

  const demo = useMemo(() => (isLive ? null : appUsageFromBlocks(demoActivities(period, projectId, memberId))), [isLive, period, projectId, memberId]);

  // A live session shows only real usage (or nothing) — never the demo app mix.
  const lines = useMemo(() => (isLive ? (live.data ?? []) : (demo ?? [])), [isLive, live.data, demo]);
  const pending = live.pending;

  const shares = useMemo(() => apportionPercent(lines.map((l) => l.minutes)), [lines]);
  const all = useMemo<Row[]>(() => lines.map((l, i) => ({ ...l, share: shares[i] })), [lines, shares]);
  const background = all.filter(isBackground).length;
  const rows = all.filter((l) => (showBackground || !isBackground(l)) && (category === "all" || l.group === category));
  const focused = all.filter((l) => !isBackground(l));

  const totals = appGroupTotals(lines);
  const totalTime = displayedTotal(lines.map((l) => l.minutes), format);
  const top = focused[0] ?? null;
  const idleKnown = lines.some((l) => l.idleMinutes !== null);
  const hasTime = totals.total > 0;
  const dash = "—";

  const columns: Column<Row>[] = [
    {
      key: "name",
      header: "Application",
      sortValue: (r) => r.name,
      render: (r) => (
        <span className="flex min-w-0 items-center gap-2 font-medium">
          <span aria-hidden className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ background: r.color }} />
          <span className="truncate" title={r.name}>
            {r.name}
          </span>
        </span>
      ),
    },
    { key: "group", header: "Category", sortValue: (r) => APP_GROUP_LABELS[r.group], render: (r) => <Badge tone={groupTone[r.group]}>{APP_GROUP_LABELS[r.group]}</Badge> },
    {
      key: "minutes",
      header: "Time",
      align: "right",
      sortValue: (r) => r.minutes,
      render: (r) => (isBackground(r) ? <span className="text-xs text-muted-foreground">Background</span> : <span className="font-semibold tabular-nums">{fmt(r.minutes)}</span>),
    },
    { key: "share", header: "Share", align: "right", sortValue: (r) => r.minutes, render: (r) => <span className="tabular-nums text-muted-foreground">{r.share}%</span> },
    ...(idleKnown
      ? [
          {
            key: "idle",
            header: "Idle",
            align: "right" as const,
            sortValue: (r: Row) => r.idleMinutes ?? -1,
            render: (r: Row) => (r.idleMinutes === null ? <Dash label="Not reported" /> : <span className="tabular-nums">{fmt(r.idleMinutes)}</span>),
          },
        ]
      : []),
    { key: "users", header: "Members", align: "right", sortValue: (r) => r.userCount, render: (r) => <span className="tabular-nums">{r.userCount}</span> },
    { key: "sessions", header: "Sessions", align: "right", sortValue: (r) => r.activityCount, render: (r) => <span className="tabular-nums">{r.activityCount}</span> },
  ];

  function exportCsv() {
    const headers = ["Application", "Category", "Hours (decimal)", "Hours (h:mm)", "Share %", ...(idleKnown ? ["Idle hours (decimal)"] : []), "Members", "Sessions"];
    const body: (string | number)[][] = rows.map((r) => [
      r.name,
      APP_GROUP_LABELS[r.group],
      decimalHours(r.minutes),
      formatHM(r.minutes),
      r.share,
      ...(idleKnown ? [r.idleMinutes === null ? "" : decimalHours(r.idleMinutes)] : []),
      r.userCount,
      r.activityCount,
    ]);
    body.push([
      "Total",
      "",
      decimalHoursTotal(rows.map((r) => r.minutes)),
      formatHM(displayedTotal(rows.map((r) => r.minutes), "clock")),
      rows.reduce((s, r) => s + r.share, 0),
      ...(idleKnown ? [decimalHoursTotal(rows.map((r) => r.idleMinutes ?? 0))] : []),
      "",
      "",
    ]);
    downloadCSV(`apps-${period.fromKey}-to-${period.toKey}`, headers, body);
  }

  return (
    <ReportShell
      title="Apps & Websites"
      description="The applications your team works in, by time, by member and by category."
      actions={<ExportMenu onExportCSV={exportCsv} disabled={pending || rows.length === 0} />}
    >
      <ReportToolbar
        period={range}
        projectId={projectId}
        onProject={scope.setProjectId}
        memberId={memberId}
        onMember={scope.setMemberId}
        liveWorkerId={scope.liveWorkerId}
        extra={
          <>
            <Select
              aria-label="Category"
              value={category}
              onChange={(e) => setCategory(e.target.value as AppGroup | "all")}
              className="h-10 w-full min-w-0 sm:w-auto sm:min-w-40"
            >
              <option value="all">All categories</option>
              {APP_GROUPS.map((g) => (
                <option key={g} value={g}>
                  {APP_GROUP_LABELS[g]}
                </option>
              ))}
            </Select>
            <HoursFormatToggle value={format} onChange={setFormat} />
          </>
        }
      />

      {isLive && (
        <div className="no-print">
          <LiveReportNotice loading={live.loading} error={live.error} onRetry={live.retry} />
        </div>
      )}

      <KpiGrid>
        <Kpi label="App time" icon={Clock} accent="blue" value={pending ? dash : fmt(totalTime)} sub={pending ? undefined : `${focused.length} ${focused.length === 1 ? "app" : "apps"} in use`} />
        <Kpi label="Most used" icon={AppWindow} accent="cyan" value={pending || !top ? dash : top.name} sub={pending || !top ? undefined : `${fmt(top.minutes)} · ${top.share}%`} />
        <Kpi
          label="Productive apps"
          icon={TrendingUp}
          accent="green"
          value={pending || !hasTime ? dash : `${safePercent(totals.productive, totals.total)}%`}
          sub={pending || !hasTime ? undefined : `${fmt(totals.productive)} of app time`}
        />
        <Kpi
          label="Unproductive apps"
          icon={TrendingDown}
          accent="pink"
          value={pending || !hasTime ? dash : `${safePercent(totals.unproductive, totals.total)}%`}
          sub={pending || !hasTime ? undefined : `${fmt(totals.unproductive)} of app time`}
        />
      </KpiGrid>

      {pending ? (
        live.loading ? <ReportSkeleton /> : null
      ) : !hasTime ? (
        <NoTimeState
          period={period}
          what="No app time in this period"
          onWiden={period.custom || range.toolbar.preset !== "30d" ? () => range.choosePreset("30d") : undefined}
        />
      ) : (
        <>
          <div className="grid grid-cols-1 gap-5 xl:grid-cols-3">
            <Card>
              <SectionHeading title="By category" sub="What the apps are usually used for" />
              <CardContent>
                <ShareDonut
                  label="App time by category"
                  fmt={fmt}
                  slices={APP_GROUPS.map((g) => ({ key: g, label: APP_GROUP_LABELS[g], minutes: totals[g], color: GROUP_COLORS[g] }))}
                />
              </CardContent>
            </Card>
            <Card className="xl:col-span-2">
              <SectionHeading
                title="Top applications"
                sub={category === "all" ? "By focused time" : `${APP_GROUP_LABELS[category]} apps, by focused time`}
              />
              <CardContent>
                {rows.some((r) => !isBackground(r)) ? (
                  <RankedBars
                    label="Top applications by focused time"
                    items={rows
                      .filter((r) => !isBackground(r))
                      .slice(0, 8)
                      .map((r) => ({
                        key: r.key,
                        label: r.name,
                        value: r.minutes,
                        display: fmt(r.minutes),
                        detail: `· ${r.share}%`,
                        color: r.color,
                        badge: (
                          <Badge tone={groupTone[r.group]} className="hidden shrink-0 sm:inline-flex">
                            {APP_GROUP_LABELS[r.group]}
                          </Badge>
                        ),
                      }))}
                  />
                ) : (
                  <p className="py-8 text-center text-sm text-muted-foreground">
                    {category === "all" ? "Only background apps in this period." : `No ${APP_GROUP_LABELS[category].toLowerCase()} apps in this period.`}
                  </p>
                )}
              </CardContent>
            </Card>
          </div>

          <Card>
            <SectionHeading title="All applications" sub={`${rows.length} ${rows.length === 1 ? "app" : "apps"}`}>
              {background > 0 && (
                <button
                  type="button"
                  aria-pressed={showBackground}
                  onClick={() => setShowBackground((v) => !v)}
                  className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-border px-3 text-xs font-medium text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  {showBackground ? "Hide background apps" : `Show background apps (${background})`}
                </button>
              )}
            </SectionHeading>
            <CardContent>
              <DataTable columns={columns} rows={rows} initialSort={{ key: "minutes", dir: "desc" }} emptyText="No apps match these filters." />
            </CardContent>
          </Card>
        </>
      )}

      <ReportFootnote>
        Time is each application&apos;s focused (foreground) time, as reported by the desktop app; a block from an older agent without
        per-window time counts for its focused app. One app reads under one name everywhere (chrome.exe and Google Chrome are the same
        app). Categories describe what an app is usually used for; apps not yet classified are listed as unclassified. Background apps
        were running but never in front.
      </ReportFootnote>
    </ReportShell>
  );
}
