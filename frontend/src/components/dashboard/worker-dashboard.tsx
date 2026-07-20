"use client";

import Link from "next/link";
import { Clock, Gauge, FolderKanban, Activity as ActivityIcon, ArrowRight } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Ring } from "@/components/ui/ring";
import { Progress } from "@/components/ui/progress";
import { Button } from "@/components/ui/button";
import { StatCard } from "@/components/dashboard/stat-card";
import { ScreenMockView } from "@/components/screen-mock";
import { activitiesForUser, NOW, productivitySplit, projectById, projects } from "@/lib/tenant-data";
import { brand, greeting } from "@/lib/brand";
import { formatDuration } from "@/lib/utils";
import { Reveal, Stagger } from "@/components/motion/reveal";
import type { User } from "@/lib/types";

function agoLabel(iso: string) {
  const mins = Math.round((NOW.getTime() - new Date(iso).getTime()) / 60000);
  if (mins < 60) return `${mins}m ago`;
  const h = Math.floor(mins / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

/** Personal weekly shape derived from this member's trackedToday — not team aggregates. */
function personalWeek(trackedToday: number) {
  const factors = [0.95, 1.05, 0.88, 1.1, 1.0, 0.3, 0.12];
  const days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
  return days.map((day, i) => {
    const tracked = Math.max(0, Math.round(trackedToday * factors[i]));
    return { day, tracked, productive: Math.round(tracked * 0.78) };
  });
}

export function WorkerDashboard({ user }: { user: User }) {
  const myActivities = activitiesForUser(user.id);
  const myProjects = projects.filter((p) => !p.archived && p.memberIds.includes(user.id));
  const feed = myActivities.slice(0, 6);
  const split = productivitySplit.find((s) => s.userId === user.id);
  const splitTotal = split ? split.productive + split.neutral + split.unproductive : 0;
  const week = personalWeek(user.trackedToday);

  // Top apps from this user's own sessions
  const appMins = new Map<string, number>();
  for (const a of myActivities) {
    for (const w of a.activeWindows) {
      appMins.set(w.appName, (appMins.get(w.appName) ?? 0) + Math.round(w.seconds / 60));
    }
  }
  const myApps = [...appMins.entries()]
    .sort((a, b) => b[1] - a[1])
    .slice(0, 5);

  return (
    <div className="space-y-6">
      <Reveal>
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="page-title">{greeting(user.name)}</h1>
            {user.status === "active" && (
              <Badge tone="success" className="gap-1.5">
                <span className="h-1.5 w-1.5 rounded-full bg-success live-dot" /> Tracking now
              </Badge>
            )}
          </div>
          <p className="mt-1 text-sm text-muted-foreground">Your own activity only — not the rest of the team.</p>
        </div>
        <div className="flex gap-2">
          <Link href="/settings">
            <Button variant="outline" size="md">Privacy</Button>
          </Link>
          <Link href="/timesheet">
            <Button size="md">My timesheet <ArrowRight className="h-4 w-4" /></Button>
          </Link>
        </div>
      </div>
      </Reveal>

      <Stagger className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4" baseDelay={55}>
        <StatCard label="Tracked today" value={formatDuration(user.trackedToday)} icon={Clock} accent={brand.primary} sub="your time" />
        <StatCard label="Activity level" value={`${user.productivity}%`} icon={Gauge} accent={brand.success} sub="today" />
        <StatCard label="My projects" value={String(myProjects.length)} icon={FolderKanban} accent={brand.info} sub="active" />
        <StatCard label="Sessions" value={String(myActivities.length)} icon={ActivityIcon} accent={brand.pink} sub="captured" />
      </Stagger>

      <Reveal delay={100}>
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <Card className="card-elev-lg lg:col-span-2">
          <CardHeader>
            <CardTitle>Your week (minutes)</CardTitle>
            <Badge tone="muted">Derived from your tracked time</Badge>
          </CardHeader>
          <CardContent className="space-y-2">
            {week.map((d) => (
              <div key={d.day} className="flex items-center gap-3 text-sm">
                <span className="w-8 text-xs text-muted-foreground">{d.day}</span>
                <div className="h-2 flex-1 overflow-hidden rounded-full bg-muted">
                  <div
                    className="h-full rounded-full bg-primary"
                    style={{ width: `${Math.min(100, (d.tracked / Math.max(user.trackedToday * 1.2, 1)) * 100)}%` }}
                  />
                </div>
                <span className="w-14 text-right tabular-nums text-xs text-muted-foreground">{formatDuration(d.tracked)}</span>
              </div>
            ))}
          </CardContent>
        </Card>

        <Card>
          <CardHeader><CardTitle>Today&apos;s focus</CardTitle></CardHeader>
          <CardContent className="flex flex-col items-center gap-4 py-4">
            <Ring value={user.productivity} size={132} />
            {split && (
              <div className="w-full space-y-2 text-sm">
                <SplitRow label="Focused" value={split.productive} total={splitTotal} color="#22c55e" />
                <SplitRow label="Neutral" value={split.neutral} total={splitTotal} color="#0ea5e9" />
                <SplitRow label="Distracted" value={split.unproductive} total={splitTotal} color="#ef4444" />
              </div>
            )}
          </CardContent>
        </Card>
      </div>
      </Reveal>

      <Reveal delay={140}>
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader><CardTitle>Apps you used</CardTitle><Badge tone="muted">From your sessions</Badge></CardHeader>
          <CardContent className="space-y-3">
            {myApps.map(([app, mins]) => (
              <div key={app} className="flex items-center justify-between gap-3 text-sm">
                <span className="truncate font-medium">{app}</span>
                <span className="tabular-nums text-muted-foreground">{formatDuration(mins)}</span>
              </div>
            ))}
            {myApps.length === 0 && <p className="text-sm text-muted-foreground">No app signals yet.</p>}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>My projects</CardTitle>
            <Link href="/projects" className="text-xs text-primary hover:underline">View all</Link>
          </CardHeader>
          <CardContent className="space-y-3">
            {myProjects.map((p) => (
              <Link key={p.id} href={`/projects/${p.id}`} className="flex items-center gap-3 rounded-lg p-2 hover:bg-muted/60">
                <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg text-sm font-bold" style={{ background: `${p.color}22`, color: p.color }}>
                  {p.title[0]}
                </span>
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
      </div>
      </Reveal>

      <Reveal delay={180}>
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
          {feed.length === 0 && <p className="text-sm text-muted-foreground">No activity captured yet — install the desktop agent to start.</p>}
        </CardContent>
      </Card>
      </Reveal>
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
