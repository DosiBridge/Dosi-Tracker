"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { CalendarCheck, Clock, UserCheck, UserX } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid } from "./report-shell";
import { attendance, userById, users } from "@/lib/tenant-data";
import { trackedMembers } from "@/lib/roles";
import { rangeForKey, type RangeKey } from "@/lib/reports-data";
import { exportRecords } from "@/lib/export";
import { formatDuration, cn } from "@/lib/utils";
import { getApi } from "@/hooks/useApi";
import type { ResolvedRange } from "./date-range-picker";

/**
 * Presence & worked-time, derived honestly from tracked activity. The tracker does not record
 * shifts, clock-in/out, lateness or on-site/remote, so those concepts are intentionally absent —
 * a member is "present" on a day they tracked any activity, "absent" on a working day they didn't.
 */

interface Row {
  userId: string;
  name: string;
  present: number;
  absent: number;
  worked: number; // minutes
  presentDays: Set<string>;
}

interface LiveRow {
  userId: string;
  daysPresent: number;
  daysAbsent: number;
  workedMinutes: number;
  presentDays: string[]; // ISO datetimes
}
interface LiveAttendance {
  days: string[];
  rows: LiveRow[];
}
interface LiveIdentityUser {
  id: string;
  userName: string;
  name?: string | null;
  surname?: string | null;
}

const dayKey = (iso: string) => iso.slice(0, 10);

function liveRangeFor(key: RangeKey): { from: Date; to: Date } {
  const to = new Date();
  const from = new Date();
  switch (key) {
    case "today": from.setHours(0, 0, 0, 0); break;
    case "yesterday":
      from.setDate(from.getDate() - 1); from.setHours(0, 0, 0, 0);
      to.setDate(to.getDate() - 1); to.setHours(23, 59, 59, 999); break;
    case "30d": from.setDate(from.getDate() - 30); break;
    case "month": from.setDate(1); from.setHours(0, 0, 0, 0); break;
    default: from.setDate(from.getDate() - 7); break;
  }
  return { from, to };
}

export function AttendanceReport() {
  const [rangeKey, setRangeKey] = useState<RangeKey>("7d");
  const [range, setRange] = useState<ResolvedRange>(() => ({ key: "7d", ...rangeForKey("7d") }));
  const [memberId, setMemberId] = useState("all");

  const [live, setLive] = useState<LiveAttendance | null>(null);
  const [liveUsers, setLiveUsers] = useState<LiveIdentityUser[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);

  const loadLive = useCallback(async () => {
    setLoading(true);
    setError(false);
    try {
      const { from, to } = liveRangeFor(range.key);
      const qs = new URLSearchParams({ From: from.toISOString(), To: to.toISOString() });
      const report = (await getApi(`/api/app/reporting/attendance?${qs.toString()}`)) as LiveAttendance;
      setLive(report && Array.isArray(report.rows) ? report : { days: [], rows: [] });
      const identity = await getApi("/api/identity/users?MaxResultCount=200").catch(() => []);
      setLiveUsers(Array.isArray(identity) ? (identity as LiveIdentityUser[]) : []);
    } catch {
      setLive(null);
      setError(true);
    } finally {
      setLoading(false);
    }
  }, [range]);

  useEffect(() => {
    if (typeof window === "undefined" || !localStorage.getItem("dosi-token")) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    loadLive();
  }, [loadLive]);

  // --- Demo fallback (mock attendance): present = any non-absent day. ---
  const demoDates = useMemo(() => {
    const from = range.from.toISOString().slice(0, 10);
    const to = range.to.toISOString().slice(0, 10);
    const inRange = attendance.filter((a) => a.date >= from && a.date <= to);
    return [...new Set(inRange.map((a) => a.date))].sort();
  }, [range]);

  const demoRows = useMemo<Row[]>(() => {
    const from = range.from.toISOString().slice(0, 10);
    const to = range.to.toISOString().slice(0, 10);
    const inRange = attendance.filter((a) => a.date >= from && a.date <= to);
    return trackedMembers(users)
      .filter((u) => memberId === "all" || u.id === memberId)
      .map((u) => {
        const ua = inRange.filter((a) => a.userId === u.id);
        const presentDays = new Set(ua.filter((a) => a.status !== "absent").map((a) => a.date));
        return {
          userId: u.id,
          name: u.name,
          present: presentDays.size,
          absent: ua.filter((a) => a.status === "absent").length,
          worked: ua.reduce((s, a) => s + a.worked, 0),
          presentDays,
        };
      });
  }, [range, memberId]);

  const liveRows = useMemo<Row[] | null>(() => {
    if (!live) return null;
    return live.rows
      .filter((r) => memberId === "all" || r.userId === memberId)
      .map((r) => {
        const id = liveUsers.find((x) => x.id === r.userId);
        const nm = id ? `${id.name ?? ""} ${id.surname ?? ""}`.trim() : "";
        return {
          userId: r.userId,
          name: nm || id?.userName || userById(r.userId)?.name || r.userId.slice(0, 8),
          present: r.daysPresent,
          absent: r.daysAbsent,
          worked: Math.round(r.workedMinutes),
          presentDays: new Set(r.presentDays.map(dayKey)),
        };
      });
  }, [live, liveUsers, memberId]);

  const rows = liveRows ?? demoRows;
  const dates = live ? live.days.map(dayKey) : demoDates;
  const showLoading = loading && liveRows === null;
  const showError = error && liveRows === null;

  const totalPresent = rows.reduce((s, r) => s + r.present, 0);
  const totalAbsent = rows.reduce((s, r) => s + r.absent, 0);
  const totalWorked = rows.reduce((s, r) => s + r.worked, 0);
  const attendanceRate = totalPresent + totalAbsent ? Math.round((totalPresent / (totalPresent + totalAbsent)) * 100) : 0;

  const columns: Column<Row>[] = [
    { key: "name", header: "Member", sortValue: (r) => r.name, render: (r) => (
      <div className="flex items-center gap-2.5">
        <Avatar name={r.name} size="sm" status={userById(r.userId)?.status} />
        <span className="font-medium">{r.name}</span>
      </div>
    )},
    { key: "present", header: "Present", align: "right", sortValue: (r) => r.present, render: (r) => <span className="text-success">{r.present}</span> },
    { key: "absent", header: "Absent", align: "right", sortValue: (r) => r.absent, render: (r) => <span className="text-danger">{r.absent}</span> },
    { key: "worked", header: "Total worked", align: "right", sortValue: (r) => r.worked, render: (r) => <span className="font-medium">{formatDuration(r.worked)}</span> },
  ];

  function onRange(r: ResolvedRange) { setRange(r); setRangeKey(r.key); }
  function doExport() {
    exportRecords(`attendance-${range.key}`, rows, [
      { header: "Member", value: (r) => r.name },
      { header: "Present days", value: (r) => r.present },
      { header: "Absent days", value: (r) => r.absent },
      { header: "Total worked (min)", value: (r) => r.worked },
    ]);
  }

  return (
    <ReportShell
      title="Attendance"
      description="Presence and worked time, derived from tracked activity per working day."
      actions={<ExportMenu onExportCSV={doExport} />}
    >
      <FilterBar rangeKey={rangeKey} onRange={onRange} memberId={memberId} onMember={setMemberId} />

      {showLoading && (
        <div className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">Loading live report data…</div>
      )}
      {showError && (
        <div className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">Live data unavailable right now — showing demo data.</div>
      )}

      <KpiGrid>
        <Kpi label="Attendance rate" value={`${attendanceRate}%`} icon={CalendarCheck} tone="#22c55e" sub={range.label} />
        <Kpi label="Present days" value={String(totalPresent)} icon={UserCheck} tone="#6d5efc" />
        <Kpi label="Absent days" value={String(totalAbsent)} icon={UserX} tone="#ef4444" />
        <Kpi label="Total worked" value={formatDuration(totalWorked)} icon={Clock} tone="#0ea5e9" />
      </KpiGrid>

      <Card>
        <CardHeader>
          <CardTitle>Daily presence</CardTitle>
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 text-xs text-muted-foreground">
            <span className="flex items-center gap-1.5"><span className="h-2.5 w-2.5 rounded bg-success/80" /> Present</span>
            <span className="flex items-center gap-1.5"><span className="h-2.5 w-2.5 rounded bg-danger/60" /> Absent</span>
          </div>
        </CardHeader>
        <CardContent>
          <div className="overflow-x-auto overscroll-x-contain">
            <div className="min-w-[640px]">
              <div className="grid gap-2" style={{ gridTemplateColumns: `180px repeat(${dates.length}, minmax(40px, 1fr))` }}>
                <div className="sticky left-0 z-10 bg-card text-xs font-medium text-muted-foreground">Member</div>
                {dates.map((d) => (
                  <div key={d} className="text-center text-xs font-medium text-muted-foreground">
                    {new Date(d + "T00:00:00Z").toLocaleDateString("en", { weekday: "short", timeZone: "UTC" })}
                  </div>
                ))}
                {rows.map((r) => (
                  <div key={r.userId} className="contents">
                    <div className="sticky left-0 z-10 flex items-center gap-2 bg-card py-1">
                      <Avatar name={r.name} size="sm" />
                      <span className="truncate text-sm font-medium">{r.name.split(" ")[0]}</span>
                    </div>
                    {dates.map((d) => {
                      const present = r.presentDays.has(d);
                      return (
                        <div
                          key={d}
                          title={`${r.name} — ${present ? "Present" : "Absent"} (${d})`}
                          className={cn("flex h-9 items-center justify-center rounded-lg text-[10px] font-semibold text-white", present ? "bg-success/80" : "bg-danger/50")}
                        >
                          {present ? "✓" : "—"}
                        </div>
                      );
                    })}
                  </div>
                ))}
              </div>
            </div>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader><CardTitle>Summary</CardTitle></CardHeader>
        <CardContent>
          <DataTable columns={columns} rows={rows} initialSort={{ key: "worked", dir: "desc" }} />
        </CardContent>
      </Card>
    </ReportShell>
  );
}
