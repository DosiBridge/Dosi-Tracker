"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import Link from "next/link";
import {
  Plus,
  Mail,
  Clock,
  FolderKanban,
  Send,
  Users,
  Pencil,
  UserPlus,
  UserMinus,
  KeyRound,
  UserX,
  MonitorSmartphone,
  RotateCw,
} from "lucide-react";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Modal } from "@/components/ui/modal";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Input, Select } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { SearchField, SegmentedControl, Toolbar } from "@/components/ui/toolbar";
import { ActivityRing } from "@/components/dashboard/activity-ring";
import { DevicesPanel } from "@/components/dashboard/devices-panel";
import { memberPresence } from "@/components/dashboard/devices";
import { SignInDetails } from "@/components/dashboard/agent-setup";
import { linkButtonClass } from "@/components/dashboard/link-button";
import { AddMemberDialog } from "@/components/projects/project-members";
import {
  PROJECT_ROLE_LABELS,
  editRulesFor,
  membershipErrorText,
  parseHourlyRate,
  projectRoleOf,
  roleChoices,
} from "@/components/projects/membership";
import { requestErrorText } from "@/components/layout/request-error";
import { activities, projects, users } from "@/lib/tenant-data";
import { useSession } from "@/components/session-provider";
import { countSeats, inviteableRoles, roleLabels } from "@/lib/roles";
import { fmtLimit, planNameFor, seatLimitFor, usagePct } from "@/lib/saas-data";
import type { Role, UserStatus } from "@/lib/types";
import { cn, formatDuration } from "@/lib/utils";
import {
  addProjectMember,
  getApi,
  getProjectMembers,
  getTeamPresence,
  inviteMember,
  removeMemberFromWorkspace,
  removeProjectMember,
  resetMemberPassword,
  updateProjectMember,
} from "@/hooks/useApi";
import { useReportingZone } from "@/hooks/use-reporting-zone";
import { mapTeamMember, type ApiTeamMemberDto } from "@/lib/live-dataset";
import { referenceNow } from "@/lib/live-session";
import {
  TEAM_ROUTES,
  memberAccountActions,
  reportingRoute,
  type CreateProjectMemberDto,
  type ProjectMemberDto,
  type ProjectRole,
  type ReportSummaryDto,
} from "@/lib/api-contract";
import { zonedDayKey, zonedDayRange } from "@/lib/time-zone";
import { toast } from "@/components/toast";

const roleTone: Record<Role, "primary" | "info" | "warning" | "muted"> = {
  host: "primary",
  owner: "primary",
  admin: "warning",
  worker: "info",
  client: "muted",
};

/** Role filters, named as roleLabels names them ("Members", never "Workers"). */
const filters: { key: Role | "all"; label: string }[] = [
  { key: "all", label: "All" },
  { key: "owner", label: `${roleLabels.owner}s` },
  { key: "admin", label: `${roleLabels.admin}s` },
  { key: "worker", label: `${roleLabels.worker}s` },
  { key: "client", label: `${roleLabels.client}s` },
];

/** The subset of member fields the roster cards render — satisfied by both the
 * mock `User` records and live ABP identity users mapped below. */
interface MemberView {
  id: string;
  name: string;
  email: string;
  role: Role;
  status: UserStatus;
  /** Minutes tracked today (reporting zone, live). */
  trackedToday: number;
  /** Today's activity %; null when nothing was measured. */
  activity: number | null;
  /** The projects this member belongs to. */
  projectIds: string[];
}

interface ProjectOption {
  id: string;
  title: string;
  archived: boolean;
}

/** A project role as a select option value (lower case: "worker", "admin", "manager", "client"). */
const roleValue = (role: ProjectRole) => role.toLowerCase();
const roleFromValue = (value: string): ProjectRole => projectRoleOf(value);

/** The project role an app role invites as. */
const inviteRoleFor: Record<Role, ProjectRole> = { host: "Worker", owner: "Admin", admin: "Admin", worker: "Worker", client: "Client" };

/** Demo workspace changes made on this page (they last until the next reload). */
interface DemoChanges {
  added: Record<string, string[]>;
  removed: Record<string, string[]>;
  roles: Record<string, Role>;
  gone: string[];
}

const NO_DEMO_CHANGES: DemoChanges = { added: {}, removed: {}, roles: {}, gone: [] };
const DEMO_NOTE = "Demo workspace — this change lasts until you reload.";

function pluralMembers(n: number): string {
  return `${n} ${n === 1 ? "member" : "members"}`;
}

export default function TeamPage() {
  const { user, workspace, isLive, refreshSession } = useSession();
  const zone = useReportingZone(isLive);
  const [view, setView] = useState<"members" | "devices">("members");
  const [query, setQuery] = useState("");
  const [role, setRole] = useState<Role | "all">("all");
  const [invite, setInvite] = useState(false);
  const [adding, setAdding] = useState<{ personId: string | null } | null>(null);
  const [editing, setEditing] = useState<MemberView | null>(null);
  const [leaving, setLeaving] = useState<MemberView | null>(null);
  const [leavingBusy, setLeavingBusy] = useState(false);
  const [demo, setDemo] = useState<DemoChanges>(NO_DEMO_CHANGES);

  // LIVE MODE: the real roster from GET /api/app/team/members (roles from the
  // backend's owner/manager flags), refetched after every change. A failure
  // shows an error — never the demo roster.
  const [liveMembers, setLiveMembers] = useState<(MemberView & { isOwner: boolean })[] | null>(null);
  const [liveLoading, setLiveLoading] = useState(false);
  const [liveError, setLiveError] = useState<string | null>(null);
  const [liveProjects, setLiveProjects] = useState<ProjectOption[] | null>(null);
  const [presence, setPresence] = useState<Map<string, UserStatus> | null>(null);
  const [today, setToday] = useState<Map<string, { minutes: number; activity: number | null }> | null>(null);

  const loadRoster = useCallback(async () => {
    setLiveLoading(true);
    setLiveError(null);
    try {
      const items = await getApi(TEAM_ROUTES.members);
      const now = referenceNow();
      const list = (Array.isArray(items) ? items : []) as ApiTeamMemberDto[];
      setLiveMembers(
        list
          .filter((m) => m && typeof m.userId === "string")
          .map((m) => {
            const mapped = mapTeamMember(m, activities, now);
            return {
              id: mapped.id,
              name: mapped.name,
              email: mapped.email,
              role: mapped.role,
              status: mapped.status,
              trackedToday: mapped.trackedToday,
              activity: mapped.trackedToday > 0 ? mapped.productivity : null,
              projectIds: Array.isArray(m.projectIds) ? m.projectIds : [],
              isOwner: m.isOwner === true,
            };
          }),
      );
    } catch {
      setLiveMembers(null);
      setLiveError("Couldn't load your team from the server.");
    } finally {
      setLiveLoading(false);
    }
  }, []);

  // Every project, not ABP's default first page of 10 (archived ones are kept to name memberships).
  const loadProjects = useCallback(async () => {
    try {
      const items = await getApi("/api/app/project?MaxResultCount=1000");
      const list = (Array.isArray(items) ? items : []) as { id?: unknown; title?: unknown; isArchived?: unknown }[];
      setLiveProjects(
        list
          .filter((p) => typeof p?.id === "string")
          .map((p) => ({ id: p.id as string, title: typeof p.title === "string" && p.title ? p.title : "Untitled project", archived: p.isArchived === true }))
          .sort((a, b) => a.title.localeCompare(b.title)),
      );
    } catch {
      setLiveProjects(null); // the session's project list stands in
    }
  }, []);

  // Who is working right now, from the agents' heartbeats (older servers: the roster's last-block status stands).
  const loadPresence = useCallback(async () => {
    try {
      setPresence(memberPresence(await getTeamPresence(), new Date()));
    } catch {
      setPresence(null);
    }
  }, []);

  // Today's time and activity per member, as the reporting zone's calendar day.
  const loadToday = useCallback(async () => {
    const day = zonedDayKey(new Date(), zone.timeZone);
    const { from, to } = zonedDayRange(day, day, zone.timeZone);
    try {
      const summary = (await getApi(reportingRoute("summary", { from, to, timeZone: zone.timeZone }))) as Partial<ReportSummaryDto>;
      const map = new Map<string, { minutes: number; activity: number | null }>();
      for (const r of Array.isArray(summary?.perUser) ? summary.perUser : []) {
        if (r && typeof r.userId === "string") {
          map.set(r.userId, {
            minutes: Math.round(Number(r.trackedMinutes) || 0),
            activity: typeof r.averageProductivity === "number" ? Math.round(r.averageProductivity) : null,
          });
        }
      }
      setToday(map);
    } catch {
      setToday(null);
    }
  }, [zone.timeZone]);

  useEffect(() => {
    if (!isLive) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void loadRoster();
    void loadProjects();
    void loadPresence();
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") void loadPresence();
    }, 60_000);
    return () => clearInterval(timer);
  }, [isLive, loadRoster, loadProjects, loadPresence]);

  useEffect(() => {
    if (!isLive || !zone.ready) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void loadToday();
  }, [isLive, zone.ready, loadToday]);

  /** After any change: refresh this roster, its figures and the session's copy of it. */
  const onTeamChanged = useCallback(() => {
    void loadRoster();
    void loadToday();
    void refreshSession({ silent: true });
  }, [loadRoster, loadToday, refreshSession]);

  // Demo: the seeded roster, with this page's demo changes applied.
  const demoMembers = useMemo<(MemberView & { isOwner: boolean })[]>(
    () =>
      users
        .filter((u) => !demo.gone.includes(u.id))
        .map((u) => {
          const base = projects.filter((p) => (p.memberIds ?? []).includes(u.id)).map((p) => p.id);
          const projectIds = [...base.filter((pid) => !(demo.removed[u.id] ?? []).includes(pid)), ...(demo.added[u.id] ?? [])];
          return {
            id: u.id,
            name: u.name,
            email: u.email,
            role: demo.roles[u.id] ?? u.role,
            status: u.status,
            trackedToday: u.trackedToday,
            activity: u.trackedToday > 0 ? u.productivity : null,
            projectIds,
            isOwner: u.role === "owner",
          };
        }),
    [demo],
  );

  // Live: the fetched roster (empty after a failure); never demo people. Live
  // status and today's figures come from presence and the reporting summary when the server has them.
  const members = useMemo(() => {
    if (!isLive) return demoMembers;
    return (liveMembers ?? []).map((m) => {
      const t = today?.get(m.id);
      return {
        ...m,
        status: presence ? (presence.get(m.id) ?? "offline") : m.status,
        trackedToday: t ? t.minutes : m.trackedToday,
        activity: t ? (t.minutes > 0 ? t.activity : null) : m.activity,
      };
    });
  }, [isLive, demoMembers, liveMembers, presence, today]);

  const projectOptions = useMemo<ProjectOption[]>(
    () => liveProjects ?? projects.map((p) => ({ id: p.id, title: p.title, archived: p.archived })).sort((a, b) => a.title.localeCompare(b.title)),
    [liveProjects],
  );
  const activeProjects = projectOptions.filter((p) => !p.archived);

  const seatLimit = seatLimitFor(workspace);
  // Billing counts a seat per member holding a project membership (the owner
  // always occupies one) — mirror that for a live tenant.
  const seatsUsed = isLive
    ? liveMembers
      ? Math.max(1, liveMembers.filter((m) => m.projectIds.length > 0).length)
      : workspace.seatsUsed
    : countSeats(members);
  const seatPct = usagePct(seatsUsed, seatLimit);
  const seatsFull = seatLimit !== Number.POSITIVE_INFINITY && seatsUsed >= seatLimit;
  const canManage = user.role === "owner" || user.role === "admin";
  const activeNow = members.filter((u) => u.status === "active").length;

  // A live roster never says who is a client (the role is per project), so that filter would always be empty there.
  const visibleFilters = isLive ? filters.filter((f) => f.key !== "client") : filters;
  const filtered = useMemo(
    () =>
      members.filter(
        (u) =>
          (role === "all" || u.role === role) &&
          (u.name.toLowerCase().includes(query.toLowerCase()) || u.email.toLowerCase().includes(query.toLowerCase())),
      ),
    [query, role, members],
  );

  // Anyone but the viewer (nobody changes their own memberships); clients join projects too, as clients.
  const people = useMemo(
    () => members.filter((m) => m.id !== user.id).map((m) => ({ id: m.id, name: m.name, email: m.email })),
    [members, user.id],
  );
  const membersOfProject = useCallback(
    (projectId: string) => new Set(members.filter((m) => m.projectIds.includes(projectId)).map((m) => m.id)),
    [members],
  );
  const nameOf = useCallback(
    (userId: string, hint: string | null) => members.find((m) => m.id === userId)?.name ?? hint ?? "Team member",
    [members],
  );

  async function addMember(input: CreateProjectMemberDto) {
    const name = members.find((m) => m.id === input.userId)?.name ?? "They";
    const title = projectOptions.find((p) => p.id === input.projectId)?.title ?? "the project";
    if (!isLive) {
      setDemo((d) => ({ ...d, added: { ...d.added, [input.userId]: [...(d.added[input.userId] ?? []), input.projectId] } }));
      toast({ title: `${name} added to ${title}`, description: DEMO_NOTE });
      return;
    }
    await addProjectMember(input);
    onTeamChanged();
    toast({ title: `${name} added to ${title}`, description: "Their desktop agent offers the project within five minutes." });
  }

  async function removeFromWorkspace(member: MemberView) {
    if (!isLive) {
      setDemo((d) => ({ ...d, gone: [...d.gone, member.id] }));
      setLeaving(null);
      setEditing(null);
      toast({ title: `${member.name} removed from the workspace`, description: DEMO_NOTE });
      return;
    }
    setLeavingBusy(true);
    try {
      await removeMemberFromWorkspace(member.id);
      setLeaving(null);
      setEditing(null);
      onTeamChanged();
      toast({
        title: `${member.name} removed from the workspace`,
        description: "They can't sign in or track time any more, and their seat is free. Their tracked time stays in reports.",
      });
    } catch (err) {
      toast({ tone: "danger", title: "Couldn't remove them", description: membershipErrorText(err, "The change wasn't saved.") });
    } finally {
      setLeavingBusy(false);
    }
  }

  const roster = (
    <>
      <Toolbar>
        <SearchField value={query} onChange={setQuery} placeholder="Search members…" />
        <SegmentedControl
          value={role}
          onChange={setRole}
          className="flex-nowrap overflow-x-auto"
          options={visibleFilters.map((f) => ({ value: f.key, label: f.label }))}
        />
      </Toolbar>

      {liveError && (
        <div role="alert" className="flex items-center gap-3 rounded-lg border border-danger/30 bg-danger/5 px-3 py-2 text-sm text-danger-on-tint">
          <span className="flex-1">{liveError}</span>
          <Button size="sm" variant="outline" onClick={() => void loadRoster()}>
            Try again
          </Button>
        </div>
      )}

      {liveLoading && !liveMembers ? (
        <p role="status" className="py-10 text-center text-sm text-muted-foreground">
          Loading team…
        </p>
      ) : isLive && liveError && members.length === 0 ? null : filtered.length === 0 ? (
        <EmptyState
          icon={Users}
          title="No members match"
          description="Try another role filter or clear the search."
          action={{
            label: "Reset filters",
            onClick: () => {
              setQuery("");
              setRole("all");
            },
          }}
        />
      ) : (
        <ul className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">
          {filtered.map((u) => {
            const manageable = canManage && !u.isOwner && u.id !== user.id;
            return (
              <li key={u.id} className="min-w-0">
                <Card className="h-full min-w-0 p-5">
                  <div className="flex items-start justify-between gap-2">
                    <div className="flex min-w-0 items-center gap-3">
                      <Avatar name={u.name} size="lg" status={u.status} />
                      <div className="min-w-0">
                        <h2 className="truncate font-semibold leading-tight" title={u.name}>
                          {u.name}
                        </h2>
                        <Badge tone={roleTone[u.role]} className="mt-1">
                          {roleLabels[u.role]}
                        </Badge>
                      </div>
                    </div>
                    {manageable && (
                      <Button
                        size="icon"
                        variant="ghost"
                        className="h-8 w-8 shrink-0 text-muted-foreground"
                        onClick={() => setEditing(u)}
                        aria-label={`Edit ${u.name}`}
                        title="Role, rate, projects and access"
                      >
                        <Pencil className="h-4 w-4" />
                      </Button>
                    )}
                  </div>

                  <div className="mt-4 flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                    <Mail aria-hidden className="h-3.5 w-3.5 shrink-0" />
                    {u.email ? (
                      <a href={`mailto:${u.email}`} className="min-w-0 truncate hover:text-foreground hover:underline" title={u.email}>
                        {u.email}
                      </a>
                    ) : (
                      <span>No email on file</span>
                    )}
                  </div>

                  <div className="mt-4 flex items-center justify-between gap-3 rounded-xl border border-border/60 bg-muted/40 p-3">
                    <div className="space-y-2">
                      <p className="flex items-center gap-1.5 text-sm">
                        <Clock aria-hidden className="h-4 w-4 text-muted-foreground" />
                        <span className="font-medium tabular-nums">{u.role === "client" ? "—" : formatDuration(u.trackedToday)}</span>
                        <span className="text-xs text-muted-foreground">today</span>
                      </p>
                      <p className="flex items-center gap-1.5 text-sm">
                        <FolderKanban aria-hidden className="h-4 w-4 text-muted-foreground" />
                        <span className="font-medium">{u.projectIds.length}</span>
                        <span className="text-xs text-muted-foreground">{u.projectIds.length === 1 ? "project" : "projects"}</span>
                      </p>
                    </div>
                    {u.role !== "client" && u.activity !== null && <ActivityRing value={u.activity} size={56} stroke={6} caption="activity today" />}
                  </div>

                  {manageable && (
                    <div className="mt-3 flex flex-wrap gap-2">
                      <Button size="sm" variant="outline" onClick={() => setAdding({ personId: u.id })}>
                        <UserPlus className="h-3.5 w-3.5" /> Add to project
                      </Button>
                    </div>
                  )}
                </Card>
              </li>
            );
          })}
        </ul>
      )}
    </>
  );

  return (
    <PageStack>
      <PageHeader
        eyebrow="Workspace"
        title="Team"
        description={`${pluralMembers(seatsUsed)} · ${activeNow} tracking now`}
        actions={
          canManage ? (
            <>
              <Button variant="outline" onClick={() => setAdding({ personId: null })}>
                <UserPlus className="h-4 w-4" /> Add to project
              </Button>
              <Button onClick={() => setInvite(true)}>
                <Plus className="h-4 w-4" /> Invite member
              </Button>
            </>
          ) : undefined
        }
      />

      <Card variant="quiet" className="p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <div className="text-sm font-medium">
              Seats · <span className="text-muted-foreground">{planNameFor(workspace)} plan</span>
            </div>
            <div className="text-xs text-muted-foreground">
              {seatsUsed} of {fmtLimit(seatLimit)} seats used
              {seatsFull && " · limit reached — inviting someone new needs a free seat"}
            </div>
          </div>
          {user.role === "owner" && (seatPct >= 80 || seatsFull) && (
            <Link href="/billing" className={linkButtonClass(seatsFull ? "primary" : "outline", "sm")}>
              Upgrade plan
            </Link>
          )}
        </div>
        <div className="mt-3">
          <Progress
            value={seatPct}
            aria-label="Seats used"
            color={seatsFull ? "var(--danger)" : seatPct >= 80 ? "var(--warning)" : undefined}
          />
        </div>
      </Card>

      <ViewTabs value={view} onChange={setView} />
      <div role="tabpanel" id={`team-${view}-panel`} aria-labelledby={`team-${view}-tab`} className="space-y-6">
        {view === "members" ? roster : <DevicesPanel live={isLive} zone={zone} nameOf={nameOf} />}
      </div>

      {invite && (
        <InviteModal
          onClose={() => setInvite(false)}
          inviterRole={user.role}
          live={isLive}
          projects={activeProjects}
          workspaceName={workspace.name}
          onInvited={onTeamChanged}
        />
      )}
      {adding && (
        <AddMemberDialog
          people={people}
          projects={activeProjects}
          initialPersonId={adding.personId}
          memberIdsOf={membersOfProject}
          rulesFor={(personId) => editRulesFor(user, personId)}
          onClose={() => setAdding(null)}
          onSubmit={addMember}
        />
      )}
      {editing && (
        <ManageMemberModal
          member={editing}
          viewer={user}
          live={isLive}
          projects={projectOptions}
          workspaceName={workspace.name}
          targetIsOwner={members.find((m) => m.id === editing.id)?.isOwner ?? false}
          onClose={() => setEditing(null)}
          onChanged={onTeamChanged}
          onDemoChange={(change) =>
            setDemo((d) => ({
              ...d,
              roles: change.role ? { ...d.roles, [editing.id]: change.role } : d.roles,
              removed: change.removedProjectId
                ? { ...d.removed, [editing.id]: [...(d.removed[editing.id] ?? []), change.removedProjectId] }
                : d.removed,
            }))
          }
          onAddToProject={() => {
            const personId = editing.id;
            setEditing(null);
            setAdding({ personId });
          }}
          onRemoveFromWorkspace={() => setLeaving(editing)}
        />
      )}
      <ConfirmDialog
        open={!!leaving}
        onCancel={() => setLeaving(null)}
        onConfirm={() => leaving && void removeFromWorkspace(leaving)}
        title={leaving ? `Remove ${leaving.name} from the workspace?` : "Remove from the workspace?"}
        description="They're taken off every project and their account is deactivated: they can't sign in or track time, and their seat is freed from the next invoice. Time they already tracked stays in reports."
        confirmLabel="Remove from workspace"
        busy={leavingBusy}
      />
    </PageStack>
  );
}

/* ----------------------------- View switch ----------------------------- */

function ViewTabs({ value, onChange }: { value: "members" | "devices"; onChange: (v: "members" | "devices") => void }) {
  const tabs = [
    { key: "members" as const, label: "Members", icon: Users },
    { key: "devices" as const, label: "Devices", icon: MonitorSmartphone },
  ];
  // WAI-ARIA tabs: arrow keys move between them; only the selected one is in the Tab order.
  function onKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const next = value === "members" ? "devices" : "members";
    onChange(next);
    document.getElementById(`team-${next}-tab`)?.focus();
  }
  return (
    <div role="tablist" aria-label="Team views" onKeyDown={onKeyDown} className="inline-flex max-w-full items-center overflow-x-auto rounded-xl bg-muted p-1">
      {tabs.map((t) => {
        const active = t.key === value;
        return (
          <button
            key={t.key}
            type="button"
            role="tab"
            id={`team-${t.key}-tab`}
            aria-selected={active}
            aria-controls={active ? `team-${t.key}-panel` : undefined}
            tabIndex={active ? 0 : -1}
            onClick={() => onChange(t.key)}
            className={cn(
              "inline-flex h-9 shrink-0 items-center gap-2 whitespace-nowrap rounded-lg px-4 text-sm font-medium transition-colors",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              // On the muted track the muted text colour falls under 4.5:1; a softened foreground clears it in both themes.
              active ? "bg-card text-foreground shadow-sm" : "text-foreground/75 hover:text-foreground",
            )}
          >
            <t.icon aria-hidden className={cn("h-4 w-4", active && "text-primary")} />
            {t.label}
          </button>
        );
      })}
    </div>
  );
}

/* ----------------------------- Invite ----------------------------- */

/**
 * Pragmatic address check: one @, no whitespace, a dotted domain. Deliberately
 * not RFC 5322 — the goal is to catch typos and pasted junk before an invite
 * is sent, not to adjudicate exotic-but-legal addresses.
 */
function isEmailAddress(value: string): boolean {
  return /^[^\s@]+@[^\s@.]+(\.[^\s@.]+)+$/.test(value);
}

interface InviteOutcome {
  email: string;
  ok: boolean;
  userCreated?: boolean;
  initialSecret?: string | null;
  error?: string;
}

function InviteModal({
  onClose,
  inviterRole,
  live,
  projects: projectChoices,
  workspaceName,
  onInvited,
}: {
  onClose: () => void;
  inviterRole: Role;
  live: boolean;
  /** Every active project, sorted by title. */
  projects: ProjectOption[];
  workspaceName: string;
  onInvited?: () => void;
}) {
  const roles = inviteableRoles({ role: inviterRole });
  const canSetRate = inviterRole === "owner";
  const [emails, setEmails] = useState("");
  const [role, setRole] = useState<Role>(roles[0] ?? "worker");
  const [project, setProject] = useState(projectChoices[0]?.id ?? "");
  const [sent, setSent] = useState(false);
  const [hourlyRate, setHourlyRate] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [results, setResults] = useState<InviteOutcome[] | null>(null);

  async function send() {
    const emailList = emails.split(",").map((e) => e.trim()).filter(Boolean);
    if (emailList.length === 0) {
      setFormError("Enter at least one email address.");
      return;
    }
    // Reject malformed addresses before anything is sent — naming the offender
    // so the user can correct it rather than guess.
    const invalid = emailList.filter((e) => !isEmailAddress(e));
    if (invalid.length > 0) {
      setFormError(`Not a valid email address: ${invalid.join(", ")}`);
      return;
    }

    if (!live) {
      setFormError(null);
      setSent(true);
      setTimeout(() => {
        setSent(false);
        setEmails("");
        onClose();
      }, 1200);
      return;
    }

    if (!project) {
      setFormError("Select a project first.");
      return;
    }
    const rate = canSetRate ? parseHourlyRate(hourlyRate) : 0;
    if (rate === null) {
      setFormError("Hourly rate must be a number from 0 to 10,000 with at most two decimals.");
      return;
    }

    setSubmitting(true);
    setFormError(null);
    const outcomes: InviteOutcome[] = [];
    for (const email of emailList) {
      try {
        const res = await inviteMember({ projectId: project, email, role: inviteRoleFor[role], hourlyRate: rate });
        outcomes.push({ email, ok: true, userCreated: !!res?.userCreated, initialSecret: res?.initialPassword ?? null });
      } catch (err) {
        outcomes.push({ email, ok: false, error: membershipErrorText(err, "The invite failed.") });
      }
    }
    setSubmitting(false);
    if (outcomes.some((o) => o.ok)) onInvited?.();

    if (outcomes.length === 1 && !outcomes[0].ok) {
      // Single failed invite (e.g. seat limit reached): keep the form so the
      // user can adjust and retry, with the API error surfaced inline.
      setFormError(outcomes[0].error ?? "The invite failed.");
      return;
    }
    setResults(outcomes);
    setEmails("");
  }

  return (
    <Modal open onClose={onClose} title="Invite members" description="Add people to a project by email.">
      {sent ? (
        <div role="status" className="flex flex-col items-center gap-3 py-8 text-center">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-success/15 text-success-on-tint">
            <Send aria-hidden className="h-6 w-6" />
          </div>
          <p className="text-sm font-medium">Invitations sent!</p>
          <p className="text-xs text-muted-foreground">Members will receive an email to join.</p>
        </div>
      ) : results ? (
        <div className="space-y-4">
          {results.map((r, i) => (
            <div key={`${r.email}-${i}`} className="rounded-xl border border-border/60 bg-muted/40 p-3">
              <div className="flex items-center justify-between gap-2">
                <span className="truncate text-sm font-medium">{r.email}</span>
                {r.ok ? <Badge tone="success">{r.userCreated ? "Account created" : "Added"}</Badge> : <Badge tone="danger">Failed</Badge>}
              </div>
              {r.ok && !r.userCreated && (
                <p className="mt-1 text-xs text-muted-foreground">
                  They already had an account here and were added to the project. They sign in with their usual details.
                </p>
              )}
              {r.ok && r.userCreated && r.initialSecret && (
                <SignInDetails
                  className="mt-3"
                  workspaceName={workspaceName}
                  email={r.email}
                  secret={{ label: <>One-time password</>, value: r.initialSecret }}
                >
                  Share these securely — the one-time password is shown only once. They sign in to the web app or the desktop agent
                  with them, then choose their own password in Settings → Security.
                </SignInDetails>
              )}
              {!r.ok && <p className="mt-1 text-xs text-danger-on-tint">{r.error}</p>}
            </div>
          ))}
          <div className="flex justify-end gap-2 pt-2">
            <Button variant="ghost" onClick={() => setResults(null)}>
              Invite more
            </Button>
            <Button onClick={onClose}>Done</Button>
          </div>
        </div>
      ) : (
        <div className="space-y-4">
          <div className="space-y-1.5">
            <label htmlFor="invite-emails" className="text-sm font-medium">
              Email addresses
            </label>
            <Input
              id="invite-emails"
              value={emails}
              onChange={(e) => setEmails(e.target.value)}
              placeholder="jane@company.com, john@company.com"
              aria-describedby="invite-emails-hint"
            />
            <p id="invite-emails-hint" className="text-xs text-muted-foreground">
              Separate multiple emails with commas.
            </p>
          </div>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div className="space-y-1.5">
              <label htmlFor="invite-role" className="text-sm font-medium">
                Role
              </label>
              <Select id="invite-role" value={role} onChange={(e) => setRole(e.target.value as Role)}>
                {roles.map((r) => (
                  <option key={r} value={r}>
                    {roleLabels[r]}
                  </option>
                ))}
              </Select>
            </div>
            <div className="space-y-1.5">
              <label htmlFor="invite-project" className="text-sm font-medium">
                Project
              </label>
              <Select id="invite-project" value={project} onChange={(e) => setProject(e.target.value)} disabled={projectChoices.length === 0}>
                {projectChoices.length === 0 ? (
                  <option value="">No projects yet</option>
                ) : (
                  projectChoices.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.title}
                    </option>
                  ))
                )}
              </Select>
            </div>
          </div>
          {projectChoices.length === 0 && (
            <p className="rounded-lg bg-muted/60 px-3 py-2 text-xs text-muted-foreground">
              People join a project when invited.{" "}
              <Link href="/projects" className="font-medium text-primary-strong underline underline-offset-2">
                Create a project first
              </Link>
              .
            </p>
          )}
          {live && canSetRate && (
            <div className="space-y-1.5">
              <label htmlFor="invite-rate" className="text-sm font-medium">
                Hourly rate (USD) <span className="font-normal text-muted-foreground">(optional)</span>
              </label>
              <Input id="invite-rate" inputMode="decimal" value={hourlyRate} onChange={(e) => setHourlyRate(e.target.value)} placeholder="e.g. 25" />
            </div>
          )}
          {formError && (
            <p role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
              {formError}
            </p>
          )}
          <div className="flex justify-end gap-2 pt-2">
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button onClick={() => void send()} disabled={!emails.trim() || roles.length === 0 || submitting || (live && !project)}>
              <Send className="h-4 w-4" /> {submitting ? "Sending…" : "Send invites"}
            </Button>
          </div>
        </div>
      )}
    </Modal>
  );
}

/* ----------------------------- Manage a member ----------------------------- */

type Prefill =
  | { state: "loading" }
  | { state: "failed"; message: string }
  | { state: "ready"; membership: ProjectMemberDto | null };

type ResetState =
  | { step: "idle" }
  | { step: "confirm" }
  | { step: "working" }
  | { step: "done"; secret: string | null }
  | { step: "failed"; message: string };

/**
 * One member's place in the workspace: their role and hourly rate on each of
 * their projects (PUT /api/app/team/member), removing them from a project,
 * adding them to another, a new one-time password, and removing them from the
 * workspace — each offered only when the viewer may do it (contract §7).
 */
function ManageMemberModal({
  member,
  viewer,
  live,
  projects: allProjects,
  workspaceName,
  targetIsOwner,
  onClose,
  onChanged,
  onDemoChange,
  onAddToProject,
  onRemoveFromWorkspace,
}: {
  member: MemberView;
  viewer: { id: string; role: Role };
  live: boolean;
  projects: ProjectOption[];
  workspaceName: string;
  targetIsOwner: boolean;
  onClose: () => void;
  onChanged: () => void;
  onDemoChange: (change: { role?: Role; removedProjectId?: string }) => void;
  onAddToProject: () => void;
  onRemoveFromWorkspace: () => void;
}) {
  const memberProjects = useMemo(
    () => allProjects.filter((p) => member.projectIds.includes(p.id)),
    [allProjects, member.projectIds],
  );
  const rules = editRulesFor(viewer, member.id);
  const account = memberAccountActions({
    canManageTeam: viewer.role === "owner" || viewer.role === "admin",
    callerIsOwner: viewer.role === "owner",
    targetIsOwner,
    isSelf: viewer.id === member.id,
  });
  const [projectId, setProjectId] = useState(memberProjects[0]?.id ?? "");
  const [prefillKey, setPrefillKey] = useState(0);
  // The membership read for one project (and retry); anything else on screen is still loading.
  const [loaded, setLoaded] = useState<{ key: string; result: Prefill } | null>(null);
  // The demo has no memberships to read: the member's own role stands in, with no rate.
  const [role, setRole] = useState<ProjectRole>(member.role === "admin" ? "Admin" : member.role === "client" ? "Client" : "Worker");
  const [rate, setRate] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [reset, setReset] = useState<ResetState>({ step: "idle" });

  const prefillFor = `${projectId}:${prefillKey}`;
  const prefill: Prefill = !live
    ? { state: "ready", membership: null }
    : loaded?.key === prefillFor
      ? loaded.result
      : { state: "loading" };

  // Prefill role + rate from the selected project's current membership. Save
  // stays off until it has loaded: a failed read must never be saved as rate 0.
  useEffect(() => {
    if (!live || !projectId) return;
    let cancelled = false;
    const key = `${projectId}:${prefillKey}`;
    getProjectMembers(projectId)
      .then((rows) => {
        if (cancelled) return;
        const row = rows.find((r) => r.userId === member.id) ?? null;
        setRole(projectRoleOf(row?.role));
        setRate(row && typeof row.hourlyRate === "number" ? String(row.hourlyRate) : "");
        setLoaded({ key, result: { state: "ready", membership: row } });
      })
      .catch((err) => {
        if (!cancelled) setLoaded({ key, result: { state: "failed", message: requestErrorText(err, "Couldn't load their current role and rate.") } });
      });
    return () => {
      cancelled = true;
    };
  }, [live, member.id, projectId, prefillKey]);

  const ready = prefill.state === "ready";
  const membership = ready ? prefill.membership : null;
  const currentRole = membership ? projectRoleOf(membership.role) : role;
  const choices = roleChoices(rules, currentRole);
  const projectTitle = memberProjects.find((p) => p.id === projectId)?.title ?? "this project";

  async function save() {
    if (!projectId || !ready) return;
    const hourlyRate = rules.canChangeRate ? parseHourlyRate(rate) : (membership?.hourlyRate ?? 0);
    if (hourlyRate === null) {
      setError("Hourly rate must be a number from 0 to 10,000 with at most two decimals.");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      if (live) {
        await updateProjectMember(projectId, member.id, { role, hourlyRate });
        onChanged();
        toast({ tone: "success", title: "Member updated", description: `${member.name}'s role and rate on ${projectTitle} were saved.` });
      } else {
        onDemoChange({ role: role === "Admin" || role === "Manager" ? "admin" : role === "Client" ? "client" : "worker" });
        toast({ tone: "success", title: "Member updated", description: DEMO_NOTE });
      }
      onClose();
    } catch (err) {
      setError(membershipErrorText(err, "Couldn't save the change."));
    } finally {
      setSaving(false);
    }
  }

  async function removeFromProject() {
    setRemoving(true);
    try {
      if (live) {
        if (!membership?.id) throw new Error("This server didn't say which membership to remove. Refresh and try again.");
        await removeProjectMember(membership.id);
        onChanged();
        toast({ title: `${member.name} removed from ${projectTitle}`, description: "Their time already tracked stays in reports." });
      } else {
        onDemoChange({ removedProjectId: projectId });
        toast({ title: `${member.name} removed from ${projectTitle}`, description: DEMO_NOTE });
      }
      setConfirmRemove(false);
      onClose();
    } catch (err) {
      setConfirmRemove(false);
      setError(membershipErrorText(err, "Couldn't remove them from the project."));
    } finally {
      setRemoving(false);
    }
  }

  async function issueReset() {
    if (!live) {
      setReset({ step: "done", secret: null });
      return;
    }
    setReset({ step: "working" });
    try {
      const res = await resetMemberPassword(member.id);
      setReset({ step: "done", secret: typeof res?.initialPassword === "string" ? res.initialPassword : null });
    } catch (err) {
      setReset({ step: "failed", message: requestErrorText(err, "The reset failed.") });
    }
  }

  return (
    <Modal open onClose={onClose} title={`Edit ${member.name}`} description="Role and hourly rate apply per project.">
      <div className="space-y-6">
        {memberProjects.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {member.name} isn&apos;t on any project yet. Add them to one — role and rate are set per project.
          </p>
        ) : (
          <section aria-labelledby="manage-projects-heading" className="space-y-4">
            <h3 id="manage-projects-heading" className="sr-only">
              Project role and rate
            </h3>
            <div className="space-y-1.5">
              <label htmlFor="edit-project" className="text-sm font-medium">
                Project
              </label>
              <Select
                id="edit-project"
                value={projectId}
                onChange={(e) => {
                  setProjectId(e.target.value);
                  setError(null);
                }}
              >
                {memberProjects.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.archived ? `${p.title} (archived)` : p.title}
                  </option>
                ))}
              </Select>
            </div>
            {prefill.state === "failed" ? (
              <div role="alert" className="flex flex-wrap items-center gap-3 rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
                <span className="min-w-0 flex-1">{prefill.message}</span>
                <Button size="sm" variant="outline" onClick={() => setPrefillKey((k) => k + 1)}>
                  <RotateCw className="h-3.5 w-3.5" /> Retry
                </Button>
              </div>
            ) : (
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <div className="space-y-1.5">
                  <label htmlFor="edit-role" className="text-sm font-medium">
                    Role
                  </label>
                  <Select
                    id="edit-role"
                    value={roleValue(role)}
                    onChange={(e) => setRole(roleFromValue(e.target.value))}
                    disabled={!ready || !rules.canChangeRole}
                  >
                    {choices.map((r) => (
                      <option key={r} value={roleValue(r)}>
                        {PROJECT_ROLE_LABELS[r]}
                      </option>
                    ))}
                  </Select>
                </div>
                <div className="space-y-1.5">
                  <label htmlFor="edit-rate" className="text-sm font-medium">
                    Hourly rate (USD)
                  </label>
                  <Input
                    id="edit-rate"
                    type="number"
                    min={0}
                    max={10000}
                    step="0.01"
                    value={rate}
                    onChange={(e) => setRate(e.target.value)}
                    placeholder="0"
                    disabled={!ready || !rules.canChangeRate}
                    aria-describedby={rules.canChangeRate ? undefined : "edit-rate-hint"}
                  />
                  {!rules.canChangeRate && (
                    <p id="edit-rate-hint" className="text-xs text-muted-foreground">
                      Only the workspace owner changes rates.
                    </p>
                  )}
                </div>
              </div>
            )}
            {prefill.state === "loading" && (
              <p role="status" className="text-xs text-muted-foreground">
                Loading their current role and rate…
              </p>
            )}
            {error && (
              <p role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
                {error}
              </p>
            )}
            <div className="flex flex-wrap items-center justify-between gap-2">
              <Button
                variant="ghost"
                size="sm"
                className="text-danger-on-tint hover:bg-danger/10"
                onClick={() => setConfirmRemove(true)}
                disabled={!ready || saving || (live && !membership?.id)}
              >
                <UserMinus className="h-3.5 w-3.5" /> Remove from this project
              </Button>
              <div className="flex gap-2">
                <Button variant="ghost" onClick={onClose}>
                  Cancel
                </Button>
                <Button onClick={() => void save()} disabled={saving || !ready || !projectId}>
                  {saving ? "Saving…" : "Save changes"}
                </Button>
              </div>
            </div>
          </section>
        )}

        <div className="flex flex-wrap gap-2 border-t border-border pt-4">
          <Button variant="outline" size="sm" onClick={onAddToProject}>
            <UserPlus className="h-3.5 w-3.5" /> Add to another project
          </Button>
        </div>

        {(account.canResetPassword || account.canRemoveFromWorkspace) && (
          <section aria-labelledby="manage-account-heading" className="space-y-3 border-t border-border pt-4">
            <h3 id="manage-account-heading" className="text-sm font-semibold">
              Account
            </h3>
            {account.canResetPassword && (
              <div className="rounded-xl border border-border p-3">
                {reset.step === "done" ? (
                  reset.secret ? (
                    <SignInDetails
                      heading={<>New sign-in details for {member.name}</>}
                      workspaceName={workspaceName}
                      email={member.email || null}
                      secret={{ label: <>One-time password</>, value: reset.secret }}
                    >
                      Shown only once — share it securely. Their old password no longer works, and they choose a new one the next
                      time they sign in.
                    </SignInDetails>
                  ) : (
                    <p role="status" className="text-sm text-muted-foreground">
                      {live ? (
                        <>The password was reset, but this server didn&apos;t return the new one. Reset it again to get a password to share.</>
                      ) : (
                        <>Demo workspace — no password was issued. In a live workspace the one-time password appears here to share.</>
                      )}
                    </p>
                  )
                ) : reset.step === "confirm" || reset.step === "working" ? (
                  <div className="space-y-3">
                    <p className="text-sm">
                      Give {member.name} a new one-time password? Their current password stops working at once, and the desktop agent
                      asks them to sign in again.
                    </p>
                    <div className="flex justify-end gap-2">
                      <Button variant="ghost" size="sm" onClick={() => setReset({ step: "idle" })} disabled={reset.step === "working"}>
                        Cancel
                      </Button>
                      <Button size="sm" variant="danger" onClick={() => void issueReset()} disabled={reset.step === "working"}>
                        <KeyRound className="h-3.5 w-3.5" /> {reset.step === "working" ? "Resetting…" : <>Reset password</>}
                      </Button>
                    </div>
                  </div>
                ) : (
                  <div className="flex flex-wrap items-center justify-between gap-3">
                    <div className="min-w-0 text-sm">
                      <div className="font-medium">Reset password</div>
                      <div className="text-xs text-muted-foreground">Issue a one-time password when they&apos;re locked out.</div>
                      {reset.step === "failed" && (
                        <div role="alert" className="mt-1 text-xs text-danger-on-tint">
                          {reset.message}
                        </div>
                      )}
                    </div>
                    <Button variant="outline" size="sm" onClick={() => setReset({ step: "confirm" })}>
                      <KeyRound className="h-3.5 w-3.5" /> Reset…
                    </Button>
                  </div>
                )}
              </div>
            )}
            {account.canRemoveFromWorkspace && (
              <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-danger/30 bg-danger/5 p-3">
                <div className="min-w-0 text-sm">
                  <div className="font-medium text-danger-on-tint">Remove from workspace</div>
                  <div className="text-xs text-muted-foreground">Takes them off every project, deactivates the account and frees the seat.</div>
                </div>
                <Button variant="outline" size="sm" className="border-danger/40 text-danger-on-tint hover:bg-danger/10" onClick={onRemoveFromWorkspace}>
                  <UserX className="h-3.5 w-3.5" /> Remove…
                </Button>
              </div>
            )}
          </section>
        )}
      </div>

      <ConfirmDialog
        open={confirmRemove}
        onCancel={() => setConfirmRemove(false)}
        onConfirm={() => void removeFromProject()}
        title={`Remove ${member.name} from ${projectTitle}?`}
        description="They can no longer track time on this project, and their desktop agent stops offering it within five minutes. Time they already tracked stays in reports. If it was their only project, their seat is freed from the next invoice."
        confirmLabel="Remove from project"
        busy={removing}
      />
    </Modal>
  );
}
