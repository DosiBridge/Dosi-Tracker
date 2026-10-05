"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import Link from "next/link";
import {
  Plus,
  Camera,
  Video,
  Keyboard,
  MousePointerClick,
  AppWindow,
  ListTree,
  Archive,
  CalendarDays,
  CalendarRange,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { SearchField, SegmentedControl, Toolbar } from "@/components/ui/toolbar";
import { CreateProjectModal, type ProjectFormResult } from "@/components/projects/create-project-modal";
import { demoProjectFrom, normalizeNewProjectDefaults } from "@/components/projects/project-form";
import { memberNameOf, membershipErrorText } from "@/components/projects/membership";
import { requestErrorText } from "@/components/layout/request-error";
import { EmptyState } from "@/components/ui/empty-state";
import { useSession } from "@/components/session-provider";
import { createTenantProject, projectById, userById } from "@/lib/tenant-data";
import { scopeProjects } from "@/lib/scope";
import { reportingRoute, type NewProjectDefaultsDto, type ProjectMemberDto, type ReportSummaryDto } from "@/lib/api-contract";
import { zonedPeriod } from "@/lib/time-zone";
import { useReportingZone } from "@/hooks/use-reporting-zone";
import { useApi, getApi, getProjectMembers, getWorkspaceSettings, postApi, addProjectMember } from "@/hooks/useApi";
import type { Project, TrackingPermissions } from "@/lib/types";
import { cn, formatDuration } from "@/lib/utils";

const permIcons: { key: keyof TrackingPermissions; label: string; icon: LucideIcon }[] = [
  { key: "screenshot", label: "Screenshots", icon: Camera },
  { key: "webcam", label: "Webcam", icon: Video },
  { key: "keyboard", label: "Keyboard activity", icon: Keyboard },
  { key: "mouse", label: "Mouse activity", icon: MousePointerClick },
  { key: "activeWindow", label: "Active window", icon: AppWindow },
  { key: "runningPrograms", label: "Running programs", icon: ListTree },
];

/** Shape returned by GET /api/app/project (ABP ProjectDto, camelCased). */
interface ApiProject {
  id: string;
  title: string;
  description?: string | null;
  color?: string | null;
  intervalMinutes?: number;
  isArchived?: boolean;
  allowScreenshot?: boolean;
  allowWebcam?: boolean;
  allowKeyboard?: boolean;
  allowMouse?: boolean;
  allowActiveWindow?: boolean;
  allowRunningPrograms?: boolean;
  creationTime?: string;
}

/** Card view-model: a demo Project plus a flag telling us it came from the real API. */
type ProjectView = Project & { live: boolean };

/** Tracked minutes per project for this week and the last 30 days (live: the reporting API). */
interface ProjectTotals {
  week: Map<string, number>;
  month: Map<string, number>;
}

/**
 * Map a real backend project onto the card view-model. Who is on it comes
 * from the project-members call per card; tracked time from the reporting
 * summary (never from a capped page of rows).
 */
function toProjectView(p: ApiProject): ProjectView {
  return {
    id: p.id,
    title: p.title,
    description: p.description ?? "",
    color: p.color ?? "#0d9488",
    archived: !!p.isArchived,
    intervalMinutes: p.intervalMinutes ?? 10,
    permissions: {
      screenshot: p.allowScreenshot !== false,
      webcam: !!p.allowWebcam,
      keyboard: p.allowKeyboard !== false,
      mouse: p.allowMouse !== false,
      activeWindow: p.allowActiveWindow !== false,
      runningPrograms: p.allowRunningPrograms !== false,
    },
    memberIds: [...(projectById(p.id)?.memberIds ?? [])],
    createdAt: (p.creationTime ?? "").slice(0, 10),
    loggedThisWeek: 0,
    loggedThisMonth: 0,
    loggedTotal: 0,
    live: true,
  };
}

function minutesByProject(summary: unknown): Map<string, number> {
  const rows = (summary as Partial<ReportSummaryDto> | null)?.perProject;
  const map = new Map<string, number>();
  for (const r of Array.isArray(rows) ? rows : []) {
    if (r && typeof r.projectId === "string") map.set(r.projectId, Math.round(Number(r.trackedMinutes) || 0));
  }
  return map;
}

/** "Screenshot every 10 min", or the block length when screenshots are off. */
function cadenceLabel(p: Project): string {
  return p.permissions.screenshot ? `Screenshot every ${p.intervalMinutes} min` : `${p.intervalMinutes}-min blocks · no screenshots`;
}

export default function ProjectsPage() {
  const { user, workspace, isLive, refreshSession } = useSession();
  const canManage = user.role === "owner" || user.role === "admin";
  // Demo mode never touches the network.
  const { data: apiProjects, error, isLoading, refetch } = useApi<ApiProject[]>("/api/app/project?MaxResultCount=1000", { enabled: isLive });
  const zone = useReportingZone(isLive);
  const [query, setQuery] = useState("");
  const [tab, setTab] = useState<"active" | "archived">("active");
  const [open, setOpen] = useState(false);
  const [notice, setNotice] = useState<{ tone: "danger" | "warning"; text: string } | null>(null);
  // Demo-mode projects created locally through the modal (mock path only).
  const [created, setCreated] = useState<Project[]>([]);
  // projectId -> members, lazily filled for visible REAL projects. undefined = not loaded yet.
  const [membersByProject, setMembersByProject] = useState<Record<string, ProjectMemberDto[]>>({});
  const requestedMembersRef = useRef<Set<string>>(new Set());
  const [totals, setTotals] = useState<{ data: ProjectTotals | null; failed: boolean }>({ data: null, failed: false });
  const [defaults, setDefaults] = useState<NewProjectDefaultsDto | null>(null);

  // Live: the server already scopes the list to what this person may see —
  // trust it (a worker added after sign-in must still see the project).
  const liveProjects = useMemo<ProjectView[] | null>(() => {
    if (!isLive || !Array.isArray(apiProjects)) return null;
    return apiProjects.filter((p) => p && typeof p.id === "string").map(toProjectView);
  }, [isLive, apiProjects]);

  const mockProjects = useMemo<ProjectView[]>(() => {
    let scoped: Project[] = [];
    try {
      scoped = scopeProjects(user);
    } catch {
      scoped = []; // never let the demo fallback crash the page
    }
    // Demo-created projects also live in the tenant dataset (persisted there
    // via createTenantProject), so drop them from the scoped list to avoid
    // rendering duplicates.
    const createdIds = new Set(created.map((p) => p.id));
    return [...created, ...scoped.filter((p) => !createdIds.has(p.id))].map((p) => ({ ...p, live: false }));
  }, [created, user]);

  // Live: real projects only — a failed load shows an error, never the demo list.
  const list = useMemo(() => (isLive ? (liveProjects ?? []) : mockProjects), [isLive, liveProjects, mockProjects]);
  const liveLoading = isLive && isLoading && !liveProjects;
  const liveError = isLive && !!error && !liveProjects;

  // Live tracked time per project: this week and the last 30 days, as calendar
  // days of the reporting zone, from the reporting summary.
  const loadTotals = useCallback(async () => {
    const now = new Date();
    const week = zonedPeriod("week", now, zone.timeZone);
    const month = zonedPeriod("30d", now, zone.timeZone);
    try {
      const [weekSummary, monthSummary] = await Promise.all([
        getApi(reportingRoute("summary", { from: week.from, to: week.to, timeZone: zone.timeZone })),
        getApi(reportingRoute("summary", { from: month.from, to: month.to, timeZone: zone.timeZone })),
      ]);
      setTotals({ data: { week: minutesByProject(weekSummary), month: minutesByProject(monthSummary) }, failed: false });
    } catch {
      setTotals({ data: null, failed: true });
    }
  }, [zone.timeZone]);

  useEffect(() => {
    if (!isLive || !zone.ready) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void loadTotals();
  }, [isLive, zone.ready, loadTotals]);

  // The workspace's new-project defaults (owners may read them; anyone else
  // — or an older server — falls back to the built-in defaults).
  useEffect(() => {
    if (!isLive || !canManage) return;
    let cancelled = false;
    getWorkspaceSettings()
      .then((settings) => {
        if (!cancelled) setDefaults(settings?.newProjectDefaults ? normalizeNewProjectDefaults(settings.newProjectDefaults) : null);
      })
      .catch(() => {
        if (!cancelled) setDefaults(null);
      });
    return () => {
      cancelled = true;
    };
  }, [isLive, canManage]);

  const filtered = useMemo(
    () => list.filter((p) => p.archived === (tab === "archived") && p.title.toLowerCase().includes(query.toLowerCase())),
    [list, query, tab],
  );

  // Lazily fetch members for the REAL projects currently on screen (cached per projectId).
  useEffect(() => {
    if (!isLive) return;
    for (const p of filtered) {
      if (!p.live || requestedMembersRef.current.has(p.id)) continue;
      requestedMembersRef.current.add(p.id);
      getProjectMembers(p.id)
        .then((rows) => setMembersByProject((prev) => ({ ...prev, [p.id]: rows })))
        .catch(() => setMembersByProject((prev) => ({ ...prev, [p.id]: [] })));
    }
  }, [isLive, filtered]);

  const activeCount = list.filter((p) => !p.archived).length;
  const archivedCount = list.filter((p) => p.archived).length;

  function weekMinutes(p: ProjectView): number | null {
    if (!p.live) return p.loggedThisWeek;
    return totals.data ? (totals.data.week.get(p.id) ?? 0) : null;
  }
  function monthMinutes(p: ProjectView): number | null {
    if (!p.live) return p.loggedThisMonth;
    return totals.data ? (totals.data.month.get(p.id) ?? 0) : null;
  }

  async function onCreate({ dto, memberIds }: ProjectFormResult): Promise<void> {
    setNotice(null);
    if (!isLive) {
      // Demo mode: persist through the tenant-data layer (localStorage-
      // backed) so the project survives a reload; the local state only
      // triggers the re-render.
      const project = demoProjectFrom(dto, { id: `p${Date.now()}`, memberIds, createdAt: new Date().toISOString().slice(0, 10) });
      createTenantProject(workspace.id, project);
      setCreated((prev) => [project, ...prev]);
      return;
    }
    let createdProject: { id?: string } | null;
    try {
      createdProject = (await postApi("/api/app/project", dto)) as { id?: string } | null;
    } catch (err) {
      // The wizard stays open with this sentence, so nothing typed is lost.
      throw new Error(requestErrorText(err, "Couldn't create the project."));
    }
    // The backend adds the creator; add everyone else picked in the wizard.
    const others = memberIds.filter((mid) => mid !== user.id);
    if (createdProject?.id && others.length > 0) {
      const projectId = createdProject.id;
      const results = await Promise.allSettled(
        others.map((userId) => addProjectMember({ projectId, userId, role: "Worker", hourlyRate: 0 })),
      );
      const failures = results.filter((r): r is PromiseRejectedResult => r.status === "rejected");
      if (failures.length > 0) {
        setNotice({
          tone: "warning",
          text: `${dto.title} was created, but ${failures.length} of ${others.length} people couldn't be added: ${membershipErrorText(failures[0].reason, "the server refused it.")}`,
        });
      }
    }
    await refetch();
    void loadTotals();
    // The creator is now a member: refresh the session's live dataset so
    // pickers, the checklist and scoping see the new project.
    void refreshSession({ silent: true });
  }

  const defaultsNote = !isLive
    ? "These are the workspace's standard tracking defaults."
    : defaults
      ? "Starts from your workspace defaults (Settings → Workspace). Change anything for this project."
      : "Starts from the standard tracking defaults. Owners can set workspace defaults in Settings → Workspace.";

  return (
    <PageStack>
      <PageHeader
        eyebrow="Workspace"
        title="Projects"
        description={`${activeCount} active · ${archivedCount} archived`}
        actions={
          canManage ? (
            <Button onClick={() => setOpen(true)}>
              <Plus className="h-4 w-4" /> New project
            </Button>
          ) : undefined
        }
      />

      <Toolbar>
        <SearchField value={query} onChange={setQuery} placeholder="Search projects…" />
        <SegmentedControl
          value={tab}
          onChange={setTab}
          className="flex-nowrap overflow-x-auto"
          options={[
            { value: "active", label: "Active" },
            { value: "archived", label: "Archived" },
          ]}
        />
      </Toolbar>

      {notice && (
        <div
          role="alert"
          className={cn(
            "flex items-start justify-between gap-3 rounded-lg border px-3 py-2 text-sm",
            notice.tone === "danger" ? "border-danger/40 bg-danger/10 text-danger-on-tint" : "border-warning/40 bg-warning/10 text-warning-on-tint",
          )}
        >
          <span>{notice.text}</span>
          <button type="button" onClick={() => setNotice(null)} className="shrink-0 text-xs underline underline-offset-2">
            Dismiss
          </button>
        </div>
      )}

      {liveError && (
        <p role="alert" className="text-xs text-danger-on-tint">
          Couldn’t load your projects from the server.{" "}
          <button type="button" onClick={() => void refetch()} className="underline underline-offset-2 hover:text-foreground">
            Retry
          </button>
        </p>
      )}
      {isLive && totals.failed && !liveError && (
        <p className="text-xs text-muted-foreground">
          Tracked time couldn&apos;t be loaded.{" "}
          <button type="button" onClick={() => void loadTotals()} className="text-primary-strong underline underline-offset-2">
            Retry
          </button>
        </p>
      )}

      {liveLoading ? (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3" role="status" aria-label="Loading projects">
          {[0, 1, 2].map((i) => (
            <Card key={i} className="animate-pulse p-5 motion-reduce:animate-none" aria-hidden>
              <div className="flex items-center gap-3">
                <div className="h-11 w-11 rounded-xl bg-muted" />
                <div className="space-y-2">
                  <div className="h-4 w-32 rounded bg-muted" />
                  <div className="h-3 w-20 rounded bg-muted" />
                </div>
              </div>
              <div className="mt-4 h-3 w-full rounded bg-muted" />
              <div className="mt-2 h-3 w-2/3 rounded bg-muted" />
              <div className="mt-6 h-8 w-24 rounded-full bg-muted" />
            </Card>
          ))}
        </div>
      ) : liveError ? null : filtered.length === 0 ? (
        <EmptyState
          icon={Archive}
          title={query ? "No matching projects" : `No ${tab} projects`}
          description={
            query
              ? "Try a different search, or clear the filter."
              : tab === "archived"
                ? "Archived projects will show up here."
                : canManage
                  ? "Create a project to start attributing time and captures."
                  : "You haven’t been added to a project yet."
          }
          action={
            query
              ? { label: "Clear search", onClick: () => setQuery("") }
              : canManage && tab === "active"
                ? { label: "New project", onClick: () => setOpen(true) }
                : undefined
          }
        />
      ) : (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
          {filtered.map((p) => {
            const liveMembers = p.live ? membersByProject[p.id] : undefined;
            const week = weekMinutes(p);
            const month = monthMinutes(p);
            return (
              <Link
                key={p.id}
                href={`/projects/${p.id}`}
                className="group block rounded-2xl focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
              >
                <Card variant="interactive" className="h-full p-5">
                  <div className="flex items-start justify-between gap-3">
                    <div className="flex min-w-0 items-center gap-3">
                      <div
                        aria-hidden
                        className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl text-white"
                        style={{ background: p.color }}
                      >
                        <span className="font-display text-base font-bold">{p.title[0]}</span>
                      </div>
                      <div className="min-w-0">
                        <h2 className="truncate font-semibold leading-tight group-hover:text-primary-strong">{p.title}</h2>
                        <p className="text-xs text-muted-foreground">{cadenceLabel(p)}</p>
                      </div>
                    </div>
                    {p.archived && <Badge tone="muted">Archived</Badge>}
                  </div>

                  <p className={cn("mt-3 line-clamp-2 text-sm", p.description ? "text-muted-foreground" : "italic text-muted-foreground/80")}>
                    {p.description || "No description"}
                  </p>

                  <ul aria-label="What the agent captures" className="mt-4 flex flex-wrap gap-1.5">
                    {permIcons.map(({ key, label, icon: Icon }) => {
                      const on = p.permissions[key];
                      const text = `${label}: ${on ? "on" : "off"}`;
                      return (
                        <li
                          key={key}
                          title={text}
                          className={cn(
                            "flex h-7 w-7 items-center justify-center rounded-lg",
                            on ? "bg-accent text-accent-foreground" : "bg-muted text-muted-foreground/60",
                          )}
                        >
                          <Icon aria-hidden className={cn("h-3.5 w-3.5", !on && "opacity-60")} />
                          <span className="sr-only">{text}</span>
                        </li>
                      );
                    })}
                  </ul>

                  <dl className="mt-4 grid grid-cols-2 gap-2 rounded-xl border border-border/60 bg-muted/40 p-3 text-xs">
                    <div className="min-w-0">
                      <dt className="flex items-center gap-1 text-muted-foreground">
                        <CalendarDays aria-hidden className="h-3 w-3" /> This week
                      </dt>
                      <dd className="mt-0.5 text-sm font-semibold tabular-nums">{week === null ? "…" : formatDuration(week)}</dd>
                    </div>
                    <div className="min-w-0">
                      <dt className="flex items-center gap-1 text-muted-foreground">
                        <CalendarRange aria-hidden className="h-3 w-3" /> Last 30 days
                      </dt>
                      <dd className="mt-0.5 text-sm font-semibold tabular-nums">{month === null ? "…" : formatDuration(month)}</dd>
                    </div>
                  </dl>

                  <div className="mt-4 flex items-center justify-between">
                    {p.live ? (
                      liveMembers === undefined ? (
                        <span className="text-xs text-muted-foreground">Loading members…</span>
                      ) : (
                        <>
                          <div className="flex -space-x-2" aria-hidden>
                            {liveMembers.slice(0, 4).map((m) => (
                              <Avatar key={m.id} name={memberNameOf(m, userById(m.userId)?.name)} size="sm" />
                            ))}
                            {liveMembers.length > 4 && (
                              <span className="flex h-8 w-8 items-center justify-center rounded-full bg-muted text-xs font-medium ring-2 ring-card">
                                +{liveMembers.length - 4}
                              </span>
                            )}
                          </div>
                          <span className="text-xs text-muted-foreground">
                            {liveMembers.length} {liveMembers.length === 1 ? "member" : "members"}
                          </span>
                        </>
                      )
                    ) : (
                      <>
                        <div className="flex -space-x-2" aria-hidden>
                          {(p.memberIds ?? []).slice(0, 4).map((id) => {
                            const u = userById(id);
                            return u ? <Avatar key={id} name={u.name} size="sm" /> : null;
                          })}
                          {(p.memberIds ?? []).length > 4 && (
                            <span className="flex h-8 w-8 items-center justify-center rounded-full bg-muted text-xs font-medium ring-2 ring-card">
                              +{(p.memberIds ?? []).length - 4}
                            </span>
                          )}
                        </div>
                        <span className="text-xs text-muted-foreground">
                          {(p.memberIds ?? []).length} {(p.memberIds ?? []).length === 1 ? "member" : "members"}
                        </span>
                      </>
                    )}
                  </div>
                </Card>
              </Link>
            );
          })}
        </div>
      )}

      {open && (
        <CreateProjectModal
          onClose={() => setOpen(false)}
          onCreate={onCreate}
          defaults={defaults}
          defaultsNote={defaultsNote}
          currentUserId={isLive ? user.id : undefined}
        />
      )}
    </PageStack>
  );
}
