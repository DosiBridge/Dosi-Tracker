"use client";

import { useState } from "react";
import { ChevronLeft, ChevronRight, Clock, TrendingUp } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { users } from "@/lib/tenant-data";
import { cn, formatDuration } from "@/lib/utils";

const days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const trackingUsers = users.filter((u) => u.role === "worker" || u.role === "owner");

// Deterministic weekly matrix (minutes) derived from each user's baseline.
function minutesFor(userIndex: number, dayIndex: number): number {
  const base = trackingUsers[userIndex].trackedToday;
  const weekdayFactor = [0.95, 1.05, 0.85, 1.1, 1.0, 0.35, 0.15][dayIndex];
  const wobble = ((userIndex * 7 + dayIndex * 13) % 5) * 12 - 24;
  return Math.max(0, Math.round((base * weekdayFactor + wobble) / 5) * 5);
}

function cellTone(minutes: number): string {
  if (minutes === 0) return "bg-muted/40 text-muted-foreground/40";
  if (minutes < 120) return "bg-primary/15 text-primary";
  if (minutes < 240) return "bg-primary/30 text-primary";
  if (minutes < 360) return "bg-primary/55 text-white";
  return "bg-primary text-primary-foreground";
}

export default function TimesheetPage() {
  const [weekOffset, setWeekOffset] = useState(0);

  const matrix = trackingUsers.map((_, ui) => days.map((_, di) => minutesFor(ui, di)));
  const dayTotals = days.map((_, di) => matrix.reduce((s, row) => s + row[di], 0));
  const grandTotal = dayTotals.reduce((s, v) => s + v, 0);
  const busiestDay = days[dayTotals.indexOf(Math.max(...dayTotals))];

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Timesheet</h1>
          <p className="mt-1 text-sm text-muted-foreground">Weekly tracked time across the team.</p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="icon" onClick={() => setWeekOffset((w) => w - 1)}>
            <ChevronLeft className="h-4 w-4" />
          </Button>
          <span className="min-w-28 text-center text-sm font-medium">
            {weekOffset === 0 ? "This week" : weekOffset === -1 ? "Last week" : `${Math.abs(weekOffset)} weeks ago`}
          </span>
          <Button
            variant="outline"
            size="icon"
            onClick={() => setWeekOffset((w) => Math.min(0, w + 1))}
            disabled={weekOffset === 0}
          >
            <ChevronRight className="h-4 w-4" />
          </Button>
        </div>
      </div>

      {/* Summary */}
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <Card className="p-4">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/15 text-primary">
              <Clock className="h-5 w-5" />
            </div>
            <div>
              <div className="text-lg font-bold">{formatDuration(grandTotal)}</div>
              <div className="text-xs text-muted-foreground">Total this week</div>
            </div>
          </div>
        </Card>
        <Card className="p-4">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-success/15 text-success">
              <TrendingUp className="h-5 w-5" />
            </div>
            <div>
              <div className="text-lg font-bold">{busiestDay}</div>
              <div className="text-xs text-muted-foreground">Most productive day</div>
            </div>
          </div>
        </Card>
        <Card className="p-4">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-info/15 text-info">
              <Clock className="h-5 w-5" />
            </div>
            <div>
              <div className="text-lg font-bold">{formatDuration(Math.round(grandTotal / trackingUsers.length))}</div>
              <div className="text-xs text-muted-foreground">Avg per member</div>
            </div>
          </div>
        </Card>
      </div>

      {/* Heatmap grid */}
      <Card>
        <CardHeader>
          <CardTitle>Weekly breakdown</CardTitle>
          <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <span>Less</span>
            <span className="h-3 w-3 rounded bg-primary/15" />
            <span className="h-3 w-3 rounded bg-primary/30" />
            <span className="h-3 w-3 rounded bg-primary/55" />
            <span className="h-3 w-3 rounded bg-primary" />
            <span>More</span>
          </div>
        </CardHeader>
        <CardContent className="overflow-x-auto">
          <div className="min-w-[640px]">
            {/* Header row */}
            <div className="grid grid-cols-[180px_repeat(7,1fr)_90px] gap-2 border-b border-border pb-2 text-xs font-medium text-muted-foreground">
              <div>Member</div>
              {days.map((d) => (
                <div key={d} className="text-center">{d}</div>
              ))}
              <div className="text-right">Total</div>
            </div>

            {/* Member rows */}
            {trackingUsers.map((u, ui) => {
              const rowTotal = matrix[ui].reduce((s, v) => s + v, 0);
              return (
                <div key={u.id} className="grid grid-cols-[180px_repeat(7,1fr)_90px] items-center gap-2 border-b border-border py-2 last:border-0">
                  <div className="flex items-center gap-2">
                    <Avatar name={u.name} size="sm" status={u.status} />
                    <div className="min-w-0">
                      <div className="truncate text-sm font-medium">{u.name.split(" ")[0]}</div>
                      <div className="truncate text-[11px] text-muted-foreground">{u.designation}</div>
                    </div>
                  </div>
                  {matrix[ui].map((m, di) => (
                    <div
                      key={di}
                      className={cn(
                        "flex h-11 items-center justify-center rounded-lg text-[11px] font-medium transition-transform hover:scale-105",
                        cellTone(m)
                      )}
                      title={`${u.name} · ${days[di]} · ${formatDuration(m)}`}
                    >
                      {m > 0 ? `${Math.round(m / 60)}h` : "–"}
                    </div>
                  ))}
                  <div className="text-right text-sm font-semibold">{formatDuration(rowTotal)}</div>
                </div>
              );
            })}

            {/* Totals row */}
            <div className="grid grid-cols-[180px_repeat(7,1fr)_90px] items-center gap-2 pt-2 text-xs font-semibold">
              <div className="text-muted-foreground">Daily total</div>
              {dayTotals.map((t, i) => (
                <div key={i} className="text-center text-muted-foreground">{Math.round(t / 60)}h</div>
              ))}
              <div className="text-right"><Badge tone="primary">{formatDuration(grandTotal)}</Badge></div>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
