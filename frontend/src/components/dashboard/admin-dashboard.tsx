"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { Clock, Users, Gauge, Camera, ArrowRight, Sparkles } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Ring } from "@/components/ui/ring";
import { Button } from "@/components/ui/button";
import { StatCard } from "@/components/dashboard/stat-card";
import { SetupChecklist } from "@/components/dashboard/setup-checklist";
import { ActivityTrendChart, HourlyChart, ProjectDonut, TopAppsChart } from "@/components/dashboard/charts";
import { ScreenMockView } from "@/components/screen-mock";
import {
  activities,
  hourlyToday,
  NOW,
  projectById,
  projectDistribution,
  summary,
  topApps,
  userById,
  users,
  weeklyTrend,
} from "@/lib/tenant-data";
import { brand, greeting } from "@/lib/brand";
import { formatDuration } from "@/lib/utils";
import { Reveal, Stagger } from "@/components/motion/reveal";
import { getApi } from "@/hooks/useApi";

/** Shapes returned by /api/app/reporting (live mode). */
interface LiveReportSummary {
  totalActivities: number;
  totalTrackedMinutes: number;
  averageProductivity: number;
  totalMouseClicks: number;
  totalKeyboardHits: number;
  perUser: { userId: string; activityCount: number; trackedMinutes: number; averageProductivity: number }[];
  perProject: { projectId: string; activityCount: number; trackedMinutes: number; averageProductivity: number }[];
}

interface LiveDailyPoint {
  date: string;
  activityCount: number;
  trackedMinutes: number;
  averageProductivity: number;
  mouseClicks: number;
  keyboardHits: number;
}

function agoLabel(iso: string) {
  const mins = Math.round((NOW.getTime() - new Date(iso).getTime()) / 60000);
  if (mins < 60) return `${mins}m ago`;
  const h = Math.floor(mins / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

export function AdminDashboard({ userName, isOwner }: { userName: string; isOwner: boolean }) {
  // LIVE MODE: only when a real session token exists; otherwise the mock
  // dataset below renders exactly as before (demo mode).
  const [live, setLive] = useState<{ summary: LiveReportSummary; series: LiveDailyPoint[] } | null>(null);
  const [liveLoading, setLiveLoading] = useState(false);
  const [liveError, setLiveError] = useState<string | null>(null);

  useEffect(() => {
    if (typeof window === "undefined" || !localStorage.getItem("dosi-token")) return;
    let cancelled = false;
    setLiveLoading(true);
    setLiveError(null);
    const to = new Date();
    const from = new Date(to.getTime() - 7 * 24 * 60 * 60 * 1000);
    const range = `From=${encodeURIComponent(from.toISOString())}&To=${encodeURIComponent(to.toISOString())}`;
    Promise.all([
      getApi(`/api/app/reporting/summary?${range}`) as Promise<LiveReportSummary>,
      getApi(`/api/app/reporting/daily-series?${range}`) as Promise<LiveDailyPoint[]>,
    ])
      .then(([summaryRes, seriesRes]) => {
        if (cancelled) return;
        setLive({ summary: summaryRes, series: Array.isArray(seriesRes) ? seriesRes : [] });
      })
      .catch(() => {
        if (!cancelled) setLiveError("Couldn't load live team stats — showing demo data.");
      })
      .finally(() => {
        if (!cancelled) setLiveLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const liveSummary = live?.summary ?? null;
  const trendData =
    live && live.series.length > 0
      ? live.series.map((d) => ({
          day: new Date(d.date).toLocaleDateString(undefined, { weekday: "short" }),
          tracked: Math.round(d.trackedMinutes),
          productive: Math.round((d.trackedMinutes * d.averageProductivity) / 100),
        }))
      : weeklyTrend;

  const dist = projectDistribution();
  const leaderboard = [...users]
    .filter((u) => u.productivity > 0)
    .sort((a, b) => b.trackedToday - a.trackedToday)
    .slice(0, 5);
  const feed = activities.slice(0, 6);

  return (
    <div className="space-y-6">
      <Reveal>
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="page-title">{greeting(userName)}</h1>
            <Badge tone="success" className="gap-1.5">
              <span className="h-1.5 w-1.5 rounded-full bg-success live-dot" />
              {liveSummary ? `${liveSummary.perUser.length} active this week` : `${summary.activeMembers} tracking now`}
            </Badge>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            {isOwner ? "Organization overview for today." : "What your team is working on today."}
          </p>
          {liveLoading && <p className="mt-1 text-xs text-muted-foreground/70">Loading live team stats…</p>}
          {liveError && <p className="mt-1 text-xs text-muted-foreground/70">{liveError}</p>}
        </div>
        <div className="flex items-center gap-2">
          <Link href="/reports/weekly">
            <Button variant="outline" size="md">
              <Sparkles className="h-4 w-4" /> Weekly report
            </Button>
          </Link>
          <Link href="/activities">
            <Button size="md">
              View activity <ArrowRight className="h-4 w-4" />
            </Button>
          </Link>
        </div>
      </div>
      </Reveal>

      <Reveal delay={60}>
        <SetupChecklist isOwner={isOwner} />
      </Reveal>

      <Stagger className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4" baseDelay={55}>
        <StatCard
          label={liveSummary ? "Tracked (7d)" : "Tracked today"}
          value={liveSummary ? formatDuration(Math.round(liveSummary.totalTrackedMinutes)) : formatDuration(summary.totalTrackedToday)}
          icon={Clock}
          accent={brand.primary}
          sub="across all members"
        />
        <StatCard
          label="Active members"
          value={liveSummary ? String(liveSummary.perUser.length) : `${summary.activeMembers}/${summary.totalMembers}`}
          icon={Users}
          accent={brand.info}
          sub={liveSummary ? "active in last 7 days" : "currently online"}
        />
        <StatCard
          label="Avg productivity"
          value={`${liveSummary ? Math.round(liveSummary.averageProductivity) : summary.avgProductivity}%`}
          icon={Gauge}
          accent={brand.success}
          sub="team average"
        />
        <StatCard
          label={liveSummary ? "Activities (7d)" : "Captures today"}
          value={liveSummary ? String(liveSummary.totalActivities) : String(summary.screenshotsToday)}
          icon={Camera}
          accent={brand.pink}
          sub={liveSummary ? "tracked sessions" : "screen captures"}
        />
      </Stagger>

      <Reveal delay={120}>
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <Card className="card-elev-lg lg:col-span-2">
          <CardHeader>
            <CardTitle>Activity this week</CardTitle>
            <div className="flex items-center gap-4 text-xs text-muted-foreground">
              <span className="flex items-center gap-1.5"><span className="h-2 w-2 rounded-full bg-primary" /> Tracked</span>
              <span className="flex items-center gap-1.5"><span className="h-2 w-2 rounded-full bg-success" /> Productive</span>
            </div>
          </CardHeader>
          <CardContent><ActivityTrendChart data={trendData} /></CardContent>
        </Card>

        <Card>
          <CardHeader><CardTitle>Time by project</CardTitle></CardHeader>
          <CardContent>
            <ProjectDonut data={dist} />
            <div className="mt-3 space-y-2">
              {dist.slice(0, 4).map((d) => (
                <div key={d.name} className="flex items-center justify-between text-sm">
                  <span className="flex items-center gap-2 truncate">
                    <span className="h-2.5 w-2.5 rounded-full" style={{ background: d.color }} />
                    <span className="truncate text-muted-foreground">{d.name}</span>
                  </span>
                  <span className="font-medium">{formatDuration(d.value)}</span>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      </div>
      </Reveal>

      <Reveal delay={180}>
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <Card className="card-quiet">
          <CardHeader><CardTitle>Most used apps</CardTitle><Badge tone="muted">This week</Badge></CardHeader>
          <CardContent><TopAppsChart data={topApps} /></CardContent>
        </Card>
        <Card className="card-quiet">
          <CardHeader><CardTitle>Today&apos;s focus by hour</CardTitle><Badge tone="muted">Minutes tracked</Badge></CardHeader>
          <CardContent><HourlyChart data={hourlyToday} /></CardContent>
        </Card>
      </div>
      </Reveal>

      <Reveal delay={220}>
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>Team leaderboard</CardTitle>
            <Link href="/team" className="text-xs text-primary hover:underline">View all</Link>
          </CardHeader>
          <CardContent className="space-y-1">
            {leaderboard.map((u, i) => (
              <div key={u.id} className="flex items-center gap-3 rounded-lg p-2 hover:bg-muted/60">
                <span className="w-5 text-center text-sm font-semibold text-muted-foreground">{i + 1}</span>
                <Avatar name={u.name} size="sm" status={u.status} />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium">{u.name}</div>
                  <div className="truncate text-xs text-muted-foreground">{u.designation}</div>
                </div>
                <span className="text-sm font-medium">{formatDuration(u.trackedToday)}</span>
                <Ring value={u.productivity} size={36} stroke={4} />
              </div>
            ))}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Live activity feed</CardTitle>
            <Link href="/activities" className="text-xs text-primary hover:underline">View all</Link>
          </CardHeader>
          <CardContent className="space-y-3">
            {feed.map((a) => {
              const u = userById(a.userId);
              const p = projectById(a.projectId);
              return (
                <div key={a.id} className="flex items-center gap-3">
                  <ScreenMockView screen={a.screen} className="h-12 w-20 shrink-0" />
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-sm">
                      <span className="font-medium">{u?.name}</span>{" "}
                      <span className="text-muted-foreground">— {a.description}</span>
                    </div>
                    <div className="mt-0.5 flex items-center gap-2 text-xs text-muted-foreground">
                      <span className="h-2 w-2 rounded-full" style={{ background: p?.color }} />
                      <span className="truncate">{p?.title}</span>
                      <span>·</span>
                      <span>{agoLabel(a.endedAt)}</span>
                    </div>
                  </div>
                  {a.online && <Badge tone="success">live</Badge>}
                </div>
              );
            })}
          </CardContent>
        </Card>
      </div>
      </Reveal>
    </div>
  );
}
