"use client";

import { useMemo, useState } from "react";
import { CalendarCheck, Clock, UserCheck, UserX } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid } from "./report-shell";
import { attendance, attendanceSummary, userById, users } from "@/lib/tenant-data";
import { trackedMembers } from "@/lib/roles";
import { rangeForKey, type AttendanceStatus, type RangeKey } from "@/lib/reports-data";
import { exportRecords } from "@/lib/export";
import { formatDuration } from "@/lib/utils";
import { cn } from "@/lib/utils";
import type { ResolvedRange } from "./date-range-picker";

const statusMeta: Record<AttendanceStatus, { label: string; cell: string; tone: "success" | "warning" | "danger" | "info" }> = {
  present: { label: "Present", cell: "bg-success/80 text-white", tone: "success" },
  late: { label: "Late", cell: "bg-warning/80 text-white", tone: "warning" },
  absent: { label: "Absent", cell: "bg-danger/70 text-white", tone: "danger" },
  remote: { label: "Remote", cell: "bg-info/70 text-white", tone: "info" },
};

interface Row {
  userId: string;
  name: string;
  present: number;
  late: number;
  absent: number;
  remote: number;
  worked: number;
}

export function AttendanceReport() {
  const [rangeKey, setRangeKey] = useState<RangeKey>("7d");
  const [range, setRange] = useState<ResolvedRange>(() => ({ key: "7d", ...rangeForKey("7d") }));
  const [memberId, setMemberId] = useState("all");

  const filteredAttendance = useMemo(() => {
    const fromStr = range.from.toISOString().slice(0, 10);
    const toStr = range.to.toISOString().slice(0, 10);
    return attendance.filter((a) => a.date >= fromStr && a.date <= toStr);
  }, [range]);

  const dates = useMemo(() => [...new Set(filteredAttendance.map((a) => a.date))].sort(), [filteredAttendance]);
  const members = trackedMembers(users).filter((u) => memberId === "all" || u.id === memberId);

  const rows = useMemo<Row[]>(
    () =>
      members.map((u) => {
        const ua = filteredAttendance.filter((a) => a.userId === u.id);
        return {
          userId: u.id,
          name: u.name,
          present: ua.filter((a) => a.status === "present").length,
          late: ua.filter((a) => a.status === "late").length,
          absent: ua.filter((a) => a.status === "absent").length,
          remote: ua.filter((a) => a.status === "remote").length,
          worked: ua.reduce((s, a) => s + a.worked, 0),
        };
      }),
    [members, filteredAttendance]
  );

  const s = useMemo(() => {
    const counts = { present: 0, late: 0, absent: 0, remote: 0 };
    for (const r of filteredAttendance) counts[r.status]++;
    return counts;
  }, [filteredAttendance]);

  const totalDays = s.present + s.late + s.absent + s.remote;
  const attendanceRate = totalDays ? Math.round(((s.present + s.late + s.remote) / totalDays) * 100) : 0;
  const onTimeRate = totalDays ? Math.round(((s.present + s.remote) / totalDays) * 100) : 0;

  const columns: Column<Row>[] = [
    { key: "name", header: "Member", sortValue: (r) => r.name, render: (r) => (
      <div className="flex items-center gap-2.5">
        <Avatar name={r.name} size="sm" status={userById(r.userId)?.status} />
        <span className="font-medium">{r.name}</span>
      </div>
    )},
    { key: "present", header: "Present", align: "right", sortValue: (r) => r.present, render: (r) => <span className="text-success">{r.present}</span> },
    { key: "remote", header: "Remote", align: "right", sortValue: (r) => r.remote, render: (r) => <span className="text-info">{r.remote}</span> },
    { key: "late", header: "Late", align: "right", sortValue: (r) => r.late, render: (r) => <span className="text-warning">{r.late}</span> },
    { key: "absent", header: "Absent", align: "right", sortValue: (r) => r.absent, render: (r) => <span className="text-danger">{r.absent}</span> },
    { key: "worked", header: "Total worked", align: "right", sortValue: (r) => r.worked, render: (r) => <span className="font-medium">{formatDuration(r.worked)}</span> },
  ];

  function onRange(r: ResolvedRange) { setRange(r); setRangeKey(r.key); }
  function doExport() {
    exportRecords(`attendance-${range.key}`, rows, [
      { header: "Member", value: (r) => r.name },
      { header: "Present", value: (r) => r.present },
      { header: "Remote", value: (r) => r.remote },
      { header: "Late", value: (r) => r.late },
      { header: "Absent", value: (r) => r.absent },
      { header: "Total worked (min)", value: (r) => r.worked },
    ]);
  }

  return (
    <ReportShell
      title="Attendance & Shifts"
      description="Clock in/out, late arrivals, remote days, and absences."
      actions={<ExportMenu onExportCSV={doExport} />}
    >
      <FilterBar rangeKey={rangeKey} onRange={onRange} memberId={memberId} onMember={setMemberId} />

      <KpiGrid>
        <Kpi label="Attendance rate" value={`${attendanceRate}%`} icon={CalendarCheck} tone="#22c55e" sub={range.label} />
        <Kpi label="On-time rate" value={`${onTimeRate}%`} icon={UserCheck} tone="#6d5efc" />
        <Kpi label="Late arrivals" value={String(s.late)} icon={Clock} tone="#f59e0b" />
        <Kpi label="Absences" value={String(s.absent)} icon={UserX} tone="#ef4444" />
      </KpiGrid>

      {/* Matrix */}
      <Card>
        <CardHeader>
          <CardTitle>Daily attendance</CardTitle>
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 text-xs text-muted-foreground">
            {(Object.keys(statusMeta) as AttendanceStatus[]).map((k) => (
              <span key={k} className="flex items-center gap-1.5">
                <span className={cn("h-2.5 w-2.5 rounded", statusMeta[k].cell)} /> {statusMeta[k].label}
              </span>
            ))}
          </div>
        </CardHeader>
        <CardContent>
          {/* Mobile: chip rows per member */}
          <div className="space-y-3 md:hidden">
            {members.map((u) => (
              <div key={u.id} className="rounded-xl border border-border p-3">
                <div className="mb-2.5 flex items-center gap-2">
                  <Avatar name={u.name} size="sm" />
                  <span className="truncate text-sm font-medium">{u.name}</span>
                </div>
                <div className="flex gap-1.5 overflow-x-auto overscroll-x-contain pb-1">
                  {dates.map((d) => {
                    const rec = filteredAttendance.find((a) => a.userId === u.id && a.date === d);
                    const meta = rec ? statusMeta[rec.status] : null;
                    return (
                      <div
                        key={d}
                        title={rec ? `${meta?.label}${rec.clockIn ? ` · ${rec.clockIn}–${rec.clockOut}` : ""}` : ""}
                        className={cn(
                          "flex h-12 w-11 shrink-0 flex-col items-center justify-center rounded-lg text-[9px] font-semibold",
                          meta ? meta.cell : "bg-muted"
                        )}
                      >
                        <span className="opacity-70">{new Date(d + "T00:00:00Z").toLocaleDateString("en", { weekday: "narrow", timeZone: "UTC" })}</span>
                        <span>{rec?.clockIn?.slice(0, 5) ?? "—"}</span>
                      </div>
                    );
                  })}
                </div>
              </div>
            ))}
          </div>

          {/* Desktop matrix */}
          <div className="hidden overflow-x-auto overscroll-x-contain md:block">
            <div className="min-w-[640px]">
              <div className="grid gap-2" style={{ gridTemplateColumns: `180px repeat(${dates.length}, minmax(48px, 1fr))` }}>
                <div className="sticky left-0 z-10 bg-card text-xs font-medium text-muted-foreground">Member</div>
                {dates.map((d) => (
                  <div key={d} className="text-center text-xs font-medium text-muted-foreground">
                    {new Date(d + "T00:00:00Z").toLocaleDateString("en", { weekday: "short", timeZone: "UTC" })}
                  </div>
                ))}
                {members.map((u) => (
                  <div key={u.id} className="contents">
                    <div className="sticky left-0 z-10 flex items-center gap-2 bg-card py-1">
                      <Avatar name={u.name} size="sm" />
                      <span className="truncate text-sm font-medium">{u.name.split(" ")[0]}</span>
                    </div>
                    {dates.map((d) => {
                      const rec = filteredAttendance.find((a) => a.userId === u.id && a.date === d);
                      const meta = rec ? statusMeta[rec.status] : null;
                      return (
                        <div
                          key={d}
                          title={rec ? `${meta?.label}${rec.clockIn ? ` · ${rec.clockIn}–${rec.clockOut}` : ""}` : ""}
                          className={cn(
                            "flex h-10 items-center justify-center rounded-lg text-[10px] font-semibold",
                            meta ? meta.cell : "bg-muted"
                          )}
                        >
                          {rec?.clockIn ?? "—"}
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
