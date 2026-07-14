"use client";

import { useState } from "react";
import Link from "next/link";
import { Clock, CalendarRange, Users, Camera, FolderKanban } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Select } from "@/components/ui/input";
import { StatCard } from "@/components/dashboard/stat-card";
import { ScreenMockView } from "@/components/screen-mock";
import { activitiesForProject, NOW, projects, userById } from "@/lib/tenant-data";
import { formatDuration } from "@/lib/utils";
import type { User } from "@/lib/types";

function agoLabel(iso: string) {
  const mins = Math.round((NOW.getTime() - new Date(iso).getTime()) / 60000);
  if (mins < 60) return `${mins}m ago`;
  const h = Math.floor(mins / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

export function ClientDashboard({ user }: { user: User }) {
  const myProjects = projects.filter((p) => p.memberIds.includes(user.id));
  const [projectId, setProjectId] = useState(myProjects[0]?.id ?? "");
  const project = myProjects.find((p) => p.id === projectId) ?? myProjects[0];

  if (!project) {
    return (
      <Card className="flex flex-col items-center justify-center gap-3 py-20 text-center">
        <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted text-muted-foreground">
          <FolderKanban className="h-6 w-6" />
        </div>
        <div>
          <p className="font-medium">No project shared with you yet</p>
          <p className="mt-1 text-sm text-muted-foreground">Your workspace admin will grant you access to a project.</p>
        </div>
      </Card>
    );
  }

  const shots = activitiesForProject(project.id);
  const members = project.memberIds
    .map((id) => userById(id))
    .filter((u): u is User => !!u && u.role !== "client");

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-2xl font-bold tracking-tight">Welcome, {user.name.split(" ")[0]}</h1>
            <Badge tone="info">Client view</Badge>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">Progress on the projects shared with you.</p>
        </div>
        {myProjects.length > 1 && (
          <Select value={projectId} onChange={(e) => setProjectId(e.target.value)} className="w-auto min-w-48">
            {myProjects.map((p) => (
              <option key={p.id} value={p.id}>{p.title}</option>
            ))}
          </Select>
        )}
      </div>

      {/* Project header */}
      <Card>
        <CardContent className="flex flex-wrap items-center gap-4 p-5">
          <div className="flex h-12 w-12 items-center justify-center rounded-xl text-lg font-bold" style={{ background: `${project.color}22`, color: project.color }}>
            {project.title.charAt(0)}
          </div>
          <div className="min-w-0 flex-1">
            <div className="text-lg font-semibold">{project.title}</div>
            <div className="truncate text-sm text-muted-foreground">{project.description}</div>
          </div>
          <Badge tone="success">On track</Badge>
        </CardContent>
      </Card>

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <StatCard label="This week" value={formatDuration(project.loggedThisWeek)} icon={Clock} accent="#6d5efc" sub="logged" />
        <StatCard label="This month" value={formatDuration(project.loggedThisMonth)} icon={CalendarRange} accent="#0ea5e9" sub="logged" />
        <StatCard label="Total logged" value={formatDuration(project.loggedTotal)} icon={Clock} accent="#22c55e" sub="all time" />
        <StatCard label="Team members" value={String(members.length)} icon={Users} accent="#ec4899" sub="working on it" />
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        {/* Team working on it */}
        <Card>
          <CardHeader><CardTitle>Team on this project</CardTitle></CardHeader>
          <CardContent className="space-y-1">
            {members.map((u) => (
              <div key={u.id} className="flex items-center gap-3 rounded-lg p-2 hover:bg-muted/60">
                <Avatar name={u.name} size="sm" status={u.status} />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium">{u.name}</div>
                  <div className="truncate text-xs text-muted-foreground">{u.designation}</div>
                </div>
                <span className="text-sm font-medium">{formatDuration(u.trackedToday)}</span>
              </div>
            ))}
          </CardContent>
        </Card>

        {/* Recent screenshots */}
        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle className="flex items-center gap-2"><Camera className="h-4 w-4 text-muted-foreground" /> Recent screenshots</CardTitle>
            <Link href="/screenshots" className="text-xs text-primary hover:underline">View all</Link>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
              {shots.slice(0, 6).map((a) => {
                const u = userById(a.userId);
                return (
                  <div key={a.id} className="overflow-hidden rounded-lg border border-border">
                    <ScreenMockView screen={a.screen} title={a.activeWindows[0]?.windowTitle} className="aspect-video w-full rounded-none" />
                    <div className="flex items-center gap-2 p-2">
                      <Avatar name={u?.name ?? ""} size="sm" />
                      <span className="truncate text-xs">{u?.name?.split(" ")[0]}</span>
                      <span className="ml-auto text-[11px] text-muted-foreground">{agoLabel(a.endedAt)}</span>
                    </div>
                  </div>
                );
              })}
            </div>
            {shots.length === 0 && <p className="text-sm text-muted-foreground">No screenshots captured yet.</p>}
          </CardContent>
        </Card>
      </div>

      {/* Recent updates */}
      <Card>
        <CardHeader><CardTitle>Recent updates</CardTitle></CardHeader>
        <CardContent className="space-y-3">
          {shots.slice(0, 6).map((a) => {
            const u = userById(a.userId);
            return (
              <div key={a.id} className="flex items-center gap-3 border-b border-border/60 pb-3 last:border-0 last:pb-0">
                <Avatar name={u?.name ?? ""} size="sm" status={u?.status} />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm">
                    <span className="font-medium">{u?.name}</span>{" "}
                    <span className="text-muted-foreground">— {a.description}</span>
                  </div>
                  <div className="text-xs text-muted-foreground">{agoLabel(a.endedAt)}</div>
                </div>
              </div>
            );
          })}
        </CardContent>
      </Card>
    </div>
  );
}
