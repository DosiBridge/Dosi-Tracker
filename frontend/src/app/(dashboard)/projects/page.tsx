"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import Link from "next/link";
import {
  Plus,
  Clock,
  Camera,
  Video,
  Keyboard,
  MousePointerClick,
  AppWindow,
  Archive,
} from "lucide-react";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Progress } from "@/components/ui/progress";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { SearchField, SegmentedControl, Toolbar } from "@/components/ui/toolbar";
import { CreateProjectModal } from "@/components/projects/create-project-modal";
import { EmptyState } from "@/components/ui/empty-state";
import { useSession } from "@/components/session-provider";
import { createTenantProject, projectById, userById } from "@/lib/tenant-data";
import { scopeProjects } from "@/lib/scope";
import { useApi, getApi, postApi } from "@/hooks/useApi";
import type { Project } from "@/lib/types";
import { cn, formatDuration } from "@/lib/utils";

const permIcons = [
  { key: "screenshot", icon: Camera },
  { key: "webcam", icon: Video },
  { key: "keyboard", icon: Keyboard },
  { key: "mouse", icon: MousePointerClick },
  { key: "activeWindow", icon: AppWindow },
] as const;

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

/** Shape returned by GET /api/app/team/project-members/{projectId}. */
interface ApiProjectMember {
  id: string;
  projectId: string;
  userId: string;
  role?: string;
}

/** Card view-model: a demo Project plus a flag telling us it came from the real API. */
type ProjectView = Project & { live: boolean };

/**
 * Map a real backend project onto the card view-model the markup expects.
 * Membership and tracked totals come from the session's live dataset (built
 * from the real roster and rows) when it knows the project.
 */
function toProjectView(p: ApiProject): ProjectView {
  const known = projectById(p.id);
  return {
    id: p.id,
    title: p.title,
    description: p.description ?? "",
    color: p.color ?? "#0d9488",
    archived: !!p.isArchived,
    intervalMinutes: p.intervalMinutes ?? 10,
    permissions: {
      screenshot: !!p.allowScreenshot,
      webcam: !!p.allowWebcam,
      keyboard: !!p.allowKeyboard,
      mouse: !!p.allowMouse,
      activeWindow: !!p.allowActiveWindow,
      runningPrograms: !!p.allowRunningPrograms,
    },
    // Real members are also fetched lazily per project (see membersByProject);
    // the roster-derived list is used for scoping and is always an array.
    memberIds: [...(known?.memberIds ?? [])],
    createdAt: (p.creationTime ?? "").slice(0, 10),
    loggedThisWeek: known?.loggedThisWeek ?? 0,
    loggedThisMonth: known?.loggedThisMonth ?? 0,
    loggedTotal: known?.loggedTotal ?? 0,
    live: true,
  };
}

/**
 * Avatar name for a real member: their roster name, else a neutral label.
 * Workers only receive their own roster row, so teammates are unknown here —
 * initials derived from a GUID ("3A") read as a broken name.
 */
function memberAvatarName(userId: string): string {
  return userById(userId)?.name ?? "Team member";
}

export default function ProjectsPage() {
  const { user, workspace, isLive, refreshSession } = useSession();
  const canManage = user.role === "owner" || user.role === "admin";
  // Demo mode never touches the network.
  const { data: apiProjects, error, isLoading, refetch } = useApi<ApiProject[]>('/api/app/project?MaxResultCount=1000', { enabled: isLive });
  const [query, setQuery] = useState("");
  const [tab, setTab] = useState<"active" | "archived">("active");
  const [open, setOpen] = useState(false);
  const [isCreating, setIsCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  // Demo-mode projects created locally through the modal (mock path only).
  const [created, setCreated] = useState<Project[]>([]);
  // projectId -> members, lazily filled for visible REAL projects. undefined = not loaded yet.
  const [membersByProject, setMembersByProject] = useState<Record<string, ApiProjectMember[]>>({});
  const requestedMembersRef = useRef<Set<string>>(new Set());

  // Live projects when the API call succeeded. A member (worker/client) only
  // sees the projects they belong to, exactly as in demo mode.
  const liveProjects = useMemo<ProjectView[] | null>(() => {
    if (!isLive || !Array.isArray(apiProjects)) return null;
    const views = apiProjects.filter((p) => p && typeof p.id === "string").map(toProjectView);
    return canManage ? views : views.filter((p) => p.memberIds.includes(user.id));
  }, [isLive, apiProjects, canManage, user.id]);

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

  const filtered = useMemo(
    () =>
      list.filter(
        (p) =>
          p.archived === (tab === "archived") &&
          p.title.toLowerCase().includes(query.toLowerCase())
      ),
    [list, query, tab]
  );

  // Lazily fetch members for the REAL projects currently on screen (cached per projectId).
  useEffect(() => {
    if (!isLive) return;
    for (const p of filtered) {
      if (!p.live || requestedMembersRef.current.has(p.id)) continue;
      requestedMembersRef.current.add(p.id);
      getApi(`/api/app/team/project-members/${encodeURIComponent(p.id)}`)
        .then((rows) => {
          setMembersByProject((prev) => ({
            ...prev,
            [p.id]: Array.isArray(rows) ? (rows as ApiProjectMember[]) : [],
          }));
        })
        .catch(() => {
          setMembersByProject((prev) => ({ ...prev, [p.id]: [] }));
        });
    }
  }, [isLive, filtered]);

  const maxTotal = Math.max(...list.map((p) => p.loggedTotal), 1);
  const activeCount = list.filter((p) => !p.archived).length;
  const archivedCount = list.filter((p) => p.archived).length;

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
          options={[
            { value: "active", label: "Active" },
            { value: "archived", label: "Archived" },
          ]}
        />
      </Toolbar>

      {createError && (
        <div className="flex items-start justify-between gap-3 rounded-lg border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger">
          <span>{createError}</span>
          <button
            type="button"
            onClick={() => setCreateError(null)}
            className="shrink-0 text-xs underline underline-offset-2"
          >
            Dismiss
          </button>
        </div>
      )}

      {liveError && (
        <p className="text-xs text-danger">
          Couldn’t load your projects from the server.{" "}
          <button
            type="button"
            onClick={() => void refetch()}
            className="underline underline-offset-2 hover:text-foreground"
          >
            Retry
          </button>
        </p>
      )}

      {isCreating && <p className="text-xs text-muted-foreground">Creating project…</p>}

      {liveLoading ? (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
          {[0, 1, 2].map((i) => (
            <Card key={i} className="animate-pulse p-5" aria-hidden>
              <div className="flex items-center gap-3">
                <div className="h-11 w-11 rounded-xl bg-muted" />
                <div className="space-y-2">
                  <div className="h-4 w-32 rounded bg-muted" />
                  <div className="h-3 w-20 rounded bg-muted" />
                </div>
              </div>
              <div className="mt-4 h-3 w-full rounded bg-muted" />
              <div className="mt-2 h-3 w-2/3 rounded bg-muted" />
              <div className="mt-6 h-2 w-full rounded bg-muted" />
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
            return (
              <Link key={p.id} href={`/projects/${p.id}`}>
                <Card variant="interactive" className="group h-full p-5">
                  <div className="flex items-start justify-between">
                    <div className="flex items-center gap-3">
                      <div
                        className="flex h-11 w-11 items-center justify-center rounded-xl text-white"
                        style={{ background: p.color }}
                      >
                        <span className="font-display text-base font-bold">{p.title[0]}</span>
                      </div>
                      <div>
                        <h2 className="font-semibold leading-tight group-hover:text-primary">{p.title}</h2>
                        <p className="text-xs text-muted-foreground">Interval · {p.intervalMinutes}m</p>
                      </div>
                    </div>
                    {p.archived && <Badge tone="muted">Archived</Badge>}
                  </div>

                  <p className="mt-3 line-clamp-2 text-sm text-muted-foreground">{p.description}</p>

                  <div className="mt-4 flex flex-wrap gap-1.5">
                    {permIcons.map(({ key, icon: Icon }) => {
                      const on = p.permissions[key];
                      return (
                        <span
                          key={key}
                          className={cn(
                            "flex h-7 w-7 items-center justify-center rounded-lg",
                            on ? "bg-accent text-accent-foreground" : "bg-muted text-muted-foreground/40"
                          )}
                          title={key}
                        >
                          <Icon className="h-3.5 w-3.5" />
                        </span>
                      );
                    })}
                  </div>

                  <div className="mt-4 space-y-1.5">
                    <div className="flex items-center justify-between text-xs">
                      <span className="flex items-center gap-1 text-muted-foreground">
                        <Clock className="h-3 w-3" /> {formatDuration(p.loggedThisWeek)} this week
                      </span>
                      <span className="font-medium tabular-nums">{formatDuration(p.loggedTotal)}</span>
                    </div>
                    <Progress value={(p.loggedTotal / maxTotal) * 100} color={p.color} />
                  </div>

                  <div className="mt-4 flex items-center justify-between">
                    {p.live ? (
                      liveMembers === undefined ? (
                        <span className="text-xs text-muted-foreground">Loading members…</span>
                      ) : (
                        <>
                          <div className="flex -space-x-2">
                            {liveMembers.slice(0, 4).map((m) => (
                              <Avatar key={m.id} name={memberAvatarName(m.userId)} size="sm" />
                            ))}
                            {liveMembers.length > 4 && (
                              <span className="flex h-8 w-8 items-center justify-center rounded-full bg-muted text-xs font-medium ring-2 ring-card">
                                +{liveMembers.length - 4}
                              </span>
                            )}
                          </div>
                          <span className="text-xs text-muted-foreground">{liveMembers.length} members</span>
                        </>
                      )
                    ) : (
                      <>
                        <div className="flex -space-x-2">
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
                        <span className="text-xs text-muted-foreground">{(p.memberIds ?? []).length} members</span>
                      </>
                    )}
                  </div>
                </Card>
              </Link>
            );
          })}
        </div>
      )}

      <CreateProjectModal
        open={open}
        onClose={() => setOpen(false)}
        onCreate={async (p) => {
          setCreateError(null);
          if (!isLive) {
            // Demo mode: persist through the tenant-data layer (localStorage-
            // backed) so the project survives a reload; the local state only
            // triggers the re-render.
            createTenantProject(workspace.id, p);
            setCreated((prev) => [p, ...prev]);
            return;
          }
          setIsCreating(true);
          try {
            const createdProject = (await postApi('/api/app/project', {
              title: p.title,
              description: p.description,
              color: p.color,
              intervalMinutes: p.intervalMinutes,
              allowScreenshot: p.permissions.screenshot,
              allowWebcam: p.permissions.webcam,
              allowKeyboard: p.permissions.keyboard,
              allowMouse: p.permissions.mouse,
              allowActiveWindow: p.permissions.activeWindow,
              allowRunningPrograms: p.permissions.runningPrograms,
            })) as { id?: string } | null;
            // The backend adds the creator; add everyone else picked in the
            // wizard (real roster members) so the selection isn't silently lost.
            const others = (p.memberIds ?? []).filter((mid) => mid !== user.id);
            if (createdProject?.id && others.length > 0) {
              const results = await Promise.allSettled(
                others.map((userId) =>
                  postApi('/api/app/team/member', { projectId: createdProject.id, userId, role: 'Worker', hourlyRate: 0 }),
                ),
              );
              const failed = results.filter((r) => r.status === 'rejected').length;
              if (failed > 0) {
                setCreateError(`The project was created, but ${failed} member${failed === 1 ? '' : 's'} couldn’t be added (check your plan’s seat limit).`);
              }
            }
            await refetch();
            // The creator is now a member: refresh the session's live dataset
            // so pickers, the checklist and scoping see the new project.
            void refreshSession({ silent: true });
          } catch (e) {
            setCreateError(`Couldn’t create the project — ${e instanceof Error ? e.message : "Something went wrong. Please try again."}`);
          } finally {
            setIsCreating(false);
          }
        }}
      />
    </PageStack>
  );
}
