"use client";

import { useEffect, useMemo, useState } from "react";
import { CalendarDays, ChevronLeft, ChevronRight, Clock, Globe, TrendingUp, Users } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { activities, users } from "@/lib/tenant-data";
import { useSession } from "@/components/session-provider";
import { useAiAssistant } from "@/components/ai/ai-provider";
import { DataQualityCard } from "@/components/ai/data-quality-card";
import { trackedMembers } from "@/lib/roles";
import { cn, formatDuration } from "@/lib/utils";
import { getApi } from "@/hooks/useApi";
import { useReportingZone } from "@/hooks/use-reporting-zone";
import { reportingRoute } from "@/lib/api-contract";
import { referenceNow } from "@/lib/live-session";
import { TIMESHEET_DATA_QUALITY_KINDS } from "@/lib/ai-assist";
import { formatDayKey, timeZoneLabel } from "@/lib/time-zone";
import {
  averagePerWorkedDay,
  buildWeekMatrix,
  heatStep,
  timesheetWeek,
  weekMatrixFromActivities,
  type GridMember,
  type UserDailyPoint,
} from "@/lib/timesheet";

const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/**
 * Heat shading for a day's hours, in the Work Report's language: rising
 * strengths of primary with foreground text on the tints, and light text only
 * on the solid top step — every step clears WCAG AA in both themes.
 */
const HEAT_CLASS: Record<0 | 1 | 2 | 3 | 4, string> = {
  0: "bg-muted/40 text-muted-foreground",
  1: "bg-primary/10 text-foreground",
  2: "bg-primary/25 text-foreground",
  3: "bg-primary/45 text-foreground",
  4: "bg-primary text-primary-foreground font-semibold",
};

/** A day that hasn't happened yet: an empty outline, never a zero. */
const FUTURE_CELL = "border border-dashed border-border bg-transparent text-muted-foreground";

function weekTitle(offset: number): string {
  if (offset === 0) return "This week";
  if (offset === -1) return "Last week";
  return `${Math.abs(offset)} weeks ago`;
}

/** The live answer for one week (keyed, so a late answer never shows under another week). */
interface LiveWeek {
  key: string;
  rows: UserDailyPoint[];
}

export default function TimesheetPage() {
  const { user, isLive } = useSession();
  const selfOnly = user.role === "worker";
  const [weekOffset, setWeekOffset] = useState(0);

  // Weeks are Monday–Sunday in the reporting zone (the same days every report uses).
  const zone = useReportingZone(isLive);
  const timeZone = zone.timeZone;
  const [now] = useState(() => referenceNow());
  const week = useMemo(() => timesheetWeek(now, timeZone, weekOffset), [now, timeZone, weekOffset]);
  // "Week review": the team's week for owners and managers (their own when the server allows them only that), a member's own.
  const { status: aiStatus } = useAiAssistant();
  const teamReview = aiStatus?.features.find((f) => f.featureId === "data-quality-review")?.scopes.includes("team") !== false;
  const reviewScope = selfOnly || !teamReview ? "self" : "team";

  // LIVE MODE: real per-member, per-day minutes for the displayed week from
  // GET /api/app/reporting/user-daily-series, asked for the reporting zone so
  // each row's date is a local day — ONE ROW PER MEMBER. The server scopes
  // non-managers to their own rows.
  const [reloadKey, setReloadKey] = useState(0);
  const [live, setLive] = useState<LiveWeek | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const key = `${timeZone}|${week.days[0]}|${reloadKey}`;

  useEffect(() => {
    // Wait for the reporting zone, so the week isn't fetched in the wrong zone first.
    if (!isLive || !zone.ready) return;
    let cancelled = false;
    getApi(reportingRoute("user-daily-series", { from: week.from, to: week.to, timeZone }))
      .then((series) => {
        if (cancelled) return;
        setLive({ key, rows: Array.isArray(series) ? (series as UserDailyPoint[]) : [] });
        setFailed(null);
      })
      .catch(() => {
        if (!cancelled) setFailed(key);
      });
    return () => {
      cancelled = true;
    };
  }, [isLive, zone.ready, key, week.from, week.to, timeZone]);

  const liveRows = isLive && live?.key === key ? live.rows : null;
  const liveLoading = isLive && !liveRows && failed !== key;
  const liveError = isLive && !liveRows && failed === key;

  const roster: GridMember[] = useMemo(() => {
    if (selfOnly) return [user];
    const all = trackedMembers(users);
    return all.length ? all : [user];
  }, [selfOnly, user]);

  // Live: the backend's member-days. Demo: the same grid derived from the demo's activity rows,
  // so stepping back a week shows that week (and the days still to come stay blank).
  const grid = useMemo(
    () =>
      isLive
        ? buildWeekMatrix(liveRows ?? [], week.days, roster)
        : weekMatrixFromActivities(activities, week.days, roster, timeZone),
    [isLive, liveRows, week.days, roster, timeZone],
  );

  const gridUsers = grid.members;
  const matrix = grid.matrix;
  const future = week.days.map((d) => d > week.today);
  const dayTotals = week.days.map((_, di) => matrix.reduce((s, row) => s + row[di], 0));
  const grandTotal = dayTotals.reduce((s, v) => s + v, 0);
  const busiest = grandTotal > 0 ? dayTotals.indexOf(Math.max(...dayTotals)) : -1;
  // A member's average day is over the days they worked — not the whole week's total.
  const worked = averagePerWorkedDay(dayTotals);
  const membersWithTime = matrix.filter((row) => row.some((m) => m > 0)).length;
  const zoneLabel = timeZoneLabel(timeZone, now);

  const dayHeader = (d: string, di: number) => `${formatDayKey(d, { weekday: true, year: true })}${future[di] ? " (upcoming)" : ""}`;

  return (
    <PageStack>
      <PageHeader
        eyebrow="Monitor"
        title={selfOnly ? "My timesheet" : "Timesheet"}
        description={
          <>
            {selfOnly ? "Your tracked time" : "Tracked time across the team"} for{" "}
            <span className="font-medium text-foreground">{week.span}</span>
            <span className="inline-flex items-center gap-1 whitespace-nowrap" title={timeZone}>
              {" · "}
              <Globe aria-hidden className="h-3.5 w-3.5" />
              <span className="sr-only">Days in </span>
              {zoneLabel}
            </span>
          </>
        }
        actions={
          <div className="inline-flex items-center gap-0.5 rounded-xl border border-border bg-card p-1">
            <button
              type="button"
              onClick={() => setWeekOffset((w) => w - 1)}
              aria-label="Previous week"
              className="flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <ChevronLeft className="h-4 w-4" />
            </button>
            <span aria-live="polite" className="min-w-28 px-1 text-center text-sm font-semibold">
              {weekTitle(weekOffset)}
            </span>
            <button
              type="button"
              onClick={() => setWeekOffset((w) => Math.min(0, w + 1))}
              disabled={weekOffset === 0}
              aria-label="Next week"
              className="flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-35"
            >
              <ChevronRight className="h-4 w-4" />
            </button>
          </div>
        }
      />

      <dl aria-label="Week at a glance" className="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <Kpi icon={Clock} label={weekOffset === 0 ? "Total this week" : "Week total"} value={formatDuration(grandTotal)} sub={week.span} />
        <Kpi
          icon={TrendingUp}
          label="Busiest day"
          value={busiest >= 0 ? formatDayKey(week.days[busiest], { weekday: true }) : "—"}
          sub={busiest >= 0 ? `${formatDuration(dayTotals[busiest])} tracked` : "No tracked time yet"}
        />
        {selfOnly ? (
          <Kpi
            icon={CalendarDays}
            label="Daily average"
            value={formatDuration(worked.minutes)}
            sub={worked.days > 0 ? `over ${worked.days} ${worked.days === 1 ? "day" : "days"} worked` : "No tracked time yet"}
          />
        ) : (
          <Kpi
            icon={Users}
            label="Avg per member"
            value={formatDuration(gridUsers.length ? Math.round(grandTotal / gridUsers.length) : 0)}
            sub={`${membersWithTime} of ${gridUsers.length} ${gridUsers.length === 1 ? "member" : "members"} tracked time`}
          />
        )}
      </dl>

      <Card>
        <CardHeader>
          <div>
            <CardTitle className="text-base">{selfOnly ? "Your week" : "Team heatmap"}</CardTitle>
            <p className="mt-0.5 text-xs text-muted-foreground">
              Hours per day · {week.span}
            </p>
          </div>
          <div aria-hidden className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <span>Less</span>
            {([1, 2, 3, 4] as const).map((l) => (
              <span key={l} className={cn("h-3 w-4 rounded-sm", HEAT_CLASS[l])} />
            ))}
            <span>More</span>
          </div>
        </CardHeader>
        <CardContent>
          {liveLoading && (
            <p role="status" className="mb-3 text-xs text-muted-foreground">
              Loading tracked time…
            </p>
          )}
          {liveError && (
            <p role="alert" className="mb-3 flex items-center gap-2 text-xs text-danger-on-tint">
              Couldn&apos;t load tracked time from the server.
              <button
                type="button"
                onClick={() => setReloadKey((k) => k + 1)}
                className="font-medium text-primary-strong hover:underline"
              >
                Try again
              </button>
            </p>
          )}
          {!liveLoading && !liveError && grandTotal === 0 && (
            <p className="mb-3 text-xs text-muted-foreground">
              {future.every(Boolean) ? "This week hasn't started yet." : `No tracked time in ${week.span}.`}
            </p>
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
                  <ul className="grid grid-cols-7 gap-1" aria-label={`${u.name}, hours per day`}>
                    {week.days.map((d, di) => (
                      <li key={d} className="text-center" aria-label={`${dayHeader(d, di)}: ${future[di] ? "upcoming" : matrix[ui][di] ? formatDuration(matrix[ui][di]) : "no time"}`}>
                        <div aria-hidden className="mb-1 text-[10px] font-medium leading-tight text-muted-foreground">
                          <span className="block">{WEEKDAYS[di][0]}</span>
                          <span className={cn("block tabular-nums", d === week.today && "font-semibold text-primary-strong")}>{Number(d.slice(8, 10))}</span>
                        </div>
                        <div
                          aria-hidden
                          className={cn(
                            "flex h-9 items-center justify-center rounded-md text-[10px] font-medium tabular-nums",
                            future[di] ? FUTURE_CELL : HEAT_CLASS[heatStep(matrix[ui][di])]
                          )}
                        >
                          {future[di] ? "" : matrix[ui][di] === 0 ? "—" : formatDuration(matrix[ui][di])}
                        </div>
                      </li>
                    ))}
                  </ul>
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
              <caption className="sr-only">
                Tracked hours per {selfOnly ? "day" : "member per day"}, {week.span}. Days are calendar days in {zoneLabel}.
              </caption>
              <thead>
                <tr className="text-left text-xs text-muted-foreground">
                  <th scope="col" className="sticky left-0 z-10 bg-card pb-3 pr-3 font-medium">{selfOnly ? "You" : "Member"}</th>
                  {week.days.map((d, di) => (
                    <th key={d} scope="col" className="px-1 pb-3 text-center font-medium">
                      <abbr title={dayHeader(d, di)} className="no-underline">
                        <span className="block text-[11px]">{WEEKDAYS[di]}</span>
                        <span className={cn("block text-[10px] font-normal tabular-nums", d === week.today && "font-semibold text-primary-strong")}>
                          {formatDayKey(d)}
                        </span>
                      </abbr>
                      {d === week.today && <span className="sr-only"> (today)</span>}
                    </th>
                  ))}
                  <th scope="col" className="pb-3 pl-3 text-right font-medium">Total</th>
                </tr>
              </thead>
              <tbody>
                {gridUsers.map((u, ui) => {
                  const rowTotal = matrix[ui].reduce((s, v) => s + v, 0);
                  return (
                    <tr key={u.id} className="group border-t border-border/60">
                      <th scope="row" className="sticky left-0 z-10 bg-card py-2.5 pr-3 text-left font-normal group-hover:bg-muted/30">
                        <div className="flex items-center gap-2.5">
                          <Avatar name={u.name} size="sm" status={u.status} />
                          <div className="min-w-0">
                            <div className="truncate font-medium">{u.name}</div>
                            <div className="truncate text-xs text-muted-foreground">{u.designation}</div>
                          </div>
                        </div>
                      </th>
                      {matrix[ui].map((mins, di) => (
                        <td key={week.days[di]} className="px-1 py-2.5 text-center">
                          <div
                            className={cn(
                              "mx-auto flex h-10 w-full max-w-14 items-center justify-center rounded-md text-[11px] font-medium tabular-nums",
                              future[di] ? FUTURE_CELL : HEAT_CLASS[heatStep(mins)]
                            )}
                            title={future[di] ? "Upcoming" : formatDuration(mins)}
                          >
                            {future[di] ? <span className="sr-only">Upcoming</span> : mins === 0 ? "—" : formatDuration(mins)}
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
                    <th scope="row" className="sticky left-0 z-10 bg-card pt-3 pr-3 text-left text-xs font-medium text-muted-foreground">
                      Day total
                    </th>
                    {dayTotals.map((t, i) => (
                      <td key={week.days[i]} className="px-1 pt-3 text-center text-xs font-medium tabular-nums">
                        {future[i] ? <span className="text-muted-foreground">—</span> : formatDuration(t)}
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

      {zone.ready && (
        <DataQualityCard
          title="Week review"
          purpose={
            reviewScope === "self"
              ? `What in your tracking for ${week.span} is worth a look: gaps, clock problems, input that wasn't measured.`
              : `What in the team's tracking for ${week.span} is worth a look: gaps, clock problems, input that wasn't measured.`
          }
          page="timesheet"
          scope={reviewScope}
          kinds={TIMESHEET_DATA_QUALITY_KINDS}
          from={week.from}
          to={week.to}
          timeZone={timeZone}
        />
      )}
    </PageStack>
  );
}

/** One headline figure of the week (the Work Report's tile, compact). */
function Kpi({ icon: Icon, label, value, sub }: { icon: LucideIcon; label: string; value: string; sub?: string }) {
  return (
    <div className="flex min-w-0 flex-col rounded-2xl border border-border bg-card p-4 card-elev">
      <dt className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
        <span aria-hidden className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
          <Icon className="h-3.5 w-3.5" />
        </span>
        <span className="min-w-0 leading-snug">{label}</span>
      </dt>
      <dd className="mt-2.5 font-display text-xl font-bold tracking-tight text-ink tabular-nums">{value}</dd>
      {sub && <dd className="mt-0.5 truncate text-xs text-muted-foreground">{sub}</dd>}
    </div>
  );
}
