"use client";

import { useParams } from "next/navigation";
import Link from "next/link";
import {
  ArrowLeft,
  Clock,
  CalendarDays,
  Users,
  Timer,
  Camera,
  Video,
  Keyboard,
  MousePointerClick,
  AppWindow,
  ListTree,
  Pencil,
  Archive,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Ring } from "@/components/ui/ring";
import { ScreenMockView } from "@/components/screen-mock";
import { activitiesForProject, projectById, userById, NOW } from "@/lib/tenant-data";
import { formatDuration } from "@/lib/utils";
import type { TrackingPermissions } from "@/lib/types";

const permMeta: { key: keyof TrackingPermissions; label: string; icon: typeof Camera }[] = [
  { key: "screenshot", label: "Screenshots", icon: Camera },
  { key: "webcam", label: "Webcam", icon: Video },
  { key: "keyboard", label: "Keyboard", icon: Keyboard },
  { key: "mouse", label: "Mouse", icon: MousePointerClick },
  { key: "activeWindow", label: "Active window", icon: AppWindow },
  { key: "runningPrograms", label: "Running programs", icon: ListTree },
];

function agoLabel(iso: string) {
  const mins = Math.round((NOW.getTime() - new Date(iso).getTime()) / 60000);
  if (mins < 60) return `${mins}m ago`;
  const h = Math.floor(mins / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

export default function ProjectDetailPage() {
  const { id } = useParams<{ id: string }>();
  const project = projectById(id);

  if (!project) {
    return (
      <div className="flex flex-col items-center gap-4 py-20 text-center">
        <p className="text-muted-foreground">Project not found.</p>
        <Link href="/projects">
          <Button variant="outline">
            <ArrowLeft className="h-4 w-4" /> Back to projects
          </Button>
        </Link>
      </div>
    );
  }

  const members = project.memberIds.map((mid) => userById(mid)).filter(Boolean);
  const acts = activitiesForProject(project.id);

  const memberStats = members.map((u) => {
    const ua = acts.filter((a) => a.userId === u!.id);
    const minutes = ua.length * project.intervalMinutes;
    const prod = ua.length ? Math.round(ua.reduce((s, a) => s + a.productivity, 0) / ua.length) : u!.productivity;
    return { user: u!, minutes, prod, sessions: ua.length };
  });

  return (
    <div className="space-y-6">
      <Link href="/projects" className="inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground">
        <ArrowLeft className="h-4 w-4" /> Projects
      </Link>

      {/* Header */}
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="flex items-center gap-4">
          <div className="flex h-14 w-14 items-center justify-center rounded-2xl text-white" style={{ background: project.color }}>
            <span className="text-xl font-bold">{project.title[0]}</span>
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h1 className="text-2xl font-bold tracking-tight">{project.title}</h1>
              {project.archived && <Badge tone="muted">Archived</Badge>}
            </div>
            <p className="mt-0.5 text-sm text-muted-foreground">{project.description}</p>
          </div>
        </div>
        <div className="flex gap-2">
          <Button variant="outline"><Pencil className="h-4 w-4" /> Edit</Button>
          <Button variant="outline"><Archive className="h-4 w-4" /> Archive</Button>
        </div>
      </div>

      {/* Stats */}
      <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
        {[
          { label: "This week", value: formatDuration(project.loggedThisWeek), icon: Clock },
          { label: "This month", value: formatDuration(project.loggedThisMonth), icon: CalendarDays },
          { label: "Total logged", value: formatDuration(project.loggedTotal), icon: Timer },
          { label: "Members", value: String(members.length), icon: Users },
        ].map((s) => (
          <Card key={s.label} className="p-4">
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-muted text-muted-foreground">
                <s.icon className="h-5 w-5" />
              </div>
              <div>
                <div className="text-lg font-bold">{s.value}</div>
                <div className="text-xs text-muted-foreground">{s.label}</div>
              </div>
            </div>
          </Card>
        ))}
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        {/* Members table */}
        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle>Team members</CardTitle>
            <Badge tone="muted">{members.length}</Badge>
          </CardHeader>
          <CardContent className="space-y-1">
            {memberStats.map(({ user, minutes, prod, sessions }) => (
              <div key={user.id} className="flex items-center gap-3 rounded-lg p-2 hover:bg-muted/60">
                <Avatar name={user.name} size="md" status={user.status} />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium">{user.name}</div>
                  <div className="truncate text-xs text-muted-foreground">{user.designation}</div>
                </div>
                <div className="hidden text-right sm:block">
                  <div className="text-sm font-medium">{formatDuration(minutes)}</div>
                  <div className="text-xs text-muted-foreground">{sessions} sessions</div>
                </div>
                <Ring value={prod} size={40} stroke={4} />
              </div>
            ))}
          </CardContent>
        </Card>

        {/* Tracking settings */}
        <Card>
          <CardHeader>
            <CardTitle>Tracking settings</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex items-center justify-between rounded-lg bg-muted/50 p-3">
              <span className="text-sm text-muted-foreground">Snapshot interval</span>
              <span className="text-sm font-semibold">{project.intervalMinutes} min</span>
            </div>
            {permMeta.map(({ key, label, icon: Icon }) => (
              <div key={key} className="flex items-center justify-between">
                <span className="flex items-center gap-2 text-sm">
                  <Icon className="h-4 w-4 text-muted-foreground" /> {label}
                </span>
                <Badge tone={project.permissions[key] ? "success" : "muted"}>
                  {project.permissions[key] ? "On" : "Off"}
                </Badge>
              </div>
            ))}
          </CardContent>
        </Card>
      </div>

      {/* Recent activity */}
      <Card>
        <CardHeader>
          <CardTitle>Recent activity</CardTitle>
          <Link href="/activities" className="text-xs text-primary hover:underline">View all</Link>
        </CardHeader>
        <CardContent>
          {acts.length === 0 ? (
            <p className="py-6 text-center text-sm text-muted-foreground">No activity recorded yet.</p>
          ) : (
            <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4">
              {acts.slice(0, 8).map((a) => {
                const u = userById(a.userId);
                return (
                  <div key={a.id} className="space-y-2">
                    <ScreenMockView screen={a.screen} title={a.activeWindows[0]?.windowTitle} className="aspect-video w-full" />
                    <div className="flex items-center gap-2">
                      <Avatar name={u?.name ?? ""} size="sm" />
                      <div className="min-w-0">
                        <div className="truncate text-xs font-medium">{u?.name}</div>
                        <div className="truncate text-[11px] text-muted-foreground">{agoLabel(a.endedAt)}</div>
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
