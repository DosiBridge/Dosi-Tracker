"use client";

import { useCallback, useMemo } from "react";
import { BadgeDollarSign, Clock, Sheet, Users, Wallet } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { DataTable, type Column } from "@/components/ui/data-table";
import { getApi } from "@/hooks/useApi";
import { projectMembersRoute, reportingRoute, type ReportingQuery } from "@/lib/api-contract";
import { downloadCSV } from "@/lib/export";
import {
  asArray,
  decimalHoursTotal,
  displayedTotal,
  formatMoney,
  formatRate,
  legacyPayrollLines,
  memberName,
  payrollByMember,
  payrollLinesFromBlocks,
  payrollTotals,
  ratesByMember,
  readPayroll,
  readSummary,
  type MemberRate,
  type PayrollLine,
  type PayrollMember,
} from "@/lib/report-math";
import { billing, projectById, userById } from "@/lib/tenant-data";
import { decimalHours, formatHM, projectLabel } from "@/lib/work-report";
import {
  AccessNotice,
  ExportMenu,
  HoursFormatToggle,
  Kpi,
  KpiGrid,
  LiveReportNotice,
  MemberCell,
  NoTimeState,
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
 * PAYROLL & BILLING — what each member earned over the period, per project.
 * The server prices every tracked block at the member's hourly rate on its
 * project IN EFFECT when the block started (rates are effective-dated), so a
 * raise or a removed membership never reprices past hours. Amounts are in the
 * workspace currency with cents, rounded per member and project; every total
 * is the cent-exact sum of the rows above it.
 *
 * Rates are team-management data: the report is for Tracker.Team.Manage
 * holders. Live: GET /api/app/reporting/payroll (an older server without it
 * is priced at current rates, and says so); demo: the seeded rates.
 * ========================================================================== */

interface Loaded {
  lines: PayrollLine[];
  /** Priced in the browser at current rates (a server without the payroll endpoint). */
  legacy: boolean;
}

function isNotFound(err: unknown): boolean {
  if (!err || typeof err !== "object") return false;
  const status = (err as { status?: unknown }).status;
  if (status === 404) return true;
  const message = (err as { message?: unknown }).message;
  return typeof message === "string" && /\b(404|Not Found)\b/.test(message);
}

/**
 * The period's pay lines. A server without the payroll endpoint (404) gets the
 * old pricing — each project's tracked hours × its members' current rates —
 * flagged `legacy` so the report can say so.
 */
export async function fetchPayroll(query: ReportingQuery): Promise<Loaded> {
  try {
    return { lines: readPayroll(await getApi(reportingRoute("payroll", query))), legacy: false };
  } catch (err) {
    if (!isNotFound(err)) throw err;
  }
  const summary = readSummary(await getApi(reportingRoute("summary", query)));
  const projects = await Promise.all(
    summary.perProject
      .filter((p) => p.trackedMinutes > 0)
      .map(async (p) => {
        const [members, projectSummary] = await Promise.all([
          getApi(projectMembersRoute(p.projectId)),
          getApi(reportingRoute("summary", { ...query, projectId: p.projectId })),
        ]);
        return {
          projectId: p.projectId,
          projectTitle: p.title,
          rates: ratesByMember(asArray<MemberRate>(members)),
          perUser: readSummary(projectSummary).perUser.map((u) => ({ userId: u.userId, trackedMinutes: u.trackedMinutes })),
        };
      }),
  );
  return { lines: legacyPayrollLines(projects), legacy: true };
}

interface MemberRow extends PayrollMember {
  name: string;
}

interface LineRow extends PayrollLine {
  key: string;
  name: string;
  project: string;
}

export function PayrollReport() {
  const scope = useReportScope();
  const { isLive, projectId, memberId, access } = scope;
  const range = useReportPeriod("month", isLive);
  const { period, timeZone } = range;
  const { format, setFormat, fmt } = useHoursFormat();
  const allowed = access.manageTeam;

  const load = useCallback(
    () => fetchPayroll({ from: period.from, to: period.to, projectId, userId: memberId, timeZone }),
    [period, projectId, memberId, timeZone],
  );
  const live = usePeriodReport(isLive && allowed, range.ready && access.ready, load);

  const demo = useMemo<Loaded | null>(() => {
    if (isLive) return null;
    const rates = new Map(billing.map((b) => [b.userId, b.rate]));
    return { lines: payrollLinesFromBlocks(demoActivities(period, projectId, memberId), (userId) => rates.get(userId) ?? 0), legacy: false };
  }, [isLive, period, projectId, memberId]);

  // A live session shows only real pay (or nothing) — never the demo billing sheet.
  const loaded = isLive ? live.data : demo;
  const lines = useMemo(() => loaded?.lines ?? [], [loaded]);
  const pending = isLive && (!access.ready || (allowed && live.pending));

  const nameOf = useCallback((userId: string, hint: string | null) => memberName(userId, userById(userId) ?? (hint ? { name: hint } : null)), []);
  const members = useMemo<MemberRow[]>(() => payrollByMember(lines).map((m) => ({ ...m, name: nameOf(m.userId, m.userName) })), [lines, nameOf]);
  const lineRows = useMemo<LineRow[]>(
    () =>
      members.flatMap((m) =>
        m.lines.map((l) => ({
          ...l,
          key: `${l.userId}:${l.projectId}`,
          name: m.name,
          project: projectLabel(l.projectId, projectById(l.projectId) ?? (l.projectTitle ? { title: l.projectTitle } : null)),
        })),
      ),
    [members],
  );
  const totals = payrollTotals(lines);
  const totalHours = displayedTotal(members.map((m) => m.minutes), format);
  const hasTime = totals.minutes > 0;
  const dash = "—";

  const memberColumns: Column<MemberRow>[] = [
    { key: "name", header: "Member", sortValue: (r) => r.name, render: (r) => <MemberCell name={r.name} sub={userById(r.userId)?.designation} /> },
    {
      key: "projects",
      header: "Projects",
      align: "right",
      sortValue: (r) => r.lines.length,
      render: (r) => <span className="tabular-nums">{r.lines.length}</span>,
    },
    {
      key: "hours",
      header: "Hours",
      align: "right",
      sortValue: (r) => r.minutes,
      render: (r) => (
        <span className="tabular-nums">
          <span className="font-medium">{fmt(r.minutes)}</span>
          {r.unpricedMinutes > 0 && <span className="block text-xs text-warning-on-tint">{fmt(r.unpricedMinutes)} without a rate</span>}
        </span>
      ),
    },
    {
      key: "rate",
      header: "Rate",
      align: "right",
      sortValue: (r) => r.rate,
      render: (r) => (r.rate > 0 ? <span className="tabular-nums">{formatRate(r.rate)}</span> : <Badge tone="warning">No rate</Badge>),
    },
    { key: "amount", header: "Amount", align: "right", sortValue: (r) => r.amount, render: (r) => <span className="font-semibold tabular-nums">{formatMoney(r.amount)}</span> },
  ];

  const lineColumns: Column<LineRow>[] = [
    { key: "name", header: "Member", sortValue: (r) => r.name, render: (r) => <span className="font-medium">{r.name}</span> },
    { key: "project", header: "Project", sortValue: (r) => r.project, render: (r) => <span className="text-muted-foreground">{r.project}</span> },
    { key: "hours", header: "Hours", align: "right", sortValue: (r) => r.minutes, render: (r) => <span className="tabular-nums">{fmt(r.minutes)}</span> },
    {
      key: "rate",
      header: "Hourly rate",
      align: "right",
      sortValue: (r) => r.rate,
      render: (r) => (r.rate > 0 ? <span className="tabular-nums">{formatRate(r.rate)}</span> : <Badge tone="warning">No rate</Badge>),
    },
    { key: "amount", header: "Amount", align: "right", sortValue: (r) => r.amount, render: (r) => <span className="font-semibold tabular-nums">{formatMoney(r.amount)}</span> },
  ];

  const memberFooter = (
    <>
      <td className="sticky left-0 z-10 bg-card px-3 py-2.5">Total</td>
      <td className="px-3 py-2.5 text-right tabular-nums">{lineRows.length}</td>
      <td className="px-3 py-2.5 text-right tabular-nums">{fmt(totalHours)}</td>
      <td className="px-3 py-2.5 text-right tabular-nums">{totals.rate > 0 ? formatRate(totals.rate) : dash}</td>
      <td className="px-3 py-2.5 text-right tabular-nums">{formatMoney(totals.amount)}</td>
    </>
  );
  const lineFooter = (
    <>
      <td className="sticky left-0 z-10 bg-card px-3 py-2.5">Total</td>
      <td className="px-3 py-2.5" />
      <td className="px-3 py-2.5 text-right tabular-nums">{fmt(displayedTotal(lineRows.map((l) => l.minutes), format))}</td>
      <td className="px-3 py-2.5" />
      <td className="px-3 py-2.5 text-right tabular-nums">{formatMoney(totals.amount)}</td>
    </>
  );

  const fileBase = `payroll-${period.fromKey}-to-${period.toKey}`;
  function exportMembers() {
    const body: (string | number)[][] = members.map((m) => [
      m.name,
      decimalHours(m.minutes),
      formatHM(m.minutes),
      decimalHours(m.minutes - m.unpricedMinutes),
      m.rate > 0 ? m.rate.toFixed(2) : "",
      m.amount.toFixed(2),
    ]);
    body.push([
      "Total",
      decimalHoursTotal(members.map((m) => m.minutes)),
      formatHM(displayedTotal(members.map((m) => m.minutes), "clock")),
      decimalHoursTotal(members.map((m) => m.minutes - m.unpricedMinutes)),
      totals.rate > 0 ? totals.rate.toFixed(2) : "",
      totals.amount.toFixed(2),
    ]);
    downloadCSV(`${fileBase}-members`, ["Member", "Hours (decimal)", "Hours (h:mm)", "Paid hours (decimal)", "Rate per paid hour (USD)", "Amount (USD)"], body);
  }
  function exportLines() {
    const body: (string | number)[][] = lineRows.map((l) => [l.name, l.project, decimalHours(l.minutes), formatHM(l.minutes), l.rate > 0 ? l.rate.toFixed(2) : "", l.amount.toFixed(2)]);
    body.push(["Total", "", decimalHoursTotal(lineRows.map((l) => l.minutes)), formatHM(displayedTotal(lineRows.map((l) => l.minutes), "clock")), "", totals.amount.toFixed(2)]);
    downloadCSV(`${fileBase}-by-project`, ["Member", "Project", "Hours (decimal)", "Hours (h:mm)", "Hourly rate (USD)", "Amount (USD)"], body);
  }

  if (isLive && access.ready && !allowed) {
    return (
      <ReportShell title="Payroll & Billing" description="Pay per member and project, each hour priced at the hourly rate in effect when it was worked.">
        <AccessNotice title="Payroll is for team managers">
          Pay is worked out from members&apos; hourly rates, which only people who can manage the team may see. Ask a workspace owner for
          access, or use the Work Report for hours.
        </AccessNotice>
      </ReportShell>
    );
  }

  return (
    <ReportShell
      title="Payroll & Billing"
      description="Pay per member and project, each hour priced at the hourly rate in effect when it was worked."
      actions={
        <ExportMenu
          onExportCSV={exportMembers}
          csvLabel="Pay by member (CSV)"
          extraItems={[{ label: "Pay by member and project (CSV)", onSelect: exportLines, icon: Sheet }]}
          disabled={pending || lines.length === 0}
        />
      }
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
      {loaded?.legacy && (
        <p role="status" className="rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-sm text-warning-on-tint">
          This server can&apos;t price hours at the rate in effect when they were worked, so these amounts use each member&apos;s current
          rate. Update the server for exact pay.
        </p>
      )}

      <KpiGrid>
        <Kpi label="Total payable" icon={Wallet} accent="green" value={pending ? dash : formatMoney(totals.amount)} sub={pending ? undefined : period.label} />
        <Kpi
          label="Hours"
          icon={Clock}
          accent="blue"
          value={pending ? dash : fmt(totalHours)}
          sub={pending ? undefined : totals.unpricedMinutes > 0 ? `${fmt(totals.unpricedMinutes)} without a rate (not paid)` : "every hour has a rate"}
        />
        <Kpi label="Average rate" icon={BadgeDollarSign} accent="amber" value={pending || !(totals.rate > 0) ? dash : formatRate(totals.rate)} sub={pending ? undefined : "per paid hour"} />
        <Kpi label="Members" icon={Users} accent="violet" value={pending ? dash : String(totals.members)} sub={pending ? undefined : "with tracked time"} />
      </KpiGrid>

      {pending ? (
        live.loading ? <ReportSkeleton /> : null
      ) : !hasTime ? (
        <NoTimeState period={period} onWiden={period.custom || range.toolbar.preset !== "30d" ? () => range.choosePreset("30d") : undefined} />
      ) : (
        <>
          <Card>
            <SectionHeading title="Pay by member" sub={`${period.label} · ${members.length} ${members.length === 1 ? "member" : "members"}`} />
            <CardContent>
              <DataTable columns={memberColumns} rows={members} initialSort={{ key: "amount", dir: "desc" }} footer={memberFooter} emptyText="No tracked time in this period." />
            </CardContent>
          </Card>
          <Card>
            <SectionHeading title="By member and project" sub="Each project's hours at the member's rate on that project" />
            <CardContent>
              <DataTable columns={lineColumns} rows={lineRows} initialSort={{ key: "amount", dir: "desc" }} footer={lineFooter} emptyText="No tracked time in this period." />
            </CardContent>
          </Card>
        </>
      )}

      <ReportFootnote>
        Each block of tracked time is priced at the member&apos;s hourly rate on its project in effect when the block started, then rounded
        to the cent per member and project; every total is the sum of the rows above it. Amounts are in US dollars. Hours on a project
        without a rate are listed but not paid. Days are calendar days of the reporting time zone shown above.
      </ReportFootnote>
    </ReportShell>
  );
}
