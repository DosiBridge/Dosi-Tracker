"use client";

import { useCallback, useMemo } from "react";
import Link from "next/link";
import { ArrowRight, Gauge, Hourglass, TrendingUp, Zap } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { DataTable, type Column } from "@/components/ui/data-table";
import { getApi } from "@/hooks/useApi";
import { reportingRoute } from "@/lib/api-contract";
import { APP_CATEGORY_COLORS, OTHER_APP_COLOR } from "@/lib/apps";
import { downloadCSV } from "@/lib/export";
import {
  ALL,
  APP_GROUPS,
  APP_GROUP_LABELS,
  activityBreakdown,
  appGroupTotals,
  appUsageFromBlocks,
  decimalHoursTotal,
  displayedTotal,
  memberName,
  readAppUsage,
  readSummary,
  safePercent,
  summarizeBlocks,
  type AppGroup,
  type AppUsageLine,
  type ReportSummary,
} from "@/lib/report-math";
import { userById } from "@/lib/tenant-data";
import { decimalHours, formatHM } from "@/lib/work-report";
import {
  ActivityBadge,
  ActivityLegend,
  Dash,
  ExportMenu,
  HoursFormatToggle,
  Kpi,
  KpiGrid,
  LiveReportNotice,
  MemberCell,
  MiniBar,
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
 * PRODUCTIVITY — two different questions, each with its own definition:
 *  - how much of each member's tracked time had keyboard or mouse input
 *    (active), had none (idle), or wasn't measured either way (other);
 *  - how the team's app time splits by APP CATEGORY (productive, neutral,
 *    unproductive — lib/apps; apps nobody has classified are shown apart).
 * "Productive" is only ever the app category, never the activity score.
 * Live: /reporting/summary and /reporting/app-usage for the period in the
 * reporting zone (the app split is secondary: without it the rest stands).
 * ========================================================================== */

interface Figures {
  summary: ReportSummary;
  /** null when app usage couldn't be read. */
  apps: AppUsageLine[] | null;
}

interface Row {
  userId: string;
  name: string;
  tracked: number;
  active: number | null;
  idle: number | null;
  other: number;
  activity: number | null;
}

const GROUP_COLORS: Record<AppGroup, string> = { ...APP_CATEGORY_COLORS, unclassified: OTHER_APP_COLOR };

export function ProductivityReport() {
  const scope = useReportScope();
  const { isLive, projectId, memberId } = scope;
  const range = useReportPeriod("7d", isLive);
  const { period, timeZone } = range;
  const { format, setFormat, fmt } = useHoursFormat();

  const load = useCallback(async (): Promise<Figures> => {
    const query = { from: period.from, to: period.to, projectId, userId: memberId, timeZone };
    const [summary, apps] = await Promise.all([
      getApi(reportingRoute("summary", query)),
      getApi(reportingRoute("app-usage", query)).catch(() => null),
    ]);
    return { summary: readSummary(summary), apps: Array.isArray(apps) ? readAppUsage(apps) : null };
  }, [period, projectId, memberId, timeZone]);
  const live = usePeriodReport(isLive, range.ready, load);

  const demo = useMemo<Figures | null>(() => {
    if (isLive) return null;
    const list = demoActivities(period, projectId, memberId);
    return { summary: summarizeBlocks(list, timeZone).summary, apps: appUsageFromBlocks(list) };
  }, [isLive, period, projectId, memberId, timeZone]);

  // A live session shows only real figures (or nothing) — never the demo team.
  const figures = isLive ? live.data : demo;
  const pending = live.pending;

  const rows = useMemo<Row[]>(
    () =>
      (figures?.summary.perUser ?? [])
        .filter((u) => memberId === ALL || u.userId === memberId)
        .map((u) => {
          const split = activityBreakdown(u.trackedMinutes, u.averageProductivity, u.idleMinutes);
          return {
            userId: u.userId,
            name: memberName(u.userId, userById(u.userId) ?? (u.name ? { name: u.name } : null)),
            tracked: split.tracked,
            active: split.active,
            idle: split.idle,
            other: split.other,
            activity: split.activity,
          };
        }),
    [figures, memberId],
  );

  const idleKnown = rows.some((r) => r.idle !== null);
  const activeKnown = rows.some((r) => r.active !== null);
  // Time that is neither measured active nor idle (e.g. no input capture): a column only when there is some.
  const otherShown = rows.some((r) => r.other > 0);
  const hasTime = (figures?.summary.trackedMinutes ?? 0) > 0;
  const tracked = rows.map((r) => r.tracked);
  const totalTracked = displayedTotal(tracked, format);
  const totalActive = displayedTotal(rows.map((r) => r.active ?? 0), format);
  const totalIdle = displayedTotal(rows.map((r) => r.idle ?? 0), format);
  const average = figures?.summary.averageProductivity ?? null;
  const averagePercent = average === null ? null : Math.round(average);
  const groups = figures?.apps ? appGroupTotals(figures.apps) : null;
  const dash = "—";

  const columns: Column<Row>[] = [
    { key: "name", header: "Member", sortValue: (r) => r.name, render: (r) => <MemberCell name={r.name} status={userById(r.userId)?.status} /> },
    { key: "tracked", header: "Tracked", align: "right", sortValue: (r) => r.tracked, render: (r) => <span className="font-semibold tabular-nums">{fmt(r.tracked)}</span> },
    {
      key: "active",
      header: "Active",
      align: "right",
      sortValue: (r) => r.active ?? -1,
      render: (r) => (r.active === null ? <Dash label="Not measured" /> : <span className="tabular-nums text-success-on-tint">{fmt(r.active)}</span>),
    },
    ...(idleKnown
      ? [
          {
            key: "idle",
            header: "Idle",
            align: "right" as const,
            sortValue: (r: Row) => r.idle ?? -1,
            render: (r: Row) => (r.idle === null ? <Dash label="Not reported" /> : <span className="tabular-nums text-warning-on-tint">{fmt(r.idle)}</span>),
          },
        ]
      : []),
    ...(otherShown
      ? [
          {
            key: "other",
            header: "Other tracked",
            align: "right" as const,
            sortValue: (r: Row) => r.other,
            render: (r: Row) => <span className="tabular-nums text-muted-foreground">{fmt(r.other)}</span>,
          },
        ]
      : []),
    { key: "activity", header: "Activity", align: "right", sortValue: (r) => r.activity ?? -1, render: (r) => <ActivityBadge percent={r.activity} /> },
  ];

  function exportCsv() {
    const headers = [
      "Member",
      "Tracked hours (decimal)",
      "Active hours (decimal)",
      ...(idleKnown ? ["Idle hours (decimal)"] : []),
      ...(otherShown ? ["Other tracked hours (decimal)"] : []),
      "Activity %",
      "Tracked (h:mm)",
    ];
    const body: (string | number)[][] = rows.map((r) => [
      r.name,
      decimalHours(r.tracked),
      r.active === null ? "" : decimalHours(r.active),
      ...(idleKnown ? [r.idle === null ? "" : decimalHours(r.idle)] : []),
      ...(otherShown ? [decimalHours(r.other)] : []),
      r.activity ?? "",
      formatHM(r.tracked),
    ]);
    body.push([
      "Total",
      decimalHoursTotal(tracked),
      activeKnown ? decimalHoursTotal(rows.map((r) => r.active ?? 0)) : "",
      ...(idleKnown ? [decimalHoursTotal(rows.map((r) => r.idle ?? 0))] : []),
      ...(otherShown ? [decimalHoursTotal(rows.map((r) => r.other))] : []),
      averagePercent ?? "",
      formatHM(displayedTotal(tracked, "clock")),
    ]);
    downloadCSV(`productivity-${period.fromKey}-to-${period.toKey}`, headers, body);
  }

  const productiveShare = groups && groups.total > 0 ? safePercent(groups.productive, groups.total) : null;

  return (
    <ReportShell
      aiExplain={{ slug: "productivity", query: { from: period.from, to: period.to, projectId, userId: memberId, timeZone } }}
      title="Productivity"
      description="Active and idle time per member, and time spent in productive, neutral and unproductive apps."
      actions={<ExportMenu onExportCSV={exportCsv} disabled={pending || rows.length === 0} />}
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
        <Kpi
          label="Average activity"
          icon={Gauge}
          accent="green"
          value={pending || averagePercent === null ? dash : `${averagePercent}%`}
          sub={pending ? undefined : averagePercent === null ? "keyboard & mouse not measured" : "of tracked minutes had input"}
        >
          {!pending && averagePercent !== null && <MiniBar percent={averagePercent} className="bg-success" />}
        </Kpi>
        <Kpi
          label="Active time"
          icon={Zap}
          accent="blue"
          value={pending || !activeKnown ? dash : fmt(totalActive)}
          sub={pending ? undefined : activeKnown ? `of ${fmt(totalTracked)} tracked` : "keyboard & mouse not measured"}
        />
        <Kpi
          label="Idle time"
          icon={Hourglass}
          accent="amber"
          value={pending || !idleKnown ? dash : fmt(totalIdle)}
          sub={pending ? undefined : idleKnown ? `${safePercent(totalIdle, totalTracked)}% of tracked time` : "not reported by this server"}
        />
        <Kpi
          label="Productive apps"
          icon={TrendingUp}
          accent="violet"
          value={pending || productiveShare === null ? dash : `${productiveShare}%`}
          sub={pending ? undefined : groups && groups.total > 0 ? `${fmt(groups.productive)} of app time` : "no app time recorded"}
        />
      </KpiGrid>

      {pending ? (
        live.loading ? <ReportSkeleton /> : null
      ) : !hasTime ? (
        <NoTimeState period={period} onWiden={period.custom || range.toolbar.preset !== "30d" ? () => range.choosePreset("30d") : undefined} />
      ) : (
        <>
          <div className="grid grid-cols-1 gap-5 xl:grid-cols-3">
            <Card className="xl:col-span-2">
              <SectionHeading title="Active and idle time by member" sub="Each bar is the member's tracked time">
                <ActivityLegend idle={idleKnown} other={rows.some((r) => r.other > 0)} />
              </SectionHeading>
              <CardContent>
                <RankedBars
                  label="Tracked time by member, split into active, other and idle time"
                  items={[...rows]
                    .sort((a, b) => b.tracked - a.tracked)
                    .map((r) => ({
                      key: r.userId,
                      label: r.name,
                      value: r.tracked,
                      display: fmt(r.tracked),
                      detail: r.activity === null ? "· not measured" : `· ${r.activity}% active`,
                      segments: [
                        { key: "active" as const, label: `Active ${fmt(r.active ?? 0)}`, minutes: r.active ?? 0 },
                        { key: "other" as const, label: `Other tracked ${fmt(r.other)}`, minutes: r.other },
                        { key: "idle" as const, label: `Idle ${fmt(r.idle ?? 0)}`, minutes: r.idle ?? 0 },
                      ],
                    }))}
                />
              </CardContent>
            </Card>
            <Card>
              <SectionHeading title="Time by app category" sub="Focused app time, by what the app is for">
                <Link
                  href="/reports/apps-urls"
                  className="inline-flex items-center gap-1 rounded-md text-xs font-medium text-primary-strong hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  Apps <ArrowRight aria-hidden className="h-3.5 w-3.5" />
                </Link>
              </SectionHeading>
              <CardContent>
                {groups ? (
                  <ShareDonut
                    label="App time by category"
                    fmt={fmt}
                    slices={APP_GROUPS.map((g) => ({ key: g, label: APP_GROUP_LABELS[g], minutes: groups[g], color: GROUP_COLORS[g] }))}
                  />
                ) : (
                  <p className="py-8 text-center text-sm text-muted-foreground">App usage couldn&apos;t be loaded. The member figures are complete.</p>
                )}
              </CardContent>
            </Card>
          </div>

          <Card>
            <SectionHeading title="By member" sub={`${rows.length} ${rows.length === 1 ? "member" : "members"} with tracked time`} />
            <CardContent>
              <DataTable columns={columns} rows={rows} initialSort={{ key: "tracked", dir: "desc" }} emptyText="No tracked time in this period." />
            </CardContent>
          </Card>
        </>
      )}

      <ReportFootnote>
        Activity % is the share of tracked minutes with keyboard or mouse input; active time is tracked time × activity %, idle time is the
        minutes without input, and other tracked time is neither (for example blocks recorded without input capture). Productive, neutral
        and unproductive are app categories — what an app is usually used for — and say nothing about input; apps that haven&apos;t been
        classified are shown as unclassified. Days are calendar days of the reporting time zone shown above.
      </ReportFootnote>
    </ReportShell>
  );
}
