"use client";

import { useCallback, useMemo, useState } from "react";
import { DollarSign, Clock, Wallet, Receipt } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid, LiveReportNotice, useLiveReport } from "./report-shell";
import { activities, billing, userById } from "@/lib/tenant-data";
import { minutesForUser } from "@/lib/metrics";
import { filterActivitiesByRange, type RangeKey } from "@/lib/reports-data";
import {
  asArray,
  computePayroll,
  memberName,
  ratesByMember,
  reportQuery,
  type MemberRate,
  type MinutesRow,
  type PayrollLine,
} from "@/lib/report-math";
import { exportRecords } from "@/lib/export";
import { cn } from "@/lib/utils";
import { getApi } from "@/hooks/useApi";
import { useSession } from "@/components/session-provider";
import { resolveRange, type ResolvedRange } from "./date-range-picker";

interface Row {
  userId: string;
  name: string;
  rate: number;
  hours: number;
  billable: boolean;
  amount: number;
}

/* Minimal shape of GET /api/app/reporting/summary this report consumes. */
interface LiveSummary {
  perUser?: MinutesRow[];
  perProject?: { projectId: string; trackedMinutes: number }[];
}

/** "45" / "37.5" — rates keep their cents, whole rates read as before. */
const rateLabel = (n: number) => n.toLocaleString("en", { maximumFractionDigits: 2 });

export function PayrollReport() {
  const { isLive } = useSession();
  const [rangeKey, setRangeKey] = useState<RangeKey>("month");
  const [range, setRange] = useState<ResolvedRange>(() => resolveRange("month"));
  const [memberId, setMemberId] = useState("all");
  const [viewMode, setViewMode] = useState<"table" | "grid">("table");

  /*
   * LIVE: pay = SUM over projects of (the member's hourly rate on that project ×
   * their hours on that project). One summary for the range gives each member's
   * total and the projects with tracked time; per such project we read its
   * memberships (rates) and its own per-member minutes. A selected member is
   * filtered server-side (UserId).
   */
  const loadLive = useCallback(async (): Promise<PayrollLine[]> => {
    const scope = { from: range.from, to: range.to, userId: memberId };
    const summary = (await getApi(`/api/app/reporting/summary?${reportQuery(scope)}`)) as LiveSummary | null;
    const tracked = asArray<{ projectId: string; trackedMinutes: number }>(summary?.perProject).filter(
      (p) => p.trackedMinutes > 0,
    );
    const perProject = await Promise.all(
      tracked.map(async (p) => {
        const [members, projectSummary] = await Promise.all([
          getApi(`/api/app/team/project-members/${encodeURIComponent(p.projectId)}`),
          getApi(`/api/app/reporting/summary?${reportQuery({ ...scope, projectId: p.projectId })}`),
        ]);
        return {
          projectId: p.projectId,
          rates: ratesByMember(asArray<MemberRate>(members)),
          perUser: asArray<MinutesRow>((projectSummary as LiveSummary | null)?.perUser),
        };
      }),
    );
    return computePayroll(asArray<MinutesRow>(summary?.perUser), perProject);
  }, [range, memberId]);

  const live = useLiveReport(isLive, loadLive);

  const liveRows = useMemo<Row[]>(
    () =>
      (live.data ?? [])
        .filter((l) => memberId === "all" || l.userId === memberId)
        .map((l) => ({
          userId: l.userId,
          name: memberName(l.userId, userById(l.userId)),
          rate: l.rate,
          hours: l.hours,
          billable: l.amount > 0,
          amount: l.amount,
        })),
    [live.data, memberId],
  );

  const mockRows = useMemo<Row[]>(() => {
    const acts = filterActivitiesByRange(activities, range.from, range.to);
    return billing
      .filter((b) => memberId === "all" || b.userId === memberId)
      .map((b) => {
        const u = userById(b.userId)!;
        const minutes = minutesForUser(acts, b.userId);
        const hours = Math.round((minutes / 60) * 10) / 10;
        return {
          userId: b.userId,
          name: u.name,
          rate: b.rate,
          hours,
          billable: b.billable,
          amount: Math.round(hours * b.rate),
        };
      });
  }, [range, memberId]);

  // A live session shows only real rows (or nothing) — never the demo billing sheet.
  const rows = isLive ? liveRows : mockRows;

  const totalHours = Math.round(rows.reduce((s, r) => s + r.hours, 0) * 100) / 100;
  const totalPayable = Math.round(rows.reduce((s, r) => s + r.amount, 0) * 100) / 100;
  const billableAmount = Math.round(rows.filter((r) => r.billable).reduce((s, r) => s + r.amount, 0) * 100) / 100;
  const avgRate = isLive
    ? totalHours > 0
      ? Math.round((totalPayable / totalHours) * 100) / 100
      : 0
    : rows.length
      ? Math.round(rows.reduce((s, r) => s + r.rate, 0) / rows.length)
      : 0;

  const columns: Column<Row>[] = [
    { key: "name", header: "Member", sortValue: (r) => r.name, render: (r) => (
      <div className="flex items-center gap-2.5">
        <Avatar name={r.name} size="sm" status={userById(r.userId)?.status} />
        <div>
          <div className="font-medium">{r.name}</div>
          <div className="text-xs text-muted-foreground">{userById(r.userId)?.designation}</div>
        </div>
      </div>
    )},
    { key: "rate", header: isLive ? "Blended rate" : "Rate", align: "right", sortValue: (r) => r.rate, render: (r) => (r.rate > 0 ? `$${rateLabel(r.rate)}/hr` : "—") },
    { key: "hours", header: "Hours", align: "right", sortValue: (r) => r.hours, render: (r) => <span className="font-medium">{r.hours}h</span> },
    { key: "billable", header: "Type", sortValue: (r) => (r.billable ? 1 : 0), render: (r) => <Badge tone={r.billable ? "success" : "muted"}>{r.billable ? "Billable" : "Internal"}</Badge> },
    { key: "amount", header: "Amount", align: "right", sortValue: (r) => r.amount, render: (r) => <span className="font-semibold">${r.amount.toLocaleString()}</span> },
  ];

  function onRange(r: ResolvedRange) { setRange(r); setRangeKey(r.key); }
  function doExport() {
    exportRecords(`payroll-${range.key}`, rows, [
      { header: "Member", value: (r) => r.name },
      { header: isLive ? "Blended rate USD/hr" : "Rate USD/hr", value: (r) => r.rate },
      { header: "Hours", value: (r) => r.hours },
      { header: "Billable", value: (r) => (r.billable ? "Yes" : "No") },
      { header: "Amount USD", value: (r) => r.amount },
    ]);
  }

  return (
    <ReportShell
      title="Payroll & Billing"
      description="Billable hours, rates, and payable amounts per member."
      actions={<ExportMenu onExportCSV={doExport} />}
    >
      <FilterBar rangeKey={rangeKey} onRange={onRange} memberId={memberId} onMember={setMemberId} />

      {isLive && <LiveReportNotice loading={live.loading} error={live.error} onRetry={live.retry} />}

      <KpiGrid>
        <Kpi label="Total payable" value={`$${totalPayable.toLocaleString()}`} icon={Wallet} tone="#22c55e" sub={range.label} />
        <Kpi label="Billable amount" value={`$${billableAmount.toLocaleString()}`} icon={Receipt} tone="#6d5efc" />
        <Kpi label="Total hours" value={`${totalHours}h`} icon={Clock} tone="#0ea5e9" />
        <Kpi label={isLive ? "Blended rate" : "Avg rate"} value={`$${rateLabel(avgRate)}/hr`} icon={DollarSign} tone="#f59e0b" />
      </KpiGrid>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <div>
            <CardTitle>Payroll details</CardTitle>
            <span className="text-xs text-muted-foreground">Estimated for {range.label.toLowerCase()}</span>
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
            // "scroll" keeps the totals row inside <tfoot>: the card stack would
            // render these <td> cells inside a <div>, which is invalid nesting
            // (React's "<td> cannot be a child of <div>" hydration error).
            <DataTable
              columns={columns}
              rows={rows}
              initialSort={{ key: "amount", dir: "desc" }}
              mobileLayout="scroll"
              emptyText={isLive ? "No tracked time in this range." : undefined}
              footer={
                <>
                  <td className="sticky left-0 z-10 bg-card px-3 py-2.5">Total</td>
                  <td className="px-3 py-2.5 text-right text-muted-foreground">—</td>
                  <td className="px-3 py-2.5 text-right">{totalHours}h</td>
                  <td className="px-3 py-2.5" />
                  <td className="px-3 py-2.5 text-right">${totalPayable.toLocaleString()}</td>
                </>
              }
            />
          ) : (
            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {rows.map((r) => {
                const u = userById(r.userId);
                return (
                  <Card key={r.userId} className="p-4 hover:border-primary/40 transition-colors">
                    <div className="flex items-center gap-3">
                      <Avatar name={r.name} size="md" status={u?.status} />
                      <div className="min-w-0 flex-1">
                        <h4 className="font-semibold truncate text-sm">{r.name}</h4>
                        <p className="text-xs text-muted-foreground truncate">{u?.designation ?? "Team Member"}</p>
                      </div>
                    </div>

                    <div className="mt-4 grid grid-cols-2 gap-2 border-t border-border/60 pt-3 text-xs">
                      <div>
                        <span className="text-muted-foreground">{isLive ? "Blended Rate" : "Hourly Rate"}</span>
                        <div className="text-sm font-bold text-foreground mt-0.5">{r.rate > 0 ? `$${rateLabel(r.rate)}/hr` : "—"}</div>
                      </div>
                      <div>
                        <span className="text-muted-foreground">Hours Worked</span>
                        <div className="text-sm font-bold text-foreground mt-0.5">{r.hours}h</div>
                      </div>
                    </div>

                    <div className="mt-4 flex items-center justify-between text-xs border-t border-border/60 pt-3">
                      <span className="text-muted-foreground">Classification</span>
                      <Badge tone={r.billable ? "success" : "muted"}>{r.billable ? "Billable" : "Internal"}</Badge>
                    </div>

                    <div className="mt-3 flex items-center justify-between text-xs border-t border-border/60 pt-3 bg-muted/40 p-2 rounded-lg">
                      <span className="font-medium text-muted-foreground">Total Earnings</span>
                      <span className="text-base font-bold text-success">${r.amount.toLocaleString()}</span>
                    </div>
                  </Card>
                );
              })}
            </div>
          )}
        </CardContent>
      </Card>

      <p className="text-xs text-muted-foreground">
        {isLive
          ? "Pay is each member's hours on a project × their hourly rate on that project, summed across projects; the rate shown is the blended result (pay ÷ hours). Hours on projects without a rate are not paid."
          : "Amounts are estimates based on tracked time and configured rates. Connect the backend to generate finalized invoices."}
      </p>
    </ReportShell>
  );
}
