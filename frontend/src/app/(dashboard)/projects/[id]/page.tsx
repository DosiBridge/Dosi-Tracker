"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useParams } from "next/navigation";
import Link from "next/link";
import {
  ArrowLeft,
  CalendarDays,
  CalendarRange,
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
  Pencil,
  UserPlus,
  UserMinus,
  Hourglass,
  EyeOff,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { PageStack } from "@/components/ui/page-header";
import { AppGlyph, ScreenMockView } from "@/components/screen-mock";
import { useSession } from "@/components/session-provider";
import { toast } from "@/components/toast";
import { EditProjectModal, type ProjectFormResult } from "@/components/projects/create-project-modal";
import { AddMemberDialog } from "@/components/projects/project-members";
import { ActivityRing } from "@/components/dashboard/activity-ring";
import { linkButtonClass } from "@/components/dashboard/link-button";
import {
  draftFromDemoProject,
  draftFromProject,
  idleLabel,
  permissionsFromDto,
  type ProjectDraft,
} from "@/components/projects/project-form";
import { editRulesFor, memberNameOf, membershipErrorText, projectRoleLabel, projectRoleOf } from "@/components/projects/membership";
import { ProjectTasksSection } from "@/components/projects/project-tasks";
import { AiPeriodSummaryCard } from "@/components/ai/ai-period-summary-card";
import { requestErrorText } from "@/components/layout/request-error";
import { activitiesForProject, projectById, userById, users } from "@/lib/tenant-data";
import { averageProductivity, durationMinutes } from "@/lib/metrics";
import { canViewProject } from "@/lib/scope";
import { agoLabel, currentWorkspaceId, referenceNow } from "@/lib/live-session";
import { formatDuration } from "@/lib/utils";
import { addZonedDays, formatDayKey, zonedDayKey, zonedDayRange, zonedPeriod } from "@/lib/time-zone";
import { useReportingZone } from "@/hooks/use-reporting-zone";
import {
  MAX_REPORT_RANGE_DAYS,
  activitiesRoute,
  projectRoute,
  reportingRoute,
  type CreateProjectMemberDto,
  type CreateUpdateProjectDto,
  type ProjectDto,
  type ProjectMemberDto,
  type ReportSummaryDto,
} from "@/lib/api-contract";
import {
  addProjectMember,
  getApi,
  getProjectMembers,
  postApi,
  removeProjectMember,
  updateProject,
} from "@/hooks/useApi";
import { mapApiActivities, normalizeUtcIso } from "@/lib/live-dataset";
import type { Activity, TrackingPermissions, UserStatus } from "@/lib/types";

const permMeta: { key: keyof TrackingPermissions; label: string; icon: LucideIcon }[] = [
  { key: "screenshot", label: "Screenshots", icon: Camera },
  { key: "webcam", label: "Webcam", icon: Video },
  { key: "keyboard", label: "Keyboard activity", icon: Keyboard },
  { key: "mouse", label: "Mouse activity", icon: MousePointerClick },
  { key: "activeWindow", label: "Active window", icon: AppWindow },
  { key: "runningPrograms", label: "Running programs", icon: ListTree },
];

/** What the page shows about the project, from the API (live) or the demo dataset. */
interface ProjectView {
  id: string;
  title: string;
  description: string;
  color: string;
  archived: boolean;
  archivedAt: string | null;
  draft: ProjectDraft;
  permissions: TrackingPermissions;
  /** First day (reporting zone) the all-time total starts from. */
  sinceDayKey: string | null;
  live: boolean;
}

interface MemberRow {
  userId: string;
  /** The membership id (live), for removal. */
  membershipId: string | null;
  name: string;
  role: string;
  minutes: number | null;
  activity: number | null;
  status?: UserStatus;
}

interface ActivityRow {
  id: string;
  userName: string;
  windowTitle?: string;
  endedAt: string;
  screen: Activity["screen"];
}

interface LiveTotals {
  week: number;
  month: number;
  all: number;
  /** Per member over this month: minutes and activity %. */
  perUser: Map<string, { minutes: number; activity: number | null }>;
}

/** The project's first day in the reporting zone, or null for the server's empty creation date (0001-01-01). */
function createdDayKey(creationTime: string | null | undefined, timeZone: string): string | null {
  const iso = normalizeUtcIso(creationTime);
  if (!iso || Date.parse(iso) < Date.UTC(2000, 0, 1)) return null;
  return zonedDayKey(Date.parse(iso), timeZone);
}

function summaryTotals(summary: unknown): { minutes: number; perUser: LiveTotals["perUser"] } {
  const s = (summary ?? {}) as Partial<ReportSummaryDto>;
  const perUser: LiveTotals["perUser"] = new Map();
  for (const r of Array.isArray(s.perUser) ? s.perUser : []) {
    if (!r || typeof r.userId !== "string") continue;
    const activity = typeof r.averageProductivity === "number" ? Math.round(r.averageProductivity) : null;
    perUser.set(r.userId, { minutes: Math.round(Number(r.trackedMinutes) || 0), activity });
  }
  return { minutes: Math.round(Number(s.totalTrackedMinutes) || 0), perUser };
}

export default function ProjectDetailPage() {
  const { id } = useParams<{ id: string }>();
  const { user, isLive, workspace, refreshSession } = useSession();
  const zone = useReportingZone(isLive);
  const canManage = user.role === "owner" || user.role === "admin";

  const [apiProject, setApiProject] = useState<ProjectDto | null>(null);
  const [apiMembers, setApiMembers] = useState<ProjectMemberDto[] | null>(null);
  const [apiRecent, setApiRecent] = useState<Activity[]>([]);
  const [apiChecked, setApiChecked] = useState(false);
  const [apiError, setApiError] = useState<string | null>(null);
  const [totals, setTotals] = useState<{ data: LiveTotals | null; failed: boolean }>({ data: null, failed: false });
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState(false);
  const [confirmArchive, setConfirmArchive] = useState(false);
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<MemberRow | null>(null);
  // Demo workspace: changes made on this page last until the next reload.
  const [demoEdit, setDemoEdit] = useState<CreateUpdateProjectDto | null>(null);
  const [demoArchived, setDemoArchived] = useState<boolean | null>(null);
  const [demoMembers, setDemoMembers] = useState<string[] | null>(null);

  const loadFromApi = useCallback(async () => {
    setApiError(null);
    try {
      const project = (await getApi(projectRoute(id))) as ProjectDto;
      const [members, recent] = await Promise.all([
        getProjectMembers(id).catch(() => null),
        getApi(activitiesRoute({ projectId: id, maxResultCount: 8 })).catch(() => []),
      ]);
      setApiProject(project && typeof project === "object" ? project : null);
      setApiMembers(members ? members.filter((m) => m && typeof m.userId === "string") : null);
      // Guarded mapping: malformed window JSON or timestamps never crash the page.
      setApiRecent(mapApiActivities(recent).slice(0, 8));
    } catch (err) {
      setApiProject(null);
      setApiError(requestErrorText(err, "Couldn’t load this project from the server."));
    } finally {
      setApiChecked(true);
    }
  }, [id]);

  useEffect(() => {
    if (!isLive) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void loadFromApi();
  }, [isLive, loadFromApi]);

  // Live totals: this week, this month and since the project began (at most
  // the server's 366-day range), as calendar days of the reporting zone.
  const creationTime = apiProject?.creationTime ?? null;
  const loadTotals = useCallback(async () => {
    const now = new Date();
    const tz = zone.timeZone;
    const week = zonedPeriod("week", now, tz);
    const month = zonedPeriod("month", now, tz);
    const today = zonedDayKey(now, tz);
    const earliest = addZonedDays(today, -(MAX_REPORT_RANGE_DAYS - 1));
    const created = createdDayKey(creationTime, tz);
    const all = zonedDayRange(created && created > earliest ? (created > today ? today : created) : earliest, today, tz);
    const route = (from: Date, to: Date) => reportingRoute("summary", { from, to, projectId: id, timeZone: tz });
    try {
      const [w, m, a] = await Promise.all([getApi(route(week.from, week.to)), getApi(route(month.from, month.to)), getApi(route(all.from, all.to))]);
      const monthTotals = summaryTotals(m);
      setTotals({
        data: { week: summaryTotals(w).minutes, month: monthTotals.minutes, all: summaryTotals(a).minutes, perUser: monthTotals.perUser },
        failed: false,
      });
    } catch {
      setTotals({ data: null, failed: true });
    }
  }, [id, zone.timeZone, creationTime]);

  useEffect(() => {
    if (!isLive || !zone.ready || !apiProject) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void loadTotals();
  }, [isLive, zone.ready, apiProject, loadTotals]);

  const mockProject = projectById(id);
  const now = useMemo(() => referenceNow(), []);

  const view: ProjectView | null = useMemo(() => {
    if (apiProject) {
      const draft = draftFromProject(apiProject);
      return {
        id: apiProject.id,
        title: apiProject.title || "Untitled project",
        description: apiProject.description ?? "",
        color: draft.color,
        archived: !!apiProject.isArchived,
        archivedAt: normalizeUtcIso(apiProject.archivedAt),
        draft,
        permissions: permissionsFromDto(draft),
        sinceDayKey: createdDayKey(apiProject.creationTime, zone.timeZone),
        live: true,
      };
    }
    if (isLive || !mockProject || !canViewProject(user, mockProject)) return null;
    const draft = demoEdit ? { ...demoEdit, description: demoEdit.description ?? "" } : draftFromDemoProject(mockProject);
    return {
      id: mockProject.id,
      title: draft.title,
      description: draft.description,
      color: draft.color,
      archived: demoArchived ?? mockProject.archived,
      archivedAt: null,
      draft,
      permissions: permissionsFromDto(draft),
      sinceDayKey: mockProject.createdAt || null,
      live: false,
    };
  }, [apiProject, isLive, mockProject, user, demoEdit, demoArchived, zone.timeZone]);

  // The demo has nothing to load; a live project shows once its first read settled.
  if (isLive && !apiChecked) {
    return (
      <p role="status" className="py-20 text-center text-sm text-muted-foreground">
        Loading project…
      </p>
    );
  }

  if (!view && isLive && apiError) {
    return (
      <div className="flex flex-col items-center gap-4 py-20 text-center">
        <p role="alert" className="text-muted-foreground">
          {apiError}
        </p>
        <div className="flex gap-2">
          <Button onClick={() => void loadFromApi()}>Try again</Button>
          <Link href="/projects" className={linkButtonClass("outline", "md")}>
            <ArrowLeft aria-hidden className="h-4 w-4" /> Back to projects
          </Link>
        </div>
      </div>
    );
  }

  if (!view) {
    return (
      <div className="flex flex-col items-center gap-4 py-20 text-center">
        <p className="text-muted-foreground">{mockProject ? "You don’t have access to this project." : "Project not found."}</p>
        <Link href="/projects" className={linkButtonClass("outline", "md")}>
          <ArrowLeft aria-hidden className="h-4 w-4" /> Back to projects
        </Link>
      </div>
    );
  }

  // ---- Members, recent activity and totals from whichever source is active ----
  let memberRows: MemberRow[];
  let activityRows: ActivityRow[];
  let weekMinutes: number | null, monthMinutes: number | null, totalMinutes: number | null;

  if (view.live) {
    // Names: the session roster first, then the names newer servers put on each membership.
    memberRows = (apiMembers ?? []).map((m) => {
      const stats = totals.data?.perUser.get(m.userId);
      return {
        userId: m.userId,
        membershipId: m.id ?? null,
        name: m.userId === user.id ? user.name : memberNameOf(m, userById(m.userId)?.name),
        role: projectRoleLabel(m.role),
        minutes: totals.data ? (stats?.minutes ?? 0) : null,
        activity: stats?.activity ?? null,
        // Status only for people the roster knows; an unknown teammate gets no dot.
        status: userById(m.userId)?.status,
      };
    });
    activityRows = apiRecent.map((a) => ({
      id: a.id,
      userName: a.userId === user.id ? user.name : (userById(a.userId)?.name ?? memberNameOf({ userId: a.userId }, null)),
      windowTitle: a.activeWindows[0]?.windowTitle,
      endedAt: a.endedAt,
      screen: a.screen,
    }));
    weekMinutes = totals.data?.week ?? null;
    monthMinutes = totals.data?.month ?? null;
    totalMinutes = totals.data?.all ?? null;
  } else {
    const ids = demoMembers ?? mockProject?.memberIds ?? [];
    const acts = activitiesForProject(view.id);
    const monthStart = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 1)).getTime();
    // The demo's blocks cover its last week: members are counted over the same rolling 7 days as its
    // "This week" figure, so the rows add up to it (a live project counts them over the month).
    const demoWeekStart = now.getTime() - 7 * 86_400_000;
    memberRows = ids
      .map((mid) => userById(mid))
      .filter((u): u is NonNullable<typeof u> => !!u)
      .map((u) => {
        const mine = acts.filter((a) => a.userId === u.id && Date.parse(a.endedAt) >= demoWeekStart);
        return {
          userId: u.id,
          membershipId: null,
          name: u.name,
          role: u.designation,
          minutes: Math.round(mine.reduce((s, a) => s + durationMinutes(a), 0)),
          activity: mine.length ? averageProductivity(mine) : null,
          status: u.status,
        };
      });
    activityRows = acts.slice(0, 8).map((a) => ({
      id: a.id,
      userName: userById(a.userId)?.name ?? "",
      windowTitle: a.activeWindows[0]?.windowTitle,
      endedAt: a.endedAt,
      screen: a.screen,
    }));
    if (canManage) {
      weekMinutes = mockProject?.loggedThisWeek ?? 0;
      monthMinutes = mockProject?.loggedThisMonth ?? 0;
      totalMinutes = mockProject?.loggedTotal ?? 0;
    } else {
      // A member sees their own time, never the project's team total under "Your time".
      const mine = acts.filter((a) => a.userId === user.id);
      const sum = (since: number) =>
        Math.round(mine.filter((a) => Date.parse(a.endedAt) >= since).reduce((s, a) => s + durationMinutes(a), 0));
      const weekStart = now.getTime() - (((now.getUTCDay() + 6) % 7) * 86_400_000 + now.getTime() % 86_400_000);
      weekMinutes = sum(weekStart);
      monthMinutes = sum(monthStart);
      totalMinutes = sum(0);
    }
  }

  // People who see everyone's time (owners, administrators) get per-member figures;
  // anyone else sees their own totals and the names of the people they work with.
  const teamFigures = canManage;
  const project = view;
  /** The period the member rows add up over (see above). */
  const memberPeriod = project.live ? "this month" : "this week";
  const memberIds = new Set(memberRows.map((m) => m.userId));
  const sinceLabel = project.sinceDayKey ? `Since ${formatDayKey(project.sinceDayKey, { year: true })}` : "Last 12 months";
  const stats: { label: string; value: string; icon: LucideIcon; sub?: string }[] = [
    { label: teamFigures ? "This week" : "Your time this week", value: fmt(weekMinutes), icon: CalendarDays },
    { label: teamFigures ? "This month" : "Your time this month", value: fmt(monthMinutes), icon: CalendarRange },
    { label: teamFigures ? "Total logged" : "Your total", value: fmt(totalMinutes), icon: Timer, sub: project.live ? sinceLabel : undefined },
    { label: "Members", value: String(memberRows.length), icon: Users },
  ];

  /** "Project pulse" covers the last 7 days, today included, in the reporting zone. */
  const pulsePeriod = zonedPeriod("7d", now, zone.timeZone);

  const isPrivileged = user.role === "owner" || user.role === "admin";
  const myProjectMembership = apiMembers?.find((m) => m.userId === user.id);
  const isProjectMember = Boolean(myProjectMembership);
  const isClientMember = myProjectMembership
    ? projectRoleOf(myProjectMembership.role) === "Client"
    : user.role === "client";
  const canMutateTasks = !project.archived && (isPrivileged || (isProjectMember && !isClientMember));

  async function toggleArchive() {
    const wasArchived = project.archived;
    setConfirmArchive(false);
    if (!project.live) {
      setDemoArchived(!wasArchived);
      toast({
        title: wasArchived ? "Project restored" : "Project archived",
        description: "Demo workspace — this change lasts until you reload.",
      });
      return;
    }
    setBusy(true);
    try {
      await postApi(`${projectRoute(project.id)}/${wasArchived ? "unarchive" : "archive"}`, {});
      await loadFromApi();
      void refreshSession({ silent: true });
      toast({
        title: wasArchived ? "Project restored" : "Project archived",
        description: wasArchived
          ? "Members can track time on it again; their agents offer it within five minutes."
          : "Members can no longer track time on it. Blocks recorded before now still upload.",
      });
    } catch (err) {
      toast({
        tone: "danger",
        title: wasArchived ? "Couldn't restore this project" : "Couldn't archive this project",
        description: requestErrorText(err, "The change wasn't saved."),
      });
    } finally {
      setBusy(false);
    }
  }

  async function saveEdit({ dto }: ProjectFormResult) {
    if (!project.live) {
      setDemoEdit(dto);
      toast({ title: "Project updated", description: "Demo workspace — this change lasts until you reload." });
      return;
    }
    try {
      const saved = await updateProject(project.id, dto);
      setApiProject((prev) => (saved && typeof saved === "object" ? { ...(prev ?? {}), ...saved } : prev));
    } catch (err) {
      throw new Error(requestErrorText(err, "Couldn't save the project."));
    }
    void refreshSession({ silent: true });
    toast({ title: "Project updated", description: "Desktop agents pick up the new settings within five minutes." });
  }

  async function addMember(input: CreateProjectMemberDto) {
    if (!project.live) {
      setDemoMembers([...(demoMembers ?? mockProject?.memberIds ?? []), input.userId]);
      toast({ title: "Member added", description: "Demo workspace — this change lasts until you reload." });
      return;
    }
    await addProjectMember(input);
    await loadFromApi();
    void loadTotals();
    void refreshSession({ silent: true });
    toast({ title: "Member added", description: `${userById(input.userId)?.name ?? "They"} can track time on ${project.title} now.` });
  }

  async function removeMember(row: MemberRow) {
    if (!project.live) {
      setDemoMembers((demoMembers ?? mockProject?.memberIds ?? []).filter((m) => m !== row.userId));
      setRemoving(null);
      toast({ title: `${row.name} removed`, description: "Demo workspace — this change lasts until you reload." });
      return;
    }
    if (!row.membershipId) return;
    setBusy(true);
    try {
      await removeProjectMember(row.membershipId);
      setRemoving(null);
      await loadFromApi();
      void loadTotals();
      void refreshSession({ silent: true });
      toast({ title: `${row.name} removed from ${project.title}`, description: "Their time already tracked stays in reports." });
    } catch (err) {
      toast({ tone: "danger", title: "Couldn't remove them", description: membershipErrorText(err, "The change wasn't saved.") });
    } finally {
      setBusy(false);
    }
  }

  const people = users
    .filter((u) => u.role !== "client" && u.id !== user.id)
    .map((u) => ({ id: u.id, name: u.name, email: u.email }));

  return (
    <PageStack>
      <Link href="/projects" className="inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground">
        <ArrowLeft className="h-4 w-4" /> Projects
      </Link>

      <header className="flex flex-wrap items-start justify-between gap-4 border-b border-border/70 pb-5">
        <div className="flex min-w-0 items-center gap-4">
          <div aria-hidden className="flex h-14 w-14 shrink-0 items-center justify-center rounded-2xl text-white" style={{ background: project.color }}>
            <span className="font-display text-xl font-bold">{project.title[0]}</span>
          </div>
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h1 className="page-title">{project.title}</h1>
              {project.archived && (
                <Badge tone="muted">
                  {project.archivedAt ? `Archived ${formatDayKey(zonedDayKey(Date.parse(project.archivedAt), zone.timeZone), { year: true })}` : "Archived"}
                </Badge>
              )}
            </div>
            <p className="mt-1.5 text-sm leading-relaxed text-muted-foreground">{project.description || "No description yet."}</p>
          </div>
        </div>
        {canManage && (
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" onClick={() => setEditing(true)} disabled={busy}>
              <Pencil className="h-4 w-4" /> Edit project
            </Button>
            <Button
              variant="outline"
              onClick={() => (project.archived ? void toggleArchive() : setConfirmArchive(true))}
              disabled={busy}
            >
              {project.archived ? <ArchiveRestore className="h-4 w-4" /> : <Archive className="h-4 w-4" />}
              {project.archived ? "Unarchive" : "Archive"}
            </Button>
          </div>
        )}
      </header>

      {isLive && totals.failed && (
        <p role="alert" className="text-xs text-danger-on-tint">
          Tracked time couldn&apos;t be loaded.{" "}
          <button type="button" onClick={() => void loadTotals()} className="underline underline-offset-2">
            Retry
          </button>
        </p>
      )}

      {/* Stats */}
      <ul className="grid grid-cols-2 gap-3 sm:gap-4 lg:grid-cols-4" aria-label="Tracked time">
        {stats.map((s) => (
          <li key={s.label} className="flex min-w-0 items-center gap-3 rounded-2xl border border-border bg-card p-4 card-elev">
            <div aria-hidden className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary">
              <s.icon className="h-5 w-5" />
            </div>
            <div className="min-w-0">
              <p className="font-display text-lg font-bold tabular-nums text-ink">{s.value}</p>
              <p className="text-xs text-muted-foreground">{s.label}</p>
              {s.sub && <p className="text-[11px] text-muted-foreground">{s.sub}</p>}
            </div>
          </li>
        ))}
      </ul>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        {/* Members */}
        <Card className="lg:col-span-2">
          <CardHeader>
            <div>
              <CardTitle>Team members</CardTitle>
              <p className="mt-0.5 text-xs text-muted-foreground">
                {teamFigures ? `Time tracked ${memberPeriod} and activity %` : "The people working on this project"}
              </p>
            </div>
            {canManage && !project.archived && (
              <Button size="sm" variant="outline" onClick={() => setAdding(true)}>
                <UserPlus className="h-3.5 w-3.5" /> Add member
              </Button>
            )}
          </CardHeader>
          <CardContent className="space-y-1">
            {view.live && apiMembers === null && (
              <p className="py-6 text-center text-sm text-muted-foreground">The member list couldn&apos;t be loaded.</p>
            )}
            {memberRows.length === 0 && (view.live ? apiMembers !== null : true) && (
              <p className="py-6 text-center text-sm text-muted-foreground">No members yet.</p>
            )}
            <ul className="space-y-1">
              {memberRows.map((m) => (
                <li key={m.userId} className="flex items-center gap-3 rounded-lg p-2 hover:bg-muted/60">
                  <Avatar name={m.name} size="md" status={teamFigures ? m.status : undefined} />
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-sm font-medium">
                      {m.name}
                      {m.userId === user.id && <span className="font-normal text-muted-foreground"> (you)</span>}
                    </div>
                    <div className="truncate text-xs text-muted-foreground">
                      {m.role}
                      {teamFigures && m.minutes !== null && <span className="sm:hidden"> · {formatDuration(m.minutes)} {memberPeriod}</span>}
                    </div>
                  </div>
                  {teamFigures && (
                    <>
                      <div className="hidden text-right sm:block">
                        <div className="text-sm font-medium tabular-nums">{m.minutes === null ? "…" : formatDuration(m.minutes)}</div>
                        <div className="text-xs text-muted-foreground">{memberPeriod}</div>
                      </div>
                      <ActivityRing value={m.activity} size={40} stroke={4} />
                    </>
                  )}
                  {canManage && m.userId !== user.id && (m.membershipId || !project.live) && (
                    <Button
                      size="icon"
                      variant="ghost"
                      className="h-8 w-8 text-muted-foreground hover:text-danger-on-tint"
                      onClick={() => setRemoving(m)}
                      aria-label={`Remove ${m.name} from ${project.title}`}
                      title={`Remove ${m.name} from this project`}
                    >
                      <UserMinus className="h-4 w-4" />
                    </Button>
                  )}
                </li>
              ))}
            </ul>
          </CardContent>
        </Card>

        {/* Tracking settings */}
        <Card>
          <CardHeader>
            <CardTitle>Tracking settings</CardTitle>
            {canManage && (
              <Button size="sm" variant="ghost" onClick={() => setEditing(true)} aria-label="Edit tracking settings">
                <Pencil className="h-3.5 w-3.5" /> Edit
              </Button>
            )}
          </CardHeader>
          <CardContent className="space-y-3">
            <dl className="space-y-2">
              <div className="flex items-center justify-between gap-3 rounded-lg bg-muted/50 p-3">
                <dt className="text-sm text-muted-foreground">Block length</dt>
                <dd className="text-sm font-semibold">{project.draft.intervalMinutes} min</dd>
              </div>
              <div className="flex items-center justify-between gap-3 rounded-lg bg-muted/50 p-3">
                <dt className="flex items-center gap-2 text-sm text-muted-foreground">
                  <Hourglass aria-hidden className="h-4 w-4" /> Idle detection
                </dt>
                <dd className="text-right text-sm font-semibold">{idleLabel(project.draft.idleThresholdMinutes)}</dd>
              </div>
            </dl>
            <ul className="space-y-2.5" aria-label="What the agent captures">
              {permMeta.map(({ key, label, icon: Icon }) => (
                <li key={key} className="flex items-center justify-between gap-3">
                  <span className="flex items-center gap-2 text-sm">
                    <Icon aria-hidden className="h-4 w-4 text-muted-foreground" /> {label}
                  </span>
                  <Badge tone={project.permissions[key] ? "success" : "muted"}>{project.permissions[key] ? "On" : "Off"}</Badge>
                </li>
              ))}
              <li className="flex items-center justify-between gap-3">
                <span className="flex items-center gap-2 text-sm">
                  <EyeOff aria-hidden className="h-4 w-4 text-muted-foreground" /> Blur screenshots
                </span>
                <Badge tone={project.draft.blurScreenshots && project.permissions.screenshot ? "success" : "muted"}>
                  {project.draft.blurScreenshots ? (project.permissions.screenshot ? "On" : "On · no screenshots") : "Off"}
                </Badge>
              </li>
            </ul>
          </CardContent>
        </Card>
      </div>

      {/* Project tasks */}
      <ProjectTasksSection
        key={project.id}
        projectId={project.id}
        workspaceId={isLive ? workspace?.id ?? currentWorkspaceId() : null}
        projectArchived={project.archived}
        isLive={project.live}
        canMutate={canMutateTasks}
      />

      {/* "Project pulse": an AI summary of the project's last 7 days, for whoever sees the team's figures */}
      {canManage && zone.ready && (
        <AiPeriodSummaryCard
          scope="team"
          projectId={project.id}
          from={pulsePeriod.from}
          to={pulsePeriod.to}
          timeZone={zone.timeZone}
        />
      )}

      {/* Recent activity */}
      <Card>
        <CardHeader>
          <CardTitle>Recent activity</CardTitle>
          <Link href="/activities" className="text-xs text-primary-strong hover:underline">
            View all
          </Link>
        </CardHeader>
        <CardContent>
          {activityRows.length === 0 ? (
            <p className="py-6 text-center text-sm text-muted-foreground">No activity recorded yet.</p>
          ) : (
            <ul className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4">
              {activityRows.map((a) => (
                <li key={a.id} className="space-y-2">
                  {view.live ? (
                    <AppGlyph app={a.screen.app} color={a.screen.accent} className="aspect-video w-full" />
                  ) : (
                    <ScreenMockView screen={a.screen} title={a.windowTitle} className="aspect-video w-full" />
                  )}
                  <div className="flex items-center gap-2">
                    <Avatar name={a.userName} size="sm" />
                    <div className="min-w-0">
                      <div className="truncate text-xs font-medium">{a.userName}</div>
                      <div className="truncate text-[11px] text-muted-foreground">{agoLabel(a.endedAt, now)}</div>
                    </div>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      {editing && <EditProjectModal initial={project.draft} onClose={() => setEditing(false)} onSave={saveEdit} note={project.live ? undefined : "Demo workspace — edits last until you reload."} />}

      {adding && (
        <AddMemberDialog
          people={people}
          projects={[{ id: project.id, title: project.title }]}
          fixedProjectId={project.id}
          memberIdsOf={(pid) => (pid === project.id ? memberIds : null)}
          rulesFor={(personId) => editRulesFor(user, personId)}
          onClose={() => setAdding(false)}
          onSubmit={addMember}
        />
      )}

      <ConfirmDialog
        open={confirmArchive}
        onCancel={() => setConfirmArchive(false)}
        onConfirm={() => void toggleArchive()}
        title={`Archive ${project.title}?`}
        description="Members can no longer track time on it — their desktop agents stop offering it within five minutes. Time already tracked stays in reports, and blocks recorded before now still upload. You can unarchive it any time."
        confirmLabel="Archive project"
        busy={busy}
      />

      <ConfirmDialog
        open={!!removing}
        onCancel={() => setRemoving(null)}
        onConfirm={() => removing && void removeMember(removing)}
        title={removing ? `Remove ${removing.name} from ${project.title}?` : "Remove member?"}
        description="They can no longer track time on this project, and their desktop agent stops offering it within five minutes. Time they already tracked stays in reports. If this was their only project, their seat is freed from the next invoice."
        confirmLabel="Remove from project"
        busy={busy}
      />
    </PageStack>
  );
}

function fmt(minutes: number | null): string {
  return minutes === null ? "…" : formatDuration(minutes);
}
