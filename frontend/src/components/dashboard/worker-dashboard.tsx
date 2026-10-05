"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import Link from "next/link";
import { Clock, Gauge, CalendarRange, Activity as ActivityIcon, ArrowRight, Download, RotateCw } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { StatCard } from "@/components/dashboard/stat-card";
import { ActivityRing } from "@/components/dashboard/activity-ring";
import { linkButtonClass } from "@/components/dashboard/link-button";
import { memberPresence } from "@/components/dashboard/devices";
import { appsFromUsage, arrayOf, trendFromSeries } from "@/components/dashboard/dashboard-data";
import type { TrendPoint } from "@/components/dashboard/charts";
import { AppGlyph, ScreenMockView } from "@/components/screen-mock";
import { useSession } from "@/components/session-provider";
import { activitiesForUser, projectById, projects } from "@/lib/tenant-data";
import { agoLabel, referenceNow } from "@/lib/live-session";
import { deriveDailyTrend } from "@/lib/live-dataset";
import { appColor, friendlyAppName } from "@/lib/apps";
import { liveBlockIds } from "@/lib/monitor-live";
import { durationMinutes } from "@/lib/metrics";
import { brand, greeting } from "@/lib/brand";
import { formatDuration } from "@/lib/utils";
import { Reveal, Stagger } from "@/components/motion/reveal";
import { getApi, getTeamPresence } from "@/hooks/useApi";
import { useReportingZone } from "@/hooks/use-reporting-zone";
import {
  reportingRoute,
  type AppUsageRowDto,
  type DailyReportRowDto,
  type ReportSummaryDto,
} from "@/lib/api-contract";
import { formatDayKey, zonedDayKey, zonedDayRange, zonedPeriod } from "@/lib/time-zone";
import type { Activity, User, UserStatus } from "@/lib/types";

/** The member's own figures (the reporting API scopes a member to their own data). */
interface LiveFigures {
  today: Partial<ReportSummaryDto>;
  week: Partial<ReportSummaryDto>;
  trend: TrendPoint[];
  /** Day keys of the 7 days behind `trend`, for the date labels. */
  days: string[];
  apps: { app: string; minutes: number; color: string }[] | null;
  status: UserStatus | null;
}

/** Minutes per app over the rows (seconds summed, rounded once), biggest first. */
function appsFromActivities(rows: readonly Activity[], limit = 5): { app: string; minutes: number; color: string }[] {
  const seconds = new Map<string, number>();
  for (const a of rows) {
    for (const w of a.activeWindows) {
      const name = friendlyAppName(w.appName);
      seconds.set(name, (seconds.get(name) ?? 0) + Math.max(0, w.seconds));
    }
  }
  return [...seconds.entries()]
    .map(([app, s]) => ({ app, minutes: Math.round(s / 60), color: appColor(app) }))
    .filter((a) => a.minutes > 0)
    .sort((a, b) => b.minutes - a.minutes || a.app.localeCompare(b.app))
    .slice(0, limit);
}

export function WorkerDashboard({ user }: { user: User }) {
  // LIVE MODE: the headline figures, the week and the apps come from the
  // reporting API (a member only ever sees their own data) for calendar days
  // of the reporting zone — the same "today" the desktop agent shows. Demo
  // mode derives the same views from the demo member's own rows.
  const { isLive } = useSession();
  const zone = useReportingZone(isLive);
  const [live, setLive] = useState<LiveFigures | null>(null);
  const [liveLoading, setLiveLoading] = useState(false);
  const [liveError, setLiveError] = useState(false);

  const load = useCallback(async () => {
    const tz = zone.timeZone;
    const now = new Date();
    const todayKey = zonedDayKey(now, tz);
    const today = zonedDayRange(todayKey, todayKey, tz);
    const week = zonedPeriod("7d", now, tz);
    setLiveLoading(true);
    const [todayRes, weekRes, seriesRes, appsRes, presenceRes] = await Promise.allSettled([
      getApi(reportingRoute("summary", { from: today.from, to: today.to, timeZone: tz })),
      getApi(reportingRoute("summary", { from: week.from, to: week.to, timeZone: tz })),
      getApi(reportingRoute("daily-series", { from: week.from, to: week.to, timeZone: tz })),
      getApi(reportingRoute("app-usage", { from: week.from, to: week.to, timeZone: tz })),
      getTeamPresence(),
    ]);
    if (todayRes.status === "rejected" || weekRes.status === "rejected" || seriesRes.status === "rejected") {
      setLive(null);
      setLiveError(true);
    } else {
      setLive({
        today: (todayRes.value ?? {}) as Partial<ReportSummaryDto>,
        week: (weekRes.value ?? {}) as Partial<ReportSummaryDto>,
        trend: trendFromSeries(week.days, arrayOf<DailyReportRowDto>(seriesRes.value)),
        days: week.days,
        apps: appsRes.status === "fulfilled" ? appsFromUsage(arrayOf<AppUsageRowDto>(appsRes.value), 5) : null,
        status: presenceRes.status === "fulfilled" ? (memberPresence(presenceRes.value, new Date()).get(user.id) ?? "offline") : null,
      });
      setLiveError(false);
    }
    setLiveLoading(false);
  }, [zone.timeZone, user.id]);

  useEffect(() => {
    if (!isLive || !zone.ready) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void load();
  }, [isLive, zone.ready, load]);

  const myActivities = useMemo(() => activitiesForUser(user.id), [user.id]);
  const myProjects = projects.filter((p) => !p.archived && (p.memberIds ?? []).includes(user.id));
  const feed = myActivities.slice(0, 6);

  // Demo: the week and the apps from the member's own demo rows, ending today.
  const demoNow = useMemo(() => referenceNow(), []);
  const demoTrend = useMemo(() => (isLive ? [] : deriveDailyTrend(myActivities, demoNow, 7)), [isLive, myActivities, demoNow]);
  const demoApps = useMemo(() => {
    if (isLive) return [];
    const since = demoNow.getTime() - 7 * 86_400_000;
    return appsFromActivities(myActivities.filter((a) => Date.parse(a.endedAt) >= since));
  }, [isLive, myActivities, demoNow]);

  const week: TrendPoint[] = isLive ? (live?.trend ?? []) : demoTrend;
  const weekMax = Math.max(1, ...week.map((d) => d.tracked));
  const myApps = isLive ? (live?.apps ?? []) : demoApps;
  const status: UserStatus = isLive ? (live?.status ?? user.status) : user.status;
  // "live" marks the member's newest block while they are tracking — the Activity page's rule, never a stored flag.
  const liveFeedIds = useMemo(
    () =>
      liveBlockIds(
        myActivities,
        new Map([[user.id, { state: status === "active" ? ("tracking" as const) : ("offline" as const) }]]),
        referenceNow(),
        (projectId) => projectById(projectId)?.intervalMinutes,
      ),
    [myActivities, status, user.id],
  );
  const pending = isLive && !live;
  const dash = liveLoading && pending ? "…" : "—";

  const todayMinutes = isLive ? (live ? Math.round(Number(live.today.totalTrackedMinutes) || 0) : null) : user.trackedToday;
  const todayActivity = isLive
    ? live && typeof live.today.averageProductivity === "number"
      ? Math.round(live.today.averageProductivity)
      : null
    : user.trackedToday > 0
      ? user.productivity
      : null;
  const weekMinutes = isLive ? (live ? Math.round(Number(live.week.totalTrackedMinutes) || 0) : null) : week.reduce((s, d) => s + d.tracked, 0);
  const weekBlocks = isLive ? (live ? Number(live.week.totalActivities) || 0 : null) : myActivities.filter((a) => Date.parse(a.endedAt) >= demoNow.getTime() - 7 * 86_400_000).length;
  // A member's own minutes per project over the last 7 days: the server's own-scoped summary (live), or the
  // member's own demo blocks over the same window as "Apps you used" — never the whole team's total.
  const demoProjectWeek = useMemo(() => {
    const out = new Map<string, number>();
    if (isLive) return out;
    const since = demoNow.getTime() - 7 * 86_400_000;
    for (const a of myActivities) {
      if (Date.parse(a.endedAt) >= since) out.set(a.projectId, (out.get(a.projectId) ?? 0) + durationMinutes(a));
    }
    return out;
  }, [isLive, myActivities, demoNow]);
  const projectWeek = (projectId: string): number | null => {
    if (!isLive) return Math.round(demoProjectWeek.get(projectId) ?? 0);
    if (!live) return null;
    const row = arrayOf<{ projectId: string; trackedMinutes: number }>(live.week.perProject).find((r) => r?.projectId === projectId);
    return Math.round(Number(row?.trackedMinutes) || 0);
  };
  const activeToday = todayMinutes !== null && todayActivity !== null ? Math.round((todayMinutes * todayActivity) / 100) : null;
  const nothingYet = !pending && myActivities.length === 0 && (todayMinutes ?? 0) === 0 && (weekMinutes ?? 0) === 0;

  return (
    <div className="space-y-6">
      <Reveal>
        <div className="flex flex-wrap items-end justify-between gap-4">
          <div>
            <div className="flex flex-wrap items-center gap-2">
              <h1 className="page-title">{greeting(user.name)}</h1>
              {status === "active" && (
                <Badge tone="success" className="gap-1.5">
                  <span aria-hidden className="h-1.5 w-1.5 rounded-full bg-success live-dot" /> Tracking now
                </Badge>
              )}
            </div>
            <p className="mt-1 text-sm text-muted-foreground">Your own activity only — not the rest of the team.</p>
          </div>
          <div className="flex gap-2">
            <Link href="/settings" className={linkButtonClass("outline", "md")}>
              Privacy
            </Link>
            <Link href="/timesheet" className={linkButtonClass("primary", "md")}>
              My timesheet <ArrowRight aria-hidden className="h-4 w-4" />
            </Link>
          </div>
        </div>
      </Reveal>

      {nothingYet && (
        <Card className="flex flex-col gap-3 border-primary/25 bg-primary/5 p-5 sm:flex-row sm:items-center">
          <div className="min-w-0 flex-1">
            <p className="font-medium">Install the desktop agent to start tracking</p>
            <p className="mt-0.5 text-sm text-muted-foreground">
              Time, activity and your week show up here after the agent&apos;s first upload.
            </p>
          </div>
          <Link href="/download" className={linkButtonClass("primary", "md")}>
            <Download aria-hidden className="h-4 w-4" /> Get the agent
          </Link>
        </Card>
      )}

      <Stagger className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4" baseDelay={55}>
        <StatCard label="Tracked today" value={todayMinutes === null ? dash : formatDuration(todayMinutes)} icon={Clock} accent={brand.primary} sub="your time" />
        <StatCard
          label="Activity today"
          value={todayActivity === null ? (pending ? dash : "—") : `${todayActivity}%`}
          icon={Gauge}
          accent={brand.success}
          sub="minutes with keyboard or mouse input"
        />
        <StatCard
          label="Last 7 days"
          value={weekMinutes === null ? dash : formatDuration(weekMinutes)}
          icon={CalendarRange}
          accent={brand.info}
          sub="your tracked time"
        />
        <StatCard
          label="Blocks"
          value={weekBlocks === null ? dash : String(weekBlocks)}
          icon={ActivityIcon}
          accent={brand.pink}
          sub="tracked blocks, last 7 days"
        />
      </Stagger>
      {liveError && (
        <p role="alert" className="flex flex-wrap items-center gap-2 text-xs text-danger-on-tint">
          Couldn&apos;t load your figures from the server.
          <button type="button" onClick={() => void load()} className="inline-flex items-center gap-1 font-medium underline underline-offset-2">
            <RotateCw aria-hidden className="h-3 w-3" /> Retry
          </button>
        </p>
      )}

      <Reveal delay={100}>
        <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
          <Card className="card-elev-lg lg:col-span-2">
            <CardHeader>
              <div>
                <CardTitle>Your week</CardTitle>
                <p className="mt-0.5 text-xs text-muted-foreground">Time you tracked each day, last 7 days</p>
              </div>
            </CardHeader>
            <CardContent>
              {week.length === 0 ? (
                <p className="py-6 text-center text-sm text-muted-foreground">{pending ? "Loading your week…" : "Nothing tracked this week yet."}</p>
              ) : (
                <ul className="space-y-2">
                  {week.map((d, i) => {
                    const key = isLive ? live?.days[i] : undefined;
                    return (
                      <li key={`${d.day}-${i}`} className="flex items-center gap-3 text-sm">
                        <span className="w-20 shrink-0 text-xs text-muted-foreground">
                          {d.day}
                          {key && <span className="ml-1 text-muted-foreground/80">{formatDayKey(key)}</span>}
                        </span>
                        <div
                          className="h-2 flex-1 overflow-hidden rounded-full bg-muted"
                          role="img"
                          aria-label={`${d.day}${key ? ` ${formatDayKey(key)}` : ""}: ${formatDuration(d.tracked)}`}
                        >
                          <div className="h-full rounded-full bg-primary" style={{ width: `${Math.min(100, (d.tracked / weekMax) * 100)}%` }} />
                        </div>
                        <span className="w-16 text-right text-xs tabular-nums text-muted-foreground">{formatDuration(d.tracked)}</span>
                      </li>
                    );
                  })}
                </ul>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Today&apos;s activity</CardTitle>
            </CardHeader>
            <CardContent className="flex flex-col items-center gap-3 py-4 text-center">
              <ActivityRing value={todayActivity} size={132} stroke={10} label="Activity today" />
              <p className="text-sm text-muted-foreground">
                {todayMinutes === null
                  ? "Loading today…"
                  : todayMinutes === 0
                    ? "Nothing tracked yet today."
                    : activeToday === null
                      ? `${formatDuration(todayMinutes)} tracked; input wasn't measured.`
                      : `${formatDuration(activeToday)} of ${formatDuration(todayMinutes)} had keyboard or mouse input.`}
              </p>
            </CardContent>
          </Card>
        </div>
      </Reveal>

      <Reveal delay={140}>
        <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
          <Card>
            <CardHeader>
              <CardTitle>Apps you used</CardTitle>
              <Badge tone="muted">Last 7 days</Badge>
            </CardHeader>
            <CardContent>
              {myApps.length === 0 ? (
                <p className="text-sm text-muted-foreground">{pending ? "Loading…" : "No app time recorded in the last 7 days."}</p>
              ) : (
                <ul className="space-y-3">
                  {myApps.map((a) => (
                    <li key={a.app} className="flex items-center justify-between gap-3 text-sm">
                      <span className="flex min-w-0 items-center gap-2">
                        <span aria-hidden className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ background: a.color }} />
                        <span className="truncate font-medium">{a.app}</span>
                      </span>
                      <span className="tabular-nums text-muted-foreground">{formatDuration(a.minutes)}</span>
                    </li>
                  ))}
                </ul>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>My projects</CardTitle>
              <Link href="/projects" className="text-xs text-primary-strong hover:underline">
                View all
              </Link>
            </CardHeader>
            <CardContent>
              {myProjects.length === 0 ? (
                <p className="text-sm text-muted-foreground">You&apos;re not on any active projects yet.</p>
              ) : (
                <ul className="space-y-1">
                  {myProjects.map((p) => {
                    const minutes = projectWeek(p.id);
                    return (
                      <li key={p.id}>
                        <Link href={`/projects/${p.id}`} className="flex items-center gap-3 rounded-lg p-2 hover:bg-muted/60">
                          <span
                            aria-hidden
                            className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg text-sm font-bold"
                            style={{ background: `${p.color}22`, color: p.color }}
                          >
                            {p.title[0]}
                          </span>
                          <span className="min-w-0 flex-1">
                            <span className="block truncate text-sm font-medium">{p.title}</span>
                            <span className="block truncate text-xs text-muted-foreground">
                              {minutes === null ? "…" : `${formatDuration(minutes)} in the last 7 days`}
                            </span>
                          </span>
                          <ArrowRight aria-hidden className="h-4 w-4 text-muted-foreground" />
                        </Link>
                      </li>
                    );
                  })}
                </ul>
              )}
            </CardContent>
          </Card>
        </div>
      </Reveal>

      <Reveal delay={180}>
        <Card>
          <CardHeader>
            <CardTitle>My recent activity</CardTitle>
            <Link href="/activities" className="text-xs text-primary-strong hover:underline">
              View all
            </Link>
          </CardHeader>
          <CardContent>
            {feed.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                No activity captured yet —{" "}
                <Link href="/download" className="font-medium text-primary-strong hover:underline">
                  install the desktop agent
                </Link>{" "}
                to start.
              </p>
            ) : (
              <ul className="space-y-3">
                {feed.map((a) => {
                  const p = projectById(a.projectId);
                  return (
                    <li key={a.id} className="flex items-center gap-3">
                      {isLive ? (
                        <AppGlyph app={a.screen.app} color={a.screen.accent} className="h-12 w-20 shrink-0" />
                      ) : (
                        <ScreenMockView screen={a.screen} className="h-12 w-20 shrink-0" />
                      )}
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-sm font-medium">{a.description}</div>
                        <div className="mt-0.5 flex items-center gap-2 text-xs text-muted-foreground">
                          <span aria-hidden className="h-2 w-2 shrink-0 rounded-full" style={{ background: p?.color }} />
                          <span className="truncate">{p?.title ?? "Project"}</span>
                          <span aria-hidden>·</span>
                          <span className="shrink-0">
                            {agoLabel(a.endedAt)} · {formatDuration(Math.round(durationMinutes(a)))}
                          </span>
                        </div>
                      </div>
                      {liveFeedIds.has(a.id) && <Badge tone="success">live</Badge>}
                    </li>
                  );
                })}
              </ul>
            )}
          </CardContent>
        </Card>
      </Reveal>
    </div>
  );
}
