"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import Link from "next/link";
import { Clock, Users, Gauge, CalendarRange, ArrowRight, FileText, RotateCw } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { StatCard } from "@/components/dashboard/stat-card";
import { ActivityRing } from "@/components/dashboard/activity-ring";
import { linkButtonClass } from "@/components/dashboard/link-button";
import { memberPresence } from "@/components/dashboard/devices";
import {
  ActivityTrendChart,
  ChartEmpty,
  ChartLegend,
  DonutLegend,
  HourlyChart,
  ProjectDonut,
  TREND_SERIES,
  TopAppsChart,
  type DonutSlice,
  type TrendPoint,
} from "@/components/dashboard/charts";
import { AppGlyph, ScreenMockView } from "@/components/screen-mock";
import { useSession } from "@/components/session-provider";
import { AiEnableCta } from "@/components/ai/ai-enable-cta";
import { AiPeriodSummaryCard } from "@/components/ai/ai-period-summary-card";
import { useAiAssistant } from "@/components/ai/ai-provider";
import { activities, hourlyToday, projectById, projectDistribution, summary, topApps, userById, users } from "@/lib/tenant-data";
import { brand, greeting } from "@/lib/brand";
import { formatDuration, colorFromString } from "@/lib/utils";
import { Reveal, Stagger } from "@/components/motion/reveal";
import { getApi, getHourlyReport, getTeamPresence } from "@/hooks/useApi";
import { useReportingZone } from "@/hooks/use-reporting-zone";
import { agoLabel, referenceNow } from "@/lib/live-session";
import { deriveDailyTrend, displayName } from "@/lib/live-dataset";
import { friendlyAppName } from "@/lib/apps";
import { demoPresence, latestBlockByUser, liveBlockIds } from "@/lib/monitor-live";
import { describeCoverage } from "@/lib/live-loader";
import {
  reportingRoute,
  type AppUsageRowDto,
  type DailyReportRowDto,
  type ReportSummaryDto,
  type UserReportRowDto,
} from "@/lib/api-contract";
import { appsFromUsage, arrayOf, hoursFromReport, trendFromSeries } from "./dashboard-data";
import { zonedDayKey, zonedDayRange, zonedPeriod } from "@/lib/time-zone";
import type { UserStatus } from "@/lib/types";

/** What the live dashboard reads from the reporting API (calendar days of the reporting zone). */
interface LiveDashboard {
  today: Partial<ReportSummaryDto>;
  week: Partial<ReportSummaryDto>;
  /** The last 7 days, oldest first, one point per day (empty days included). */
  trend: TrendPoint[];
  /** Minutes per canonical app this week; null when the server couldn't say. */
  apps: { app: string; minutes: number; color: string }[] | null;
  /** Minutes per hour today; null when the server couldn't say. */
  hourly: { hour: string; minutes: number }[] | null;
  /** Who is tracking now (agent heartbeats); null on servers without presence. */
  presence: Map<string, UserStatus> | null;
}

function memberLabel(row: UserReportRowDto): string {
  return userById(row.userId)?.name ?? displayName({ name: row.name, userName: row.userName }, "Team member");
}

export function AdminDashboard({
  userName,
  isOwner,
  onboarding,
}: {
  userName: string;
  isOwner: boolean;
  /** Shown under the greeting while the workspace is being set up (the dashboard page passes the SetupChecklist). */
  onboarding?: React.ReactNode;
}) {
  // LIVE MODE: every figure comes from the reporting API for whole calendar
  // days of the reporting zone (never from a capped page of rows); the feed
  // lists the newest real blocks. Demo mode renders the seeded dataset.
  const { isLive, liveCoverage } = useSession();
  const fallbackCoverage = describeCoverage(liveCoverage);
  const zone = useReportingZone(isLive);
  const [live, setLive] = useState<LiveDashboard | null>(null);
  const [liveLoading, setLiveLoading] = useState(false);
  const [liveError, setLiveError] = useState<string | null>(null);
  // AI assist's week: Monday to today in the reporting zone, on the session's clock (the demo's frozen one).
  const aiWeek = useMemo(() => zonedPeriod("week", referenceNow(), zone.timeZone), [zone.timeZone]);
  const { status: aiStatus } = useAiAssistant();
  // The team's week when the server allows this manager the team scope, else their own.
  const aiScope = aiStatus?.features.find((f) => f.featureId === "period-summary")?.scopes.includes("team") === false ? "self" : "team";

  const load = useCallback(async () => {
    const tz = zone.timeZone;
    const now = new Date();
    const todayKey = zonedDayKey(now, tz);
    const today = zonedDayRange(todayKey, todayKey, tz);
    const week = zonedPeriod("week", now, tz);
    const last7 = zonedPeriod("7d", now, tz);
    setLiveLoading(true);
    const [todayRes, weekRes, seriesRes, appsRes, hourlyRes, presenceRes] = await Promise.allSettled([
      getApi(reportingRoute("summary", { from: today.from, to: today.to, timeZone: tz })),
      getApi(reportingRoute("summary", { from: week.from, to: week.to, timeZone: tz })),
      getApi(reportingRoute("daily-series", { from: last7.from, to: last7.to, timeZone: tz })),
      getApi(reportingRoute("app-usage", { from: week.from, to: week.to, timeZone: tz })),
      getHourlyReport({ from: today.from, to: today.to, timeZone: tz }),
      getTeamPresence(),
    ]);
    if (todayRes.status === "rejected" || weekRes.status === "rejected" || seriesRes.status === "rejected") {
      setLive(null);
      setLiveError("Couldn't load the team's figures from the server. The charts below show the activity already loaded.");
    } else {
      setLive({
        today: (todayRes.value ?? {}) as Partial<ReportSummaryDto>,
        week: (weekRes.value ?? {}) as Partial<ReportSummaryDto>,
        trend: trendFromSeries(last7.days, arrayOf<DailyReportRowDto>(seriesRes.value)),
        apps: appsRes.status === "fulfilled" ? appsFromUsage(arrayOf<AppUsageRowDto>(appsRes.value)) : null,
        hourly: hourlyRes.status === "fulfilled" ? hoursFromReport(hourlyRes.value) : null,
        presence: presenceRes.status === "fulfilled" ? memberPresence(presenceRes.value, new Date()) : null,
      });
      setLiveError(null);
    }
    setLiveLoading(false);
  }, [zone.timeZone]);

  useEffect(() => {
    if (!isLive || !zone.ready) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void load();
  }, [isLive, zone.ready, load]);

  // Demo: the trend comes from the demo's own activity and stops at today.
  const demoTrend = useMemo(() => (isLive ? [] : deriveDailyTrend(activities, referenceNow(), 7)), [isLive]);

  const trendData: TrendPoint[] = isLive ? (live?.trend ?? deriveDailyTrend(activities, new Date(), 7)) : demoTrend;
  const dist: DonutSlice[] = useMemo(() => {
    if (!isLive || !live) return projectDistribution().filter((d) => d.value > 0);
    return arrayOf<{ projectId: string; projectTitle?: string; trackedMinutes: number }>(live.week.perProject)
      .filter((r) => r && typeof r.projectId === "string" && Number(r.trackedMinutes) > 0)
      .map((r) => {
        const known = projectById(r.projectId);
        return {
          name: r.projectTitle || known?.title || "Untitled project",
          value: Math.round(Number(r.trackedMinutes)),
          color: known?.color ?? colorFromString(r.projectId, 60, 50),
        };
      })
      .sort((a, b) => b.value - a.value);
  }, [isLive, live]);
  const apps = isLive ? (live?.apps ?? topApps.map((a) => ({ ...a, app: friendlyAppName(a.app) }))) : topApps;
  const hourly = isLive ? (live?.hourly ?? hourlyToday) : hourlyToday;
  const hasHourly = hourly.some((h) => h.minutes > 0);

  const leaderboard = useMemo(() => {
    if (isLive && live) {
      return arrayOf<UserReportRowDto>(live.today.perUser)
        .filter((r) => r && typeof r.userId === "string" && Number(r.trackedMinutes) > 0)
        .sort((a, b) => Number(b.trackedMinutes) - Number(a.trackedMinutes))
        .slice(0, 5)
        .map((r) => ({
          id: r.userId,
          name: memberLabel(r),
          detail: userById(r.userId)?.designation ?? "",
          minutes: Math.round(Number(r.trackedMinutes)),
          activity: typeof r.averageProductivity === "number" ? Math.round(r.averageProductivity) : null,
          status: live.presence?.get(r.userId) ?? userById(r.userId)?.status,
        }));
    }
    return [...users]
      .filter((u) => u.trackedToday > 0)
      .sort((a, b) => b.trackedToday - a.trackedToday)
      .slice(0, 5)
      .map((u) => ({ id: u.id, name: u.name, detail: u.designation, minutes: u.trackedToday, activity: u.productivity, status: u.status }));
  }, [isLive, live]);

  const feed = activities.slice(0, 6);
  // "live" marks a member's newest block while they are tracking (fresh presence when the server reports it,
  // otherwise the members' statuses) — the Activity page's rule, never a flag stored on the block.
  const liveFeedIds = useMemo(() => {
    const now = referenceNow();
    const presence = live?.presence
      ? new Map([...live.presence].map(([id, status]) => [id, { state: status === "active" ? ("tracking" as const) : ("offline" as const) }]))
      : demoPresence(users, latestBlockByUser(activities));
    return liveBlockIds(activities, presence, now, (projectId) => projectById(projectId)?.intervalMinutes);
  }, [live]);
  const noActivityCopy = "No activity yet — install the desktop agent to start tracking.";

  // Headline figures.
  const trackingNow = isLive
    ? live?.presence
      ? [...live.presence.values()].filter((s) => s === "active").length
      : null
    : summary.activeMembers;
  const activeToday = isLive ? arrayOf<UserReportRowDto>(live?.today.perUser).filter((r) => Number(r?.trackedMinutes) > 0).length : null;
  const dash = liveLoading && !live ? "…" : "—";
  const todayMinutes = isLive ? (live ? Math.round(Number(live.today.totalTrackedMinutes) || 0) : null) : summary.totalTrackedToday;
  const weekMinutes = isLive ? (live ? Math.round(Number(live.week.totalTrackedMinutes) || 0) : null) : dist.reduce((s, d) => s + d.value, 0);
  const activityToday = isLive
    ? live && typeof live.today.averageProductivity === "number"
      ? `${Math.round(live.today.averageProductivity)}%`
      : live
        ? "—"
        : dash
    : `${summary.avgProductivity}%`;

  return (
    <div className="space-y-6">
      <Reveal>
        <div className="flex flex-wrap items-end justify-between gap-4">
          <div>
            <div className="flex flex-wrap items-center gap-2">
              <h1 className="page-title">{greeting(userName)}</h1>
              {trackingNow !== null ? (
                <Badge tone="success" className="gap-1.5">
                  <span aria-hidden className="h-1.5 w-1.5 rounded-full bg-success live-dot" />
                  {trackingNow} tracking now
                </Badge>
              ) : activeToday !== null && live ? (
                <Badge tone="info">{activeToday} active today</Badge>
              ) : null}
            </div>
            <p className="mt-1 text-sm text-muted-foreground">
              {isOwner ? "Organization overview for today." : "What your team is working on today."}
            </p>
            {liveLoading && !live && (
              <p role="status" className="mt-1 text-xs text-muted-foreground">
                Loading the team&apos;s figures…
              </p>
            )}
            {liveError && (
              <p role="alert" className="mt-1 flex flex-wrap items-center gap-2 text-xs text-danger-on-tint">
                {liveError}
                {/* The charts now fall back to the session's blocks: say when those are only the newest ones. */}
                {fallbackCoverage && <span className="text-muted-foreground">{fallbackCoverage}</span>}
                <button type="button" onClick={() => void load()} className="inline-flex items-center gap-1 font-medium underline underline-offset-2">
                  <RotateCw aria-hidden className="h-3 w-3" /> Retry
                </button>
              </p>
            )}
          </div>
          <div className="flex items-center gap-2">
            <Link href="/reports/work-report" className={linkButtonClass("outline", "md")}>
              <FileText aria-hidden className="h-4 w-4" /> Work report
            </Link>
            <Link href="/activities" className={linkButtonClass("primary", "md")}>
              View activity <ArrowRight aria-hidden className="h-4 w-4" />
            </Link>
          </div>
        </div>
      </Reveal>

      {onboarding && <Reveal delay={60}>{onboarding}</Reveal>}

      <Stagger className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4" baseDelay={55}>
        <StatCard
          label="Tracked today"
          value={todayMinutes === null ? dash : formatDuration(todayMinutes)}
          icon={Clock}
          accent={brand.primary}
          sub="across all members"
        />
        <StatCard
          label={trackingNow !== null || !isLive ? "Tracking now" : "Active today"}
          value={
            !isLive
              ? `${summary.activeMembers}/${summary.totalMembers}`
              : trackingNow !== null
                ? String(trackingNow)
                : activeToday !== null && live
                  ? String(activeToday)
                  : dash
          }
          icon={Users}
          accent={brand.info}
          sub={!isLive ? "currently online" : trackingNow !== null ? "agents tracking right now" : "members with time today"}
        />
        <StatCard label="Activity today" value={activityToday} icon={Gauge} accent={brand.success} sub="keyboard & mouse share" />
        <StatCard
          label="Tracked this week"
          value={weekMinutes === null ? dash : formatDuration(weekMinutes)}
          icon={CalendarRange}
          accent={brand.pink}
          sub="Monday to today"
        />
      </Stagger>

      <AiEnableCta />
      {zone.ready && <AiPeriodSummaryCard scope={aiScope} from={aiWeek.from} to={aiWeek.to} timeZone={zone.timeZone} />}

      <Reveal delay={120}>
        <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
          <Card className="card-elev-lg lg:col-span-2">
            <CardHeader>
              <div>
                <CardTitle>Activity · last 7 days</CardTitle>
                <p className="mt-0.5 text-xs text-muted-foreground">Hours per day; active time had keyboard or mouse input</p>
              </div>
              <ChartLegend items={TREND_SERIES.map((s) => ({ key: s.key, label: s.label, color: s.color }))} />
            </CardHeader>
            <CardContent>
              {isLive && !trendData.some((d) => d.tracked > 0) ? (
                <ChartEmpty message="Nothing tracked in the last 7 days." />
              ) : (
                <ActivityTrendChart data={trendData} />
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Time by project</CardTitle>
              <Badge tone="muted">This week</Badge>
            </CardHeader>
            <CardContent>
              {dist.length === 0 ? (
                <ChartEmpty message="No time tracked on projects this week." />
              ) : (
                <>
                  <ProjectDonut data={dist} />
                  <DonutLegend data={dist} max={5} />
                </>
              )}
            </CardContent>
          </Card>
        </div>
      </Reveal>

      <Reveal delay={180}>
        <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
          <Card className="card-quiet">
            <CardHeader>
              <CardTitle>Most used apps</CardTitle>
              <Badge tone="muted">This week</Badge>
            </CardHeader>
            <CardContent>
              {apps.length === 0 ? <ChartEmpty message="No app usage recorded this week." /> : <TopAppsChart data={apps} />}
            </CardContent>
          </Card>
          <Card className="card-quiet">
            <CardHeader>
              <CardTitle>Today by hour</CardTitle>
              <Badge tone="muted">Time tracked</Badge>
            </CardHeader>
            <CardContent>{isLive && !hasHourly ? <ChartEmpty message="Nothing tracked yet today." /> : <HourlyChart data={hourly} label="Time tracked per hour today" />}</CardContent>
          </Card>
        </div>
      </Reveal>

      <Reveal delay={220}>
        <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
          <Card>
            <CardHeader>
              <div>
                <CardTitle>Team today</CardTitle>
                <p className="mt-0.5 text-xs text-muted-foreground">Most time tracked today, with activity %</p>
              </div>
              <Link href="/team" className="text-xs text-primary-strong hover:underline">
                View team
              </Link>
            </CardHeader>
            <CardContent>
              {leaderboard.length === 0 ? (
                <p className="py-6 text-center text-sm text-muted-foreground">No one has tracked time today yet.</p>
              ) : (
                <ol className="space-y-1">
                  {leaderboard.map((u, i) => (
                    <li key={u.id} className="flex items-center gap-3 rounded-lg p-2 hover:bg-muted/60">
                      <span className="w-5 text-center text-sm font-semibold text-muted-foreground">{i + 1}</span>
                      <Avatar name={u.name} size="sm" status={u.status} />
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-sm font-medium">{u.name}</div>
                        {u.detail && <div className="truncate text-xs text-muted-foreground">{u.detail}</div>}
                      </div>
                      <span className="text-sm font-medium tabular-nums">{formatDuration(u.minutes)}</span>
                      <ActivityRing value={u.activity} size={36} stroke={4} />
                    </li>
                  ))}
                </ol>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Latest activity</CardTitle>
              <Link href="/activities" className="text-xs text-primary-strong hover:underline">
                View all
              </Link>
            </CardHeader>
            <CardContent>
              {feed.length === 0 ? (
                <p className="py-6 text-center text-sm text-muted-foreground">
                  {noActivityCopy}{" "}
                  <Link href="/download" className="text-primary-strong hover:underline">
                    Get the agent
                  </Link>
                </p>
              ) : (
                <ul className="space-y-3">
                  {feed.map((a) => {
                    const u = userById(a.userId);
                    const p = projectById(a.projectId);
                    return (
                      <li key={a.id} className="flex items-center gap-3">
                        {isLive ? (
                          <AppGlyph app={a.screen.app} color={a.screen.accent} className="h-12 w-20 shrink-0" />
                        ) : (
                          <ScreenMockView screen={a.screen} className="h-12 w-20 shrink-0" />
                        )}
                        <div className="min-w-0 flex-1">
                          <div className="truncate text-sm">
                            <span className="font-medium">{u?.name ?? "Team member"}</span>{" "}
                            <span className="text-muted-foreground">— {a.description}</span>
                          </div>
                          <div className="mt-0.5 flex items-center gap-2 text-xs text-muted-foreground">
                            <span aria-hidden className="h-2 w-2 shrink-0 rounded-full" style={{ background: p?.color }} />
                            <span className="truncate">{p?.title ?? "Project"}</span>
                            <span aria-hidden>·</span>
                            <span className="shrink-0">{agoLabel(a.endedAt)}</span>
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
        </div>
      </Reveal>
    </div>
  );
}
