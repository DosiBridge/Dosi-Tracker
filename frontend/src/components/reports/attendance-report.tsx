"use client";

import { useMemo, useState } from "react";
import { CalendarCheck, Clock, UserCheck, UserX } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ReportShell, FilterBar, ExportMenu, Kpi, KpiGrid } from "./report-shell";
import { attendance, attendanceSummary, userById, users } from "@/lib/tenant-data";
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

  const dates = useMemo(() => [...new Set(attendance.map((a) => a.date))].sort(), []);
  const members = users.filter((u) => (u.role === "worker" || u.role === "owner") && (memberId === "all" || u.id === memberId));

  const rows = useMemo<Row[]>(
    () =>
      members.map((u) => {
        const ua = attendance.filter((a) => a.userId === u.id);
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
    [members]
  );

  const s = attendanceSummary();
  const totalDays = s.present + s.late + s.absent + s.remote;
  const attendanceRate = totalDays ? Math.round(((s.present + s.late + s.remote) / totalDays) * 100) : 0;
  const onTimeRate = totalDays ? Math.round((s.present + s.remote) / totalDays * 100) : 0;

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
          <div className="flex items-center gap-3 text-xs text-muted-foreground">
            {(Object.keys(statusMeta) as AttendanceStatus[]).map((k) => (
              <span key={k} className="flex items-center gap-1.5">
                <span className={cn("h-2.5 w-2.5 rounded", statusMeta[k].cell)} /> {statusMeta[k].label}
              </span>
            ))}
          </div>
        </CardHeader>
        <CardContent className="overflow-x-auto">
          <div className="min-w-[640px]">
            <div className="grid gap-2" style={{ gridTemplateColumns: `180px repeat(${dates.length}, 1fr)` }}>
              <div className="text-xs font-medium text-muted-foreground">Member</div>
              {dates.map((d) => (
                <div key={d} className="text-center text-xs font-medium text-muted-foreground">
                  {new Date(d).toLocaleDateString("en", { weekday: "short" })}
                </div>
              ))}
              {members.map((u) => (
                <div key={u.id} className="contents">
                  <div className="flex items-center gap-2 py-1">
                    <Avatar name={u.name} size="sm" />
                    <span className="truncate text-sm font-medium">{u.name.split(" ")[0]}</span>
                  </div>
                  {dates.map((d) => {
                    const rec = attendance.find((a) => a.userId === u.id && a.date === d);
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
