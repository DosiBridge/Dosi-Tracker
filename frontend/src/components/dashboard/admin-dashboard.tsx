"use client";

import Link from "next/link";
import { Clock, Users, Gauge, Camera, ArrowRight, Sparkles } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Ring } from "@/components/ui/ring";
import { Button } from "@/components/ui/button";
import { StatCard } from "@/components/dashboard/stat-card";
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
import { formatDuration } from "@/lib/utils";

function agoLabel(iso: string) {
  const mins = Math.round((NOW.getTime() - new Date(iso).getTime()) / 60000);
  if (mins < 60) return `${mins}m ago`;
  const h = Math.floor(mins / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

export function AdminDashboard({ userName, isOwner }: { userName: string; isOwner: boolean }) {
  const dist = projectDistribution();
  const leaderboard = [...users]
    .filter((u) => u.productivity > 0)
    .sort((a, b) => b.trackedToday - a.trackedToday)
    .slice(0, 5);
  const feed = activities.slice(0, 6);

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-2xl font-bold tracking-tight">Good afternoon, {userName}</h1>
            <Badge tone="success" className="gap-1.5">
              <span className="h-1.5 w-1.5 rounded-full bg-success live-dot" />
              {summary.activeMembers} tracking now
            </Badge>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            {isOwner ? "Here's a complete overview of your organization today." : "Here's what your team is working on today."}
          </p>
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

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <StatCard label="Tracked today" value={formatDuration(summary.totalTrackedToday)} icon={Clock} trend={12} accent="#6d5efc" sub="across all members" />
        <StatCard label="Active members" value={`${summary.activeMembers}/${summary.totalMembers}`} icon={Users} trend={8} accent="#0ea5e9" sub="currently online" />
        <StatCard label="Avg productivity" value={`${summary.avgProductivity}%`} icon={Gauge} trend={-3} accent="#22c55e" sub="team average" />
        <StatCard label="Screenshots today" value={String(summary.screenshotsToday)} icon={Camera} trend={5} accent="#ec4899" sub="auto-captured" />
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle>Activity this week</CardTitle>
            <div className="flex items-center gap-4 text-xs text-muted-foreground">
              <span className="flex items-center gap-1.5"><span className="h-2 w-2 rounded-full bg-primary" /> Tracked</span>
              <span className="flex items-center gap-1.5"><span className="h-2 w-2 rounded-full bg-success" /> Productive</span>
            </div>
          </CardHeader>
          <CardContent><ActivityTrendChart data={weeklyTrend} /></CardContent>
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

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader><CardTitle>Most used apps</CardTitle><Badge tone="muted">This week</Badge></CardHeader>
          <CardContent><TopAppsChart data={topApps} /></CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>Today&apos;s focus by hour</CardTitle><Badge tone="muted">Minutes tracked</Badge></CardHeader>
          <CardContent><HourlyChart data={hourlyToday} /></CardContent>
        </Card>
      </div>

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
    </div>
  );
}
