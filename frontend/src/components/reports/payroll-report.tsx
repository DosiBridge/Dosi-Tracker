"use client";

import { useEffect, useMemo, useState } from "react";
import { DollarSign, Clock, Wallet, Receipt } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid } from "./report-shell";
import { activities, billing, projects, userById, users } from "@/lib/tenant-data";
import { filterActivitiesByRange, rangeForKey, type RangeKey } from "@/lib/reports-data";
import { exportRecords } from "@/lib/export";
import { cn, formatDuration } from "@/lib/utils";
import { getApi } from "@/hooks/useApi";
import type { ResolvedRange } from "./date-range-picker";

interface Row {
  userId: string;
  name: string;
  rate: number;
  hours: number;
  billable: boolean;
  amount: number;
}

/* Minimal shapes of the live backend payloads this report consumes. */
interface LiveSummaryUser {
  userId: string;
  activityCount: number;
  trackedMinutes: number;
  averageProductivity: number;
}
interface LiveProject {
  id: string;
  title: string;
}
interface LiveMember {
  id: string;
  projectId: string;
  userId: string;
  role: string;
  hourlyRate: number | null;
}
interface LiveIdentityUser {
  id: string;
  userName: string;
  name?: string | null;
  surname?: string | null;
}

/**
 * Real-clock bounds for a preset (the shared presets pivot on the frozen demo NOW).
 * A custom range carries the user's explicit dates, so it passes through unchanged.
 */
function liveRangeFor(key: RangeKey, custom?: { from: Date; to: Date }): { from: Date; to: Date } {
  if (key === "custom" && custom) return custom;
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
    default: // "7d"
      from.setDate(from.getDate() - 7);
      break;
  }
  return { from, to };
}

export function PayrollReport() {
  const [rangeKey, setRangeKey] = useState<RangeKey>("month");
  const [range, setRange] = useState<ResolvedRange>(() => ({ key: "month", ...rangeForKey("month") }));
  const [memberId, setMemberId] = useState("all");
  const [viewMode, setViewMode] = useState<"table" | "grid">("table");

  // LIVE MODE: only active when a real backend session token exists.
  const [isLive, setIsLive] = useState(false);
  const [liveRows, setLiveRows] = useState<Row[] | null>(null);
  const [liveLoading, setLiveLoading] = useState(false);
  const [liveError, setLiveError] = useState<string | null>(null);

  useEffect(() => {
    if (typeof window !== "undefined" && localStorage.getItem("dosi-token")) setIsLive(true);
  }, []);

  useEffect(() => {
    if (!isLive) return;
    let cancelled = false;
    setLiveLoading(true);
    setLiveError(null);
    (async () => {
      try {
        // Live data is on the real clock; the shared range presets pivot on the frozen demo NOW,
        // so re-anchor the query window to the real clock (custom ranges pass through).
        const live = liveRangeFor(range.key, { from: range.from, to: range.to });
        const qs = `From=${encodeURIComponent(live.from.toISOString())}&To=${encodeURIComponent(live.to.toISOString())}`;
        const [summaryRaw, projectsRaw, identityRaw] = await Promise.all([
          getApi(`/api/app/reporting/summary?${qs}`),
          getApi("/api/app/project").catch(() => []),
          // Admin-only endpoint; workers get 403 -> just resolve names from mock data instead.
          getApi("/api/identity/users?MaxResultCount=100").catch(() => []),
        ]);
        const perUser = (summaryRaw?.perUser ?? []) as LiveSummaryUser[];
        const liveProjects = (projectsRaw ?? []) as LiveProject[];
        const identityUsers = (identityRaw ?? []) as LiveIdentityUser[];

        const memberLists = await Promise.all(
          liveProjects.map((p) => getApi(`/api/app/team/project-members?projectId=${p.id}`).catch(() => []))
        );

        // A user's rate = the max hourlyRate across all their project memberships.
        const rateByUser = new Map<string, number>();
        for (const list of memberLists) {
          for (const m of (list ?? []) as LiveMember[]) {
            const rate = typeof m.hourlyRate === "number" ? m.hourlyRate : 0;
            rateByUser.set(m.userId, Math.max(rateByUser.get(m.userId) ?? 0, rate));
          }
        }

        const nameByUser = new Map<string, string>();
        for (const u of identityUsers) {
          const full = [u.name, u.surname].filter(Boolean).join(" ").trim();
          nameByUser.set(u.id, full || u.userName);
        }

        const next: Row[] = perUser.map((u) => {
          const rate = rateByUser.get(u.userId) ?? 0;
          const hours = Math.round((u.trackedMinutes / 60) * 100) / 100;
          return {
            userId: u.userId,
            name: nameByUser.get(u.userId) ?? userById(u.userId)?.name ?? `Member ${u.userId.slice(0, 8)}`,
            rate,
            hours,
            billable: rate > 0,
            amount: Math.round(hours * rate * 100) / 100,
          };
        });
        if (!cancelled) {
          setLiveRows(next);
          setLiveLoading(false);
        }
      } catch (err) {
        if (!cancelled) {
          setLiveRows(null);
          setLiveError(err instanceof Error ? err.message : "Failed to load live data");
          setLiveLoading(false);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isLive, range]);

  const mockRows = useMemo<Row[]>(() => {
    const acts = filterActivitiesByRange(activities, range.from, range.to);
    return billing
      .filter((b) => memberId === "all" || b.userId === memberId)
      .map((b) => {
        const u = userById(b.userId)!;
        const ua = acts.filter((a) => a.userId === b.userId);
        const minutes = ua.reduce((s, a) => {
          const p = projects.find((pr) => pr.id === a.projectId);
          return s + (p?.intervalMinutes ?? 10);
        }, 0);
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

  // Prefer live data when available; fall back to the mock rows otherwise.
  const rows = useMemo<Row[]>(() => {
    if (isLive && liveRows) {
      return memberId === "all" ? liveRows : liveRows.filter((r) => r.userId === memberId);
    }
    return mockRows;
  }, [isLive, liveRows, memberId, mockRows]);

  const totalHours = Math.round(rows.reduce((s, r) => s + r.hours, 0) * 100) / 100;
  const totalPayable = Math.round(rows.reduce((s, r) => s + r.amount, 0) * 100) / 100;
  const billableAmount = Math.round(rows.filter((r) => r.billable).reduce((s, r) => s + r.amount, 0) * 100) / 100;
  const avgRate = rows.length ? Math.round(rows.reduce((s, r) => s + r.rate, 0) / rows.length) : 0;

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
    { key: "rate", header: "Rate", align: "right", sortValue: (r) => r.rate, render: (r) => (r.rate > 0 ? `$${r.rate}/hr` : "—") },
    { key: "hours", header: "Hours", align: "right", sortValue: (r) => r.hours, render: (r) => <span className="font-medium">{r.hours}h</span> },
    { key: "billable", header: "Type", sortValue: (r) => (r.billable ? 1 : 0), render: (r) => <Badge tone={r.billable ? "success" : "muted"}>{r.billable ? "Billable" : "Internal"}</Badge> },
    { key: "amount", header: "Amount", align: "right", sortValue: (r) => r.amount, render: (r) => <span className="font-semibold">${r.amount.toLocaleString()}</span> },
  ];

  function onRange(r: ResolvedRange) { setRange(r); setRangeKey(r.key); }
  function doExport() {
    exportRecords(`payroll-${range.key}`, rows, [
      { header: "Member", value: (r) => r.name },
      { header: "Rate USD/hr", value: (r) => r.rate },
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

      {isLive && liveLoading && (
        <p className="text-xs text-muted-foreground animate-pulse">Loading live payroll data…</p>
      )}
      {isLive && !liveLoading && liveError && (
        <p className="text-xs text-danger">Couldn&apos;t load live payroll data — showing demo data.</p>
      )}

      <KpiGrid>
        <Kpi label="Total payable" value={`$${totalPayable.toLocaleString()}`} icon={Wallet} tone="#22c55e" sub={range.label} />
        <Kpi label="Billable amount" value={`$${billableAmount.toLocaleString()}`} icon={Receipt} tone="#6d5efc" />
        <Kpi label="Total hours" value={`${totalHours}h`} icon={Clock} tone="#0ea5e9" />
        <Kpi label="Avg rate" value={`$${avgRate}/hr`} icon={DollarSign} tone="#f59e0b" />
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
            <DataTable
              columns={columns}
              rows={rows}
              initialSort={{ key: "amount", dir: "desc" }}
              footer={
                <>
                  <td className="px-3 py-2.5">Total</td>
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
                        <span className="text-muted-foreground">Hourly Rate</span>
                        <div className="text-sm font-bold text-foreground mt-0.5">{r.rate > 0 ? `$${r.rate}/hr` : "—"}</div>
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
        {isLive && liveRows && !liveError
          ? "Amounts are computed from tracked time and each member's configured hourly rate."
          : "Amounts are estimates based on tracked time and configured rates. Connect the backend to generate finalized invoices."}
      </p>
    </ReportShell>
  );
}
