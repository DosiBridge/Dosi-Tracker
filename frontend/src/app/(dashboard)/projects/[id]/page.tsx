"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
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
  Archive,
  ArchiveRestore,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Ring } from "@/components/ui/ring";
import { PageStack } from "@/components/ui/page-header";
import { ScreenMockView } from "@/components/screen-mock";
import { useSession } from "@/components/session-provider";
import { activitiesForProject, projectById, userById } from "@/lib/tenant-data";
import { canViewProject } from "@/lib/scope";
import { formatDuration } from "@/lib/utils";
import { getApi, postApi } from "@/hooks/useApi";
import type { TrackingPermissions, UserStatus } from "@/lib/types";

const permMeta: { key: keyof TrackingPermissions; label: string; icon: typeof Camera }[] = [
  { key: "screenshot", label: "Screenshots", icon: Camera },
  { key: "webcam", label: "Webcam", icon: Video },
  { key: "keyboard", label: "Keyboard", icon: Keyboard },
  { key: "mouse", label: "Mouse", icon: MousePointerClick },
  { key: "activeWindow", label: "Active window", icon: AppWindow },
  { key: "runningPrograms", label: "Running programs", icon: ListTree },
];

/** Unified view-model fed either by the real API or by the mock demo data. */
interface ProjectView {
  id: string;
  title: string;
  description: string;
  color: string;
  intervalMinutes: number;
  archived: boolean;
  permissions: TrackingPermissions;
  live: boolean; // true when backed by the real API
}

interface MemberRow {
  id: string;
  name: string;
  designation: string;
  minutes: number;
  prod: number;
  sessions: number;
  status?: UserStatus;
}

interface ActivityRow {
  id: string;
  userName: string;
  windowTitle?: string;
  endedAt: string;
  screen: { app: string; kind: string; accent: string };
}

function agoLabel(iso: string, now: Date) {
  const mins = Math.round((now.getTime() - new Date(iso).getTime()) / 60000);
  if (mins < 60) return `${Math.max(mins, 0)}m ago`;
  const h = Math.floor(mins / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

const minutesBetween = (start: string, end: string) =>
  Math.max(0, (new Date(end).getTime() - new Date(start).getTime()) / 60000);

export default function ProjectDetailPage() {
  const { id } = useParams<{ id: string }>();
  const { user } = useSession();

  const [apiProject, setApiProject] = useState<any | null>(null);
  const [apiMembers, setApiMembers] = useState<any[]>([]);
  const [apiActivities, setApiActivities] = useState<any[]>([]);
  const [apiChecked, setApiChecked] = useState(false);
  const [busy, setBusy] = useState(false);

  const loadFromApi = useCallback(async () => {
    try {
      const project = await getApi(`/api/app/project/${id}`);
      const [members, activities] = await Promise.all([
        getApi(`/api/app/team/project-members?projectId=${id}`).catch(() => []),
        getApi(`/api/app/activity?ProjectId=${id}&MaxResultCount=200`).catch(() => []),
      ]);
      setApiProject(project);
      setApiMembers(Array.isArray(members) ? members : []);
      setApiActivities(Array.isArray(activities) ? activities : []);
    } catch {
      setApiProject(null); // fall back to the demo dataset below
    } finally {
      setApiChecked(true);
    }
  }, [id]);

  useEffect(() => {
    if (typeof window !== "undefined" && localStorage.getItem("dosi-token")) {
      loadFromApi();
    } else {
      setApiChecked(true);
    }
  }, [loadFromApi]);

  const mockProject = projectById(id);
  const now = useMemo(() => new Date(), []);

  const view: ProjectView | null = apiProject
    ? {
        id: apiProject.id,
        title: apiProject.title,
        description: apiProject.description ?? "",
        color: apiProject.color ?? "#006bff",
        intervalMinutes: apiProject.intervalMinutes ?? 10,
        archived: !!apiProject.isArchived,
        permissions: {
          screenshot: !!apiProject.allowScreenshot,
          webcam: !!apiProject.allowWebcam,
          keyboard: !!apiProject.allowKeyboard,
          mouse: !!apiProject.allowMouse,
          activeWindow: !!apiProject.allowActiveWindow,
          runningPrograms: !!apiProject.allowRunningPrograms,
        },
        live: true,
      }
    : mockProject && canViewProject(user, mockProject)
      ? {
          id: mockProject.id,
          title: mockProject.title,
          description: mockProject.description,
          color: mockProject.color,
          intervalMinutes: mockProject.intervalMinutes,
          archived: mockProject.archived,
          permissions: mockProject.permissions,
          live: false,
        }
      : null;

  if (!apiChecked) {
    return <p className="py-20 text-center text-sm text-muted-foreground">Loading project…</p>;
  }

  if (!view) {
    return (
      <div className="flex flex-col items-center gap-4 py-20 text-center">
        <p className="text-muted-foreground">
          {mockProject ? "You don’t have access to this project." : "Project not found."}
        </p>
        <Link href="/projects">
          <Button variant="outline">
            <ArrowLeft className="h-4 w-4" /> Back to projects
          </Button>
        </Link>
      </div>
    );
  }

  // ---- Build member + activity rows from whichever source is active ----
  let memberRows: MemberRow[];
  let activityRows: ActivityRow[];
  let weekMinutes: number, monthMinutes: number, totalMinutes: number;

  if (view.live) {
    const nameFor = (userId: string) =>
      userId === user.id ? user.name : `Member ${userId.slice(0, 8)}`;

    memberRows = apiMembers.map((m) => {
      const acts = apiActivities.filter((a) => a.userId === m.userId);
      const minutes = Math.round(acts.reduce((s, a) => s + minutesBetween(a.startedAt, a.endedAt), 0));
      const prod = acts.length
        ? Math.round(acts.reduce((s, a) => s + (a.productivity ?? 0), 0) / acts.length)
        : 0;
      return {
        id: m.userId,
        name: nameFor(m.userId),
        designation: m.role ?? "Member",
        minutes,
        prod,
        sessions: acts.length,
      };
    });

    activityRows = apiActivities.slice(0, 8).map((a) => {
      const win = a.activeWindowsJson ? JSON.parse(a.activeWindowsJson) : [];
      return {
        id: a.id,
        userName: nameFor(a.userId),
        windowTitle: win[0]?.windowTitle,
        endedAt: a.endedAt,
        screen: { app: win[0]?.appName ?? "Desktop", kind: "desktop", accent: view.color },
      };
    });

    const sumSince = (since: Date) =>
      Math.round(
        apiActivities
          .filter((a) => new Date(a.startedAt) >= since)
          .reduce((s, a) => s + minutesBetween(a.startedAt, a.endedAt), 0),
      );
    weekMinutes = sumSince(new Date(now.getTime() - 7 * 86400_000));
    monthMinutes = sumSince(new Date(now.getTime() - 30 * 86400_000));
    totalMinutes = Math.round(
      apiActivities.reduce((s, a) => s + minutesBetween(a.startedAt, a.endedAt), 0),
    );
  } else {
    const members = (mockProject?.memberIds ?? []).map((mid) => userById(mid)).filter(Boolean);
    const acts = activitiesForProject(view.id);
    memberRows = members.map((u) => {
      const ua = acts.filter((a) => a.userId === u!.id);
      const minutes = ua.length * view.intervalMinutes;
      const prod = ua.length
        ? Math.round(ua.reduce((s, a) => s + a.productivity, 0) / ua.length)
        : u!.productivity;
      return {
        id: u!.id,
        name: u!.name,
        designation: u!.designation,
        minutes,
        prod,
        sessions: ua.length,
        status: u!.status,
      };
    });
    activityRows = acts.slice(0, 8).map((a) => ({
      id: a.id,
      userName: userById(a.userId)?.name ?? "",
      windowTitle: a.activeWindows[0]?.windowTitle,
      endedAt: a.endedAt,
      screen: a.screen,
    }));
    weekMinutes = mockProject?.loggedThisWeek ?? 0;
    monthMinutes = mockProject?.loggedThisMonth ?? 0;
    totalMinutes = mockProject?.loggedTotal ?? 0;
  }

  const canManage = user.role === "owner" || user.role === "admin" || user.role === "host";

  async function toggleArchive() {
    if (!view || !view.live) return;
    setBusy(true);
    try {
      await postApi(`/api/app/project/${view.id}/${view.archived ? "unarchive" : "archive"}`, {});
      await loadFromApi();
    } catch (err) {
      console.error("Failed to toggle archive state", err);
    } finally {
      setBusy(false);
    }
  }

  return (
    <PageStack>
      <Link href="/projects" className="inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground">
        <ArrowLeft className="h-4 w-4" /> Projects
      </Link>

      <header className="flex flex-wrap items-start justify-between gap-4 border-b border-border/70 pb-5">
        <div className="flex items-center gap-4">
          <div className="flex h-14 w-14 items-center justify-center rounded-2xl text-white" style={{ background: view.color }}>
            <span className="font-display text-xl font-bold">{view.title[0]}</span>
          </div>
          <div>
            <div className="flex flex-wrap items-center gap-2">
              <h1 className="page-title">{view.title}</h1>
              {view.archived && <Badge tone="muted">Archived</Badge>}
            </div>
            <p className="mt-1.5 text-sm leading-relaxed text-muted-foreground">{view.description}</p>
          </div>
        </div>
        {view.live && canManage && (
          <div className="flex gap-2">
            <Button variant="outline" onClick={toggleArchive} disabled={busy}>
              {view.archived ? <ArchiveRestore className="h-4 w-4" /> : <Archive className="h-4 w-4" />}
              {view.archived ? "Unarchive" : "Archive"}
            </Button>
          </div>
        )}
      </header>

      {/* Stats */}
      <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
        {[
          { label: "This week", value: formatDuration(weekMinutes), icon: Clock },
          { label: "This month", value: formatDuration(monthMinutes), icon: CalendarDays },
          { label: "Total logged", value: formatDuration(totalMinutes), icon: Timer },
          { label: "Members", value: String(memberRows.length), icon: Users },
        ].map((s) => (
          <Card key={s.label} className="p-4">
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-muted text-muted-foreground">
                <s.icon className="h-5 w-5" />
              </div>
              <div>
                <div className="font-display text-lg font-bold tabular-nums">{s.value}</div>
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
            <Badge tone="muted">{memberRows.length}</Badge>
          </CardHeader>
          <CardContent className="space-y-1">
            {memberRows.length === 0 && (
              <p className="py-6 text-center text-sm text-muted-foreground">No members yet.</p>
            )}
            {memberRows.map((m) => (
              <div key={m.id} className="flex items-center gap-3 rounded-lg p-2 hover:bg-muted/60">
                <Avatar name={m.name} size="md" status={m.status ?? "active"} />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium">{m.name}</div>
                  <div className="truncate text-xs text-muted-foreground">{m.designation}</div>
                </div>
                <div className="hidden text-right sm:block">
                  <div className="text-sm font-medium">{formatDuration(m.minutes)}</div>
                  <div className="text-xs text-muted-foreground">{m.sessions} sessions</div>
                </div>
                <Ring value={m.prod} size={40} stroke={4} />
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
              <span className="text-sm font-semibold">{view.intervalMinutes} min</span>
            </div>
            {permMeta.map(({ key, label, icon: Icon }) => (
              <div key={key} className="flex items-center justify-between">
                <span className="flex items-center gap-2 text-sm">
                  <Icon className="h-4 w-4 text-muted-foreground" /> {label}
                </span>
                <Badge tone={view.permissions[key] ? "success" : "muted"}>
                  {view.permissions[key] ? "On" : "Off"}
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
          {activityRows.length === 0 ? (
            <p className="py-6 text-center text-sm text-muted-foreground">No activity recorded yet.</p>
          ) : (
            <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4">
              {activityRows.map((a) => (
                <div key={a.id} className="space-y-2">
                  <ScreenMockView screen={a.screen as any} title={a.windowTitle} className="aspect-video w-full" />
                  <div className="flex items-center gap-2">
                    <Avatar name={a.userName} size="sm" />
                    <div className="min-w-0">
                      <div className="truncate text-xs font-medium">{a.userName}</div>
                      <div className="truncate text-[11px] text-muted-foreground">{agoLabel(a.endedAt, now)}</div>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
    </PageStack>
  );
}
