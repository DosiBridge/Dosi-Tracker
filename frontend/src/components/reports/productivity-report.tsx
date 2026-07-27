"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
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
import { Gauge, TrendingUp, Minus, TrendingDown } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid } from "./report-shell";
import { activities, projects, userById, users } from "@/lib/tenant-data";
import { categoryColor, filterActivitiesByRange, rangeForKey, type RangeKey } from "@/lib/reports-data";
import { exportRecords } from "@/lib/export";
import { cn, formatDuration } from "@/lib/utils";
import { getApi } from "@/hooks/useApi";
import type { ResolvedRange } from "./date-range-picker";

/* ---- Live (API) shapes: /api/app/reporting/summary + /api/identity/users ---- */

interface LivePerUser {
  userId: string;
  activityCount: number;
  trackedMinutes: number;
  averageProductivity: number;
}

interface LiveSummary {
  perUser: LivePerUser[];
}

interface LiveIdentityUser {
  id: string;
  userName: string;
  name?: string | null;
  surname?: string | null;
}

const GUID_RE = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;

/**
 * Live queries pivot on the real clock (the shared range presets pivot on the demo NOW constant so
 * mock data stays stable). Mirrors rangeForKey's per-preset logic.
 */
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

/**
 * Split a member's tracked minutes into productive / neutral / unproductive / idle from the backend's
 * per-user tracked-minutes + average productivity. Kept identical to the demo derivation so live and
 * demo modes read the same way.
 */
function splitFor(trackedMinutes: number, averageProductivity: number) {
  const productive = Math.round((trackedMinutes * averageProductivity) / 100);
  const rem = Math.max(trackedMinutes - productive, 0);
  const unproductive = Math.round(rem * 0.35);
  const neutral = rem - unproductive;
  const idle = Math.round(trackedMinutes * 0.08);
  const total = productive + neutral + unproductive;
  const focus = total ? Math.round((productive / total) * 100) : 0;
  return { productive, neutral, unproductive, idle, focus };
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
  const [rangeKey, setRangeKey] = useState<RangeKey>("7d");
  const [range, setRange] = useState<ResolvedRange>(() => ({ key: "7d", ...rangeForKey("7d") }));
  const [memberId, setMemberId] = useState("all");
  const [viewMode, setViewMode] = useState<"table" | "grid">("table");

  const [liveSummary, setLiveSummary] = useState<LiveSummary | null>(null);
  const [liveUsers, setLiveUsers] = useState<LiveIdentityUser[]>([]);
  const [liveLoading, setLiveLoading] = useState(false);
  const [liveError, setLiveError] = useState(false);

  // The backend can only key off real GUIDs; a demo (non-GUID) member selection keeps the mock render.
  const liveApplicable = memberId === "all" || GUID_RE.test(memberId);

  const loadLive = useCallback(async () => {
    setLiveLoading(true);
    setLiveError(false);
    try {
      const { from, to } = liveRangeFor(range.key);
      const qs = new URLSearchParams({ From: from.toISOString(), To: to.toISOString() });
      const summary = await getApi(`/api/app/reporting/summary?${qs.toString()}`);
      setLiveSummary(summary && typeof summary === "object" ? (summary as LiveSummary) : null);
      // Best effort: admins resolve member names; workers get a 403 -> keep ids.
      const identityUsers = await getApi("/api/identity/users?MaxResultCount=100").catch(() => []);
      setLiveUsers(Array.isArray(identityUsers) ? (identityUsers as LiveIdentityUser[]) : []);
    } catch {
      setLiveSummary(null); // fall back to the demo dataset below
      setLiveError(true);
    } finally {
      setLiveLoading(false);
    }
  }, [range]);

  useEffect(() => {
    if (typeof window === "undefined" || !localStorage.getItem("dosi-token")) return;
    if (!liveApplicable) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    loadLive();
  }, [loadLive, liveApplicable]);

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
        const p = projects.find((pr) => pr.id === a.projectId);
        const mins = p?.intervalMinutes ?? 10;
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

  const liveRows = useMemo<Row[] | null>(() => {
    if (!liveApplicable || !liveSummary) return null;
    const perUserList = Array.isArray(liveSummary.perUser) ? liveSummary.perUser : [];
    const filtered = memberId === "all" ? perUserList : perUserList.filter((u) => u.userId === memberId);
    return filtered.map((u) => {
      const identity = liveUsers.find((x) => x.id === u.userId);
      const identityName = identity ? `${identity.name ?? ""} ${identity.surname ?? ""}`.trim() : "";
      const mockUser = userById(u.userId);
      return {
        userId: u.userId,
        name: identityName || identity?.userName || mockUser?.name || u.userId.slice(0, 8),
        ...splitFor(u.trackedMinutes, u.averageProductivity),
      };
    });
  }, [liveApplicable, liveSummary, liveUsers, memberId]);

  const rows = liveRows ?? mockRows;
  const showLiveLoading = liveApplicable && liveLoading && liveRows === null;
  const showLiveError = liveApplicable && liveError && liveRows === null;

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
  const focusPct = grand ? Math.round((totals.productive / grand) * 100) : 0;

  const donutData = [
    { name: "Productive", value: totals.productive, color: categoryColor.productive },
    { name: "Neutral", value: totals.neutral, color: categoryColor.neutral },
    { name: "Unproductive", value: totals.unproductive, color: categoryColor.unproductive },
  ];

  const barData = rows.map((r) => ({ ...r, name: r.name.split(" ")[0] }));

  const columns: Column<Row>[] = [
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
    { key: "neutral", header: "Neutral", align: "right", sortValue: (r) => r.neutral, render: (r) => <span className="text-info">{formatDuration(r.neutral)}</span> },
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

  function onRange(r: ResolvedRange) {
    setRange(r);
    setRangeKey(r.key);
  }
  function doExport() {
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
        <Kpi label="Team focus" value={`${focusPct}%`} icon={Gauge} tone="#6d5efc" sub={range.label} />
        <Kpi label="Productive" value={formatDuration(totals.productive)} icon={TrendingUp} tone="#22c55e" />
        <Kpi label="Neutral" value={formatDuration(totals.neutral)} icon={Minus} tone="#0ea5e9" />
        <Kpi label="Unproductive" value={formatDuration(totals.unproductive)} icon={TrendingDown} tone="#ef4444" />
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
                  <span className="font-medium">{Math.round((d.value / grand) * 100)}%</span>
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
                  <Bar dataKey="neutral" stackId="a" fill={categoryColor.neutral} barSize={22} />
                  <Bar dataKey="unproductive" stackId="a" fill={categoryColor.unproductive} radius={[4, 4, 0, 0]} barSize={22} />
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
            <DataTable columns={columns} rows={rows} initialSort={{ key: "focus", dir: "desc" }} />
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
                          <div className="bg-success transition-all" style={{ width: `${sumTotal ? Math.round((r.productive / sumTotal) * 100) : 0}%` }} title={`Productive: ${formatDuration(r.productive)}`} />
                          <div className="bg-info transition-all" style={{ width: `${sumTotal ? Math.round((r.neutral / sumTotal) * 100) : 0}%` }} title={`Neutral: ${formatDuration(r.neutral)}`} />
                          <div className="bg-danger transition-all" style={{ width: `${sumTotal ? Math.round((r.unproductive / sumTotal) * 100) : 0}%` }} title={`Unproductive: ${formatDuration(r.unproductive)}`} />
                        </div>
                      </div>
                    </div>

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
                  </Card>
                );
              })}
            </div>
          )}
        </CardContent>
      </Card>
    </ReportShell>
  );
}
