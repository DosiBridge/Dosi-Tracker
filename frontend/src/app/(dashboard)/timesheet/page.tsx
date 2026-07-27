"use client";

import { useEffect, useMemo, useState } from "react";
import { ChevronLeft, ChevronRight, Clock, TrendingUp } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { Button } from "@/components/ui/button";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { users } from "@/lib/tenant-data";
import { useSession } from "@/components/session-provider";
import { trackedMembers } from "@/lib/roles";
import { cn, formatDuration } from "@/lib/utils";
import { getApi } from "@/hooks/useApi";

const days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

const DAY_MS = 24 * 60 * 60 * 1000;

interface DailySeriesPoint {
  date: string;
  trackedMinutes: number;
}

/** Mon 00:00:00 UTC → Sun 23:59:59.999 UTC for the week `offset` weeks from now. */
function weekRangeUtc(offset: number): { from: Date; to: Date } {
  const now = new Date();
  const sinceMonday = (now.getUTCDay() + 6) % 7;
  const from = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() - sinceMonday + offset * 7)
  );
  const to = new Date(from.getTime() + 7 * DAY_MS - 1);
  return { from, to };
}

function minutesFor(base: number, userIndex: number, dayIndex: number): number {
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
  const { user } = useSession();
  const [weekOffset, setWeekOffset] = useState(0);

  // LIVE MODE: real per-day minutes for the displayed week (Mon–Sun, UTC).
  // The server scopes non-admin callers to their own data automatically.
  const [hasToken, setHasToken] = useState(false);
  const [liveWeek, setLiveWeek] = useState<number[] | null>(null);
  const [liveLoading, setLiveLoading] = useState(false);
  const [liveError, setLiveError] = useState<string | null>(null);

  useEffect(() => {
    if (typeof window === "undefined" || !localStorage.getItem("dosi-token")) return;
    setHasToken(true);
    const { from, to } = weekRangeUtc(weekOffset);
    let cancelled = false;
    setLiveLoading(true);
    setLiveError(null);
    getApi(
      `/api/app/reporting/daily-series?From=${encodeURIComponent(from.toISOString())}&To=${encodeURIComponent(to.toISOString())}`
    )
      .then((series) => {
        if (cancelled) return;
        const week = Array<number>(7).fill(0);
        for (const point of (Array.isArray(series) ? series : []) as DailySeriesPoint[]) {
          // Anchor on the calendar date only — the backend may serialize without a zone suffix.
          const dayUtc = Date.parse(`${String(point.date).slice(0, 10)}T00:00:00Z`);
          const di = Math.floor((dayUtc - from.getTime()) / DAY_MS);
          if (di >= 0 && di < 7) week[di] = Math.max(0, Math.round(point.trackedMinutes ?? 0));
        }
        setLiveWeek(week);
      })
      .catch(() => {
        if (cancelled) return;
        setLiveWeek(null); // fall back to the demo grid below
        setLiveError("Couldn't load tracked time from the server — showing demo data.");
      })
      .finally(() => {
        if (!cancelled) setLiveLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [weekOffset]);

  const trackingUsers = useMemo(() => {
    const all = trackedMembers(users);
    if (user.role === "worker") return all.filter((u) => u.id === user.id);
    return all;
  }, [user]);

  const liveMatrix = hasToken && liveWeek ? [liveWeek] : null;
  const live = liveMatrix !== null;
  const gridUsers = live ? [user] : trackingUsers;
  const matrix =
    liveMatrix ?? trackingUsers.map((u, ui) => days.map((_, di) => minutesFor(u.trackedToday, ui, di)));
  const dayTotals = days.map((_, di) => matrix.reduce((s, row) => s + row[di], 0));
  const grandTotal = dayTotals.reduce((s, v) => s + v, 0);
  const busiestDay = days[dayTotals.indexOf(Math.max(...dayTotals))];
  const selfOnly = user.role === "worker" || live;

  return (
    <PageStack>
      <PageHeader
        eyebrow="Monitor"
        title={selfOnly ? "My timesheet" : "Timesheet"}
        description={selfOnly ? "Your weekly tracked time." : "Weekly tracked time across the team."}
        actions={
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
        }
      />

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <Card className="p-4">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-muted text-muted-foreground">
              <Clock className="h-5 w-5" />
            </div>
            <div>
              <div className="font-display text-lg font-bold tabular-nums">{formatDuration(grandTotal)}</div>
              <div className="text-xs text-muted-foreground">Total this week</div>
            </div>
          </div>
        </Card>
        <Card className="p-4">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-muted text-muted-foreground">
              <TrendingUp className="h-5 w-5" />
            </div>
            <div>
              <div className="font-display text-lg font-bold tabular-nums">{busiestDay}</div>
              <div className="text-xs text-muted-foreground">Busiest day</div>
            </div>
          </div>
        </Card>
        <Card className="p-4">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-muted text-muted-foreground">
              <Clock className="h-5 w-5" />
            </div>
            <div>
              <div className="font-display text-lg font-bold tabular-nums">
                {formatDuration(gridUsers.length ? Math.round(grandTotal / gridUsers.length) : 0)}
              </div>
              <div className="text-xs text-muted-foreground">{selfOnly ? "Daily average" : "Avg per member"}</div>
            </div>
          </div>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">{selfOnly ? "Your week" : "Team heatmap"}</CardTitle>
        </CardHeader>
        <CardContent>
          {hasToken && liveLoading && (
            <p className="mb-3 text-xs text-muted-foreground">Loading tracked time…</p>
          )}
          {hasToken && !liveLoading && liveError && (
            <p className="mb-3 text-xs text-danger">{liveError}</p>
          )}
          {/* Mobile: per-member week cards */}
          <div className="space-y-3 md:hidden">
            {gridUsers.map((u, ui) => {
              const rowTotal = matrix[ui].reduce((s, v) => s + v, 0);
              return (
                <div key={u.id} className="rounded-xl border border-border p-3">
                  <div className="mb-3 flex items-center justify-between gap-2">
                    <div className="flex min-w-0 items-center gap-2.5">
                      <Avatar name={u.name} size="sm" status={u.status} />
                      <div className="min-w-0">
                        <div className="truncate text-sm font-medium">{u.name}</div>
                        <div className="truncate text-xs text-muted-foreground">{u.designation}</div>
                      </div>
                    </div>
                    <Badge tone="muted">{formatDuration(rowTotal)}</Badge>
                  </div>
                  <div className="grid grid-cols-7 gap-1">
                    {days.map((d, di) => (
                      <div key={d} className="text-center">
                        <div className="mb-1 text-[10px] font-medium text-muted-foreground">{d[0]}</div>
                        <div
                          className={cn(
                            "flex h-9 items-center justify-center rounded-md text-[10px] font-medium tabular-nums",
                            cellTone(matrix[ui][di])
                          )}
                        >
                          {matrix[ui][di] === 0 ? "—" : formatDuration(matrix[ui][di])}
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              );
            })}
            {!selfOnly && (
              <div className="rounded-xl border border-border bg-muted/40 p-3 text-center text-xs font-medium text-muted-foreground">
                Week total · {formatDuration(grandTotal)}
              </div>
            )}
          </div>

          {/* Desktop: heatmap table */}
          <div className="hidden overflow-x-auto overscroll-x-contain md:block">
            <table className="w-full min-w-[640px] border-collapse text-sm">
              <thead>
                <tr className="text-left text-xs text-muted-foreground">
                  <th className="sticky left-0 z-10 bg-card pb-3 pr-3 font-medium">{selfOnly ? "You" : "Member"}</th>
                  {days.map((d) => (
                    <th key={d} className="pb-3 px-1 text-center font-medium">{d}</th>
                  ))}
                  <th className="pb-3 pl-3 text-right font-medium">Total</th>
                </tr>
              </thead>
              <tbody>
                {gridUsers.map((u, ui) => {
                  const rowTotal = matrix[ui].reduce((s, v) => s + v, 0);
                  return (
                    <tr key={u.id} className="group border-t border-border/60">
                      <td className="sticky left-0 z-10 bg-card py-2.5 pr-3 group-hover:bg-muted/30">
                        <div className="flex items-center gap-2.5">
                          <Avatar name={u.name} size="sm" status={u.status} />
                          <div className="min-w-0">
                            <div className="truncate font-medium">{u.name}</div>
                            <div className="truncate text-xs text-muted-foreground">{u.designation}</div>
                          </div>
                        </div>
                      </td>
                      {matrix[ui].map((mins, di) => (
                        <td key={di} className="px-1 py-2.5 text-center">
                          <div
                            className={cn(
                              "mx-auto flex h-10 w-full max-w-14 items-center justify-center rounded-md text-[11px] font-medium tabular-nums",
                              cellTone(mins)
                            )}
                            title={formatDuration(mins)}
                          >
                            {mins === 0 ? "—" : formatDuration(mins)}
                          </div>
                        </td>
                      ))}
                      <td className="py-2.5 pl-3 text-right">
                        <Badge tone="muted">{formatDuration(rowTotal)}</Badge>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
              {!selfOnly && (
                <tfoot>
                  <tr className="border-t border-border">
                    <td className="sticky left-0 z-10 bg-card pt-3 pr-3 text-xs font-medium text-muted-foreground">Day total</td>
                    {dayTotals.map((t, i) => (
                      <td key={i} className="px-1 pt-3 text-center text-xs font-medium tabular-nums">
                        {formatDuration(t)}
                      </td>
                    ))}
                    <td className="pt-3 pl-3 text-right text-xs font-semibold">{formatDuration(grandTotal)}</td>
                  </tr>
                </tfoot>
              )}
            </table>
          </div>
        </CardContent>
      </Card>
    </PageStack>
  );
}
