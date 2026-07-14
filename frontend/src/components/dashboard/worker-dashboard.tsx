"use client";

import Link from "next/link";
import { Clock, Gauge, FolderKanban, Activity as ActivityIcon, ArrowRight } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Ring } from "@/components/ui/ring";
import { Progress } from "@/components/ui/progress";
import { Button } from "@/components/ui/button";
import { StatCard } from "@/components/dashboard/stat-card";
import { ActivityTrendChart, HourlyChart, TopAppsChart } from "@/components/dashboard/charts";
import { ScreenMockView } from "@/components/screen-mock";
import { activitiesForUser, hourlyToday, NOW, productivitySplit, projectById, projects, topApps, weeklyTrend } from "@/lib/tenant-data";
import { formatDuration } from "@/lib/utils";
import type { User } from "@/lib/types";

function agoLabel(iso: string) {
  const mins = Math.round((NOW.getTime() - new Date(iso).getTime()) / 60000);
  if (mins < 60) return `${mins}m ago`;
  const h = Math.floor(mins / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

export function WorkerDashboard({ user }: { user: User }) {
  const myActivities = activitiesForUser(user.id);
  const myProjects = projects.filter((p) => !p.archived && p.memberIds.includes(user.id));
  const feed = myActivities.slice(0, 6);
  const split = productivitySplit.find((s) => s.userId === user.id);
  const splitTotal = split ? split.productive + split.neutral + split.unproductive : 0;

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-2xl font-bold tracking-tight">Good afternoon, {user.name.split(" ")[0]}</h1>
            {user.status === "active" && (
              <Badge tone="success" className="gap-1.5">
                <span className="h-1.5 w-1.5 rounded-full bg-success live-dot" /> Tracking now
              </Badge>
            )}
          </div>
          <p className="mt-1 text-sm text-muted-foreground">Here&apos;s a summary of your own activity today.</p>
        </div>
        <Link href="/timesheet">
          <Button size="md">My timesheet <ArrowRight className="h-4 w-4" /></Button>
        </Link>
      </div>

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <StatCard label="Tracked today" value={formatDuration(user.trackedToday)} icon={Clock} trend={9} accent="#6d5efc" sub="your time" />
        <StatCard label="Productivity" value={`${user.productivity}%`} icon={Gauge} trend={4} accent="#22c55e" sub="today" />
        <StatCard label="My projects" value={String(myProjects.length)} icon={FolderKanban} accent="#0ea5e9" sub="active" />
        <StatCard label="Sessions" value={String(myActivities.length)} icon={ActivityIcon} accent="#ec4899" sub="captured" />
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle>Your activity this week</CardTitle>
            <div className="flex items-center gap-4 text-xs text-muted-foreground">
              <span className="flex items-center gap-1.5"><span className="h-2 w-2 rounded-full bg-primary" /> Tracked</span>
              <span className="flex items-center gap-1.5"><span className="h-2 w-2 rounded-full bg-success" /> Productive</span>
            </div>
          </CardHeader>
          <CardContent><ActivityTrendChart data={weeklyTrend} /></CardContent>
        </Card>

        <Card>
          <CardHeader><CardTitle>Today&apos;s focus</CardTitle></CardHeader>
          <CardContent className="flex flex-col items-center gap-4 py-4">
            <Ring value={user.productivity} size={132} />
            {split && (
              <div className="w-full space-y-2 text-sm">
                <SplitRow label="Productive" value={split.productive} total={splitTotal} color="#22c55e" />
                <SplitRow label="Neutral" value={split.neutral} total={splitTotal} color="#0ea5e9" />
                <SplitRow label="Unproductive" value={split.unproductive} total={splitTotal} color="#ef4444" />
              </div>
            )}
          </CardContent>
        </Card>
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader><CardTitle>Your most used apps</CardTitle><Badge tone="muted">This week</Badge></CardHeader>
          <CardContent><TopAppsChart data={topApps} /></CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>Your focus by hour</CardTitle><Badge tone="muted">Minutes</Badge></CardHeader>
          <CardContent><HourlyChart data={hourlyToday} /></CardContent>
        </Card>
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>My projects</CardTitle>
            <Link href="/projects" className="text-xs text-primary hover:underline">View all</Link>
          </CardHeader>
          <CardContent className="space-y-3">
            {myProjects.map((p) => (
              <Link key={p.id} href={`/projects/${p.id}`} className="flex items-center gap-3 rounded-lg p-2 hover:bg-muted/60">
                <span className="h-9 w-9 shrink-0 rounded-lg" style={{ background: `${p.color}22`, color: p.color }} />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium">{p.title}</div>
                  <div className="truncate text-xs text-muted-foreground">{formatDuration(p.loggedThisWeek)} this week</div>
                </div>
                <ArrowRight className="h-4 w-4 text-muted-foreground" />
              </Link>
            ))}
            {myProjects.length === 0 && <p className="text-sm text-muted-foreground">You&apos;re not on any active projects yet.</p>}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>My recent activity</CardTitle>
            <Link href="/activities" className="text-xs text-primary hover:underline">View all</Link>
          </CardHeader>
          <CardContent className="space-y-3">
            {feed.map((a) => {
              const p = projectById(a.projectId);
              return (
                <div key={a.id} className="flex items-center gap-3">
                  <ScreenMockView screen={a.screen} className="h-12 w-20 shrink-0" />
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-sm font-medium">{a.description}</div>
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
            {feed.length === 0 && <p className="text-sm text-muted-foreground">No activity captured yet.</p>}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

function SplitRow({ label, value, total, color }: { label: string; value: number; total: number; color: string }) {
  const pct = total ? Math.round((value / total) * 100) : 0;
  return (
    <div>
      <div className="mb-1 flex items-center justify-between text-xs">
        <span className="flex items-center gap-1.5"><span className="h-2 w-2 rounded-full" style={{ background: color }} /> {label}</span>
        <span className="text-muted-foreground">{formatDuration(value)}</span>
      </div>
      <Progress value={pct} color={color} />
    </div>
  );
}
