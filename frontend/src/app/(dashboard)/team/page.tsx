"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Plus, Mail, Clock, FolderKanban, Send, Users, Copy, Check, Pencil } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Ring } from "@/components/ui/ring";
import { Modal } from "@/components/ui/modal";
import { Input, Select } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { EmptyState } from "@/components/ui/empty-state";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { SearchField, SegmentedControl, Toolbar } from "@/components/ui/toolbar";
import { activities, projects, users } from "@/lib/tenant-data";
import { useSession } from "@/components/session-provider";
import { countSeats, inviteableRoles } from "@/lib/roles";
import { fmtLimit, planNameFor, seatLimitFor, usagePct } from "@/lib/saas-data";
import type { Role, UserStatus } from "@/lib/types";
import { formatDuration } from "@/lib/utils";
import { getApi, postApi, putApi } from "@/hooks/useApi";
import { mapTeamMember, type ApiTeamMemberDto } from "@/lib/live-dataset";
import { referenceNow } from "@/lib/live-session";
import { toast } from "@/components/toast";
import Link from "next/link";

const roleTone: Record<Role, "primary" | "info" | "warning" | "muted"> = {
  host: "primary",
  owner: "primary",
  admin: "warning",
  worker: "info",
  client: "muted",
};

const filters: { key: Role | "all"; label: string }[] = [
  { key: "all", label: "All" },
  { key: "owner", label: "Owners" },
  { key: "worker", label: "Workers" },
  { key: "client", label: "Clients" },
  { key: "admin", label: "Admins" },
];

/** The subset of member fields the roster cards render — satisfied by both the
 * mock `User` records and live ABP identity users mapped below. */
interface MemberView {
  id: string;
  name: string;
  designation: string;
  email: string;
  role: Role;
  status: UserStatus;
  trackedToday: number;
  productivity: number;
  /** Live only: the projects this member belongs to (from the team roster). */
  projectIds?: string[];
}

/** Backend project-member roles ↔ the app's roles. */
const apiRoleFor: Partial<Record<Role, string>> = { worker: "Worker", admin: "Admin", client: "Client" };

export default function TeamPage() {
  const { user, workspace } = useSession();
  const [query, setQuery] = useState("");
  const [role, setRole] = useState<Role | "all">("all");
  const [invite, setInvite] = useState(false);

  // LIVE MODE: the real roster from GET /api/app/team/members (roles from the
  // backend's owner/manager flags, today's time from the member's real rows),
  // refetched after every invite/edit. A failure shows an error — never the
  // demo roster.
  const { isLive, refreshSession } = useSession();
  const [liveMembers, setLiveMembers] = useState<MemberView[] | null>(null);
  const [liveLoading, setLiveLoading] = useState(false);
  const [liveError, setLiveError] = useState<string | null>(null);
  const [editing, setEditing] = useState<MemberView | null>(null);

  const loadRoster = useCallback(async () => {
    setLiveLoading(true);
    setLiveError(null);
    try {
      const items = await getApi("/api/app/team/members");
      const now = referenceNow();
      const list = (Array.isArray(items) ? items : []) as ApiTeamMemberDto[];
      setLiveMembers(
        list
          .filter((m) => m && typeof m.userId === "string")
          .map((m) => ({ ...mapTeamMember(m, activities, now), projectIds: Array.isArray(m.projectIds) ? m.projectIds : [] })),
      );
    } catch {
      setLiveMembers(null);
      setLiveError("Couldn't load your team from the server.");
    } finally {
      setLiveLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!isLive) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void loadRoster();
  }, [isLive, loadRoster]);

  /** After an invite or edit: refresh this roster and the session's copy of it. */
  const onTeamChanged = useCallback(() => {
    void loadRoster();
    void refreshSession({ silent: true });
  }, [loadRoster, refreshSession]);

  // Live: the fetched roster (the session roster while it loads); never demo people.
  const members = useMemo<MemberView[]>(
    () => (isLive ? (liveMembers ?? (liveError ? [] : users)) : users),
    [isLive, liveMembers, liveError],
  );

  const seatLimit = seatLimitFor(workspace);
  // Billing counts a seat per member holding a project membership (the owner
  // always occupies one) — mirror that for a live tenant.
  const seatsUsed = isLive
    ? liveMembers
      ? Math.max(1, liveMembers.filter((m) => (m.projectIds ?? []).length > 0).length)
      : workspace.seatsUsed
    : countSeats(members);
  const seatPct = usagePct(seatsUsed, seatLimit);
  const seatsFull = seatLimit !== Number.POSITIVE_INFINITY && seatsUsed >= seatLimit;
  const canManage = user.role === "owner" || user.role === "admin";

  const filtered = useMemo(
    () =>
      members.filter(
        (u) =>
          (role === "all" || u.role === role) &&
          (u.name.toLowerCase().includes(query.toLowerCase()) ||
            u.email.toLowerCase().includes(query.toLowerCase()))
      ),
    [query, role, members]
  );

  const projectCount = (member: MemberView) =>
    member.projectIds ? member.projectIds.length : projects.filter((p) => (p.memberIds ?? []).includes(member.id)).length;

  return (
    <PageStack>
      <PageHeader
        eyebrow="Workspace"
        title="Team"
        description={`${seatsUsed} members · ${members.filter((u) => u.status === "active").length} active now`}
        actions={
          canManage ? (
            <Button onClick={() => setInvite(true)} disabled={seatsFull} title={seatsFull ? "Seat limit reached — upgrade your plan" : undefined}>
              <Plus className="h-4 w-4" /> Invite member
            </Button>
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
              {seatsFull && " · limit reached"}
            </div>
          </div>
          {user.role === "owner" && (seatPct >= 80 || seatsFull) && (
            <Link href="/billing">
              <Button size="sm" variant={seatsFull ? "primary" : "outline"}>Upgrade plan</Button>
            </Link>
          )}
        </div>
        <div className="mt-3">
          <Progress value={seatPct} color={seatsFull ? "var(--danger)" : seatPct >= 80 ? "var(--warning)" : undefined} />
        </div>
      </Card>

      <Toolbar>
        <SearchField value={query} onChange={setQuery} placeholder="Search members…" />
        <SegmentedControl
          value={role}
          onChange={setRole}
          options={filters.map((f) => ({ value: f.key, label: f.label }))}
        />
      </Toolbar>

      {liveError && (
        <div className="flex items-center gap-3 rounded-lg border border-danger/30 bg-danger/5 px-3 py-2 text-sm text-danger">
          <span className="flex-1">{liveError}</span>
          <Button size="sm" variant="outline" onClick={() => void loadRoster()}>Try again</Button>
        </div>
      )}

      {liveLoading && !liveMembers ? (
        <p className="py-10 text-center text-sm text-muted-foreground">Loading team…</p>
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
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">
        {filtered.map((u) => (
          <Card key={u.id} variant="interactive" className="min-w-0 p-5">
            <div className="flex items-start justify-between gap-2">
              <div className="flex min-w-0 items-center gap-3">
                <Avatar name={u.name} size="lg" status={u.status} />
                <div className="min-w-0">
                  <h2 className="truncate font-semibold leading-tight" title={u.name}>{u.name}</h2>
                  <p className="text-xs text-muted-foreground">{u.designation}</p>
                </div>
              </div>
              <div className="flex shrink-0 items-center gap-1.5">
                <Badge tone={roleTone[u.role]} className="capitalize">{u.role}</Badge>
                {isLive && canManage && u.role !== "owner" && u.id !== user.id && (
                  <button
                    onClick={() => setEditing(u)}
                    aria-label={`Edit ${u.name}`}
                    title="Edit role & hourly rate"
                    className="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
                  >
                    <Pencil className="h-3.5 w-3.5" />
                  </button>
                )}
              </div>
            </div>

            <div className="mt-4 flex items-center gap-1.5 text-xs text-muted-foreground">
              <Mail className="h-3.5 w-3.5 shrink-0" />
              <span className="min-w-0 truncate" title={u.email}>{u.email}</span>
            </div>

            <div className="mt-4 flex items-center justify-between rounded-xl border border-border/60 bg-muted/40 p-3">
              <div className="space-y-2">
                <div className="flex items-center gap-1.5 text-sm">
                  <Clock className="h-4 w-4 text-muted-foreground" />
                  <span className="font-medium tabular-nums">{u.role === "client" ? "—" : formatDuration(u.trackedToday)}</span>
                  <span className="text-xs text-muted-foreground">today</span>
                </div>
                <div className="flex items-center gap-1.5 text-sm">
                  <FolderKanban className="h-4 w-4 text-muted-foreground" />
                  <span className="font-medium">{projectCount(u)}</span>
                  <span className="text-xs text-muted-foreground">projects</span>
                </div>
              </div>
              {u.productivity > 0 && (
                <div className="text-center">
                  <Ring value={u.productivity} size={56} stroke={6} />
                  <div className="mt-1 text-[11px] text-muted-foreground">productivity</div>
                </div>
              )}
            </div>
          </Card>
        ))}
      </div>
      )}

      <InviteModal open={invite} onClose={() => setInvite(false)} inviterRole={user.role} live={isLive} onInvited={onTeamChanged} />
      {isLive && (
        <EditMemberModal
          member={editing}
          editorRole={user.role}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            onTeamChanged();
          }}
        />
      )}
    </PageStack>
  );
}

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
  initialPassword?: string | null;
  error?: string;
}

function InviteModal({
  open,
  onClose,
  inviterRole,
  live,
  onInvited,
}: {
  open: boolean;
  onClose: () => void;
  inviterRole: Role;
  live: boolean;
  onInvited?: () => void;
}) {
  const roles = inviteableRoles({ role: inviterRole });
  const [emails, setEmails] = useState("");
  const [role, setRole] = useState<Role>(roles[0] ?? "worker");
  const [project, setProject] = useState(projects[0]?.id ?? "");
  const [sent, setSent] = useState(false);

  // LIVE MODE: real project list + real invites against the backend.
  const [hourlyRate, setHourlyRate] = useState("");
  const [apiProjects, setApiProjects] = useState<{ id: string; title: string }[] | null>(null);
  const [projectsLoading, setProjectsLoading] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [results, setResults] = useState<InviteOutcome[] | null>(null);
  const [copied, setCopied] = useState<string | null>(null);

  useEffect(() => {
    if (!open) {
      // Fresh start on reopen — the one-time passwords are shown only once.
      setSent(false);
      setSubmitting(false);
      setFormError(null);
      setResults(null);
      setCopied(null);
      return;
    }
    if (!live) return;
    let cancelled = false;
    setProjectsLoading(true);
    getApi("/api/app/project")
      .then((items) => {
        if (cancelled) return;
        const list = (Array.isArray(items) ? items : []) as { id: string; title: string; isArchived?: boolean }[];
        const activeProjects = list.filter((p) => !p.isArchived).map((p) => ({ id: p.id, title: p.title }));
        setApiProjects(activeProjects);
        setProject((prev) => (activeProjects.some((p) => p.id === prev) ? prev : activeProjects[0]?.id ?? ""));
      })
      .catch(() => {
        if (!cancelled) setApiProjects(null); // fall back to the session's project list (real in a live session)
      })
      .finally(() => {
        if (!cancelled) setProjectsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, live]);

  const projectOptions = apiProjects ?? projects.filter((p) => !p.archived).map((p) => ({ id: p.id, title: p.title }));

  async function copyPassword(text: string, key: string) {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(key);
      setTimeout(() => setCopied(null), 1500);
    } catch {
      setCopied(null);
    }
  }

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

    setSubmitting(true);
    setFormError(null);
    const outcomes: InviteOutcome[] = [];
    for (const email of emailList) {
      try {
        const res = await postApi("/api/app/team/invite-member", {
          projectId: project,
          email,
          role: role.charAt(0).toUpperCase() + role.slice(1),
          ...(hourlyRate.trim() !== "" && !Number.isNaN(Number(hourlyRate))
            ? { hourlyRate: Number(hourlyRate) }
            : {}),
        });
        outcomes.push({
          email,
          ok: true,
          userCreated: !!res?.userCreated,
          initialPassword: res?.initialPassword ?? null,
        });
      } catch (err) {
        outcomes.push({ email, ok: false, error: err instanceof Error ? err.message : "Invite failed" });
      }
    }
    setSubmitting(false);
    if (outcomes.some((o) => o.ok)) onInvited?.();

    if (outcomes.length === 1 && !outcomes[0].ok) {
      // Single failed invite (e.g. seat limit reached): keep the form so the
      // user can adjust and retry, with the API error surfaced inline.
      setFormError(outcomes[0].error ?? "Invite failed");
      return;
    }
    setResults(outcomes);
    setEmails("");
  }

  return (
    <Modal open={open} onClose={onClose} title="Invite members" description="Send project invitations by email.">
      {sent ? (
        <div className="flex flex-col items-center gap-3 py-8 text-center">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-success/15 text-success">
            <Send className="h-6 w-6" />
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
                {r.ok ? (
                  <Badge tone="success">{r.userCreated ? "User created" : "Invited"}</Badge>
                ) : (
                  <Badge tone="danger">Failed</Badge>
                )}
              </div>
              {r.ok && !r.userCreated && (
                <p className="mt-1 text-xs text-muted-foreground">Existing user added to the project.</p>
              )}
              {r.ok && r.userCreated && r.initialPassword && (
                <div className="mt-2 space-y-1.5">
                  <div className="flex items-center gap-2">
                    <code className="flex-1 truncate rounded-lg border border-border bg-card px-2 py-1.5 font-mono text-sm">
                      {r.initialPassword}
                    </code>
                    <Button size="sm" variant="outline" onClick={() => copyPassword(r.initialPassword ?? "", r.email)}>
                      {copied === r.email ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
                      {copied === r.email ? "Copied" : "Copy"}
                    </Button>
                  </div>
                  <p className="text-xs text-warning">
                    One-time initial password — share it securely. It is shown only once and cannot be retrieved again.
                  </p>
                </div>
              )}
              {!r.ok && <p className="mt-1 text-xs text-danger">{r.error}</p>}
            </div>
          ))}
          <div className="flex justify-end gap-2 pt-2">
            <Button variant="ghost" onClick={() => setResults(null)}>Invite more</Button>
            <Button onClick={onClose}>Done</Button>
          </div>
        </div>
      ) : (
        <div className="space-y-4">
          <div className="space-y-1.5">
            <label htmlFor="invite-emails" className="text-sm font-medium">Email addresses</label>
            <Input id="invite-emails" value={emails} onChange={(e) => setEmails(e.target.value)} placeholder="jane@company.com, john@company.com" />
            <p className="text-xs text-muted-foreground">Separate multiple emails with commas.</p>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <label htmlFor="invite-role" className="text-sm font-medium">Role</label>
              <Select id="invite-role" value={role} onChange={(e) => setRole(e.target.value as Role)}>
                {roles.map((r) => (
                  <option key={r} value={r}>
                    {r === "worker" ? "Member" : r.charAt(0).toUpperCase() + r.slice(1)}
                  </option>
                ))}
              </Select>
            </div>
            <div className="space-y-1.5">
              <label htmlFor="invite-project" className="text-sm font-medium">Project</label>
              <Select id="invite-project" value={project} onChange={(e) => setProject(e.target.value)} disabled={live && projectsLoading}>
                {live && projectsLoading && projectOptions.length === 0 ? (
                  <option value="">Loading projects…</option>
                ) : projectOptions.length === 0 ? (
                  <option value="">No projects available</option>
                ) : (
                  projectOptions.map((p) => (
                    <option key={p.id} value={p.id}>{p.title}</option>
                  ))
                )}
              </Select>
            </div>
          </div>
          {live && (
            <div className="space-y-1.5">
              <label htmlFor="invite-rate" className="text-sm font-medium">
                Hourly rate <span className="font-normal text-muted-foreground">(optional)</span>
              </label>
              <Input
                id="invite-rate"
                type="number"
                min={0}
                max={10000}
                step="0.01"
                value={hourlyRate}
                onChange={(e) => setHourlyRate(e.target.value)}
                placeholder="e.g. 25"
              />
            </div>
          )}
          {formError && <p className="rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger">{formError}</p>}
          <div className="flex justify-end gap-2 pt-2">
            <Button variant="ghost" onClick={onClose}>Cancel</Button>
            <Button onClick={send} disabled={!emails.trim() || roles.length === 0 || submitting || (live && !project)}>
              <Send className="h-4 w-4" /> {submitting ? "Sending…" : "Send invites"}
            </Button>
          </div>
        </div>
      )}
    </Modal>
  );
}

interface ProjectMemberRow {
  userId: string;
  role?: string | null;
  hourlyRate?: number | null;
}

/**
 * Live only: change a member's role and hourly rate on one of their projects
 * (PUT /api/app/team/member?projectId=&userId=). Kept deliberately small — one
 * project at a time, prefilled from that project's current membership.
 */
function EditMemberModal({
  member,
  editorRole,
  onClose,
  onSaved,
}: {
  member: MemberView | null;
  editorRole: Role;
  onClose: () => void;
  onSaved: () => void;
}) {
  const memberProjects = useMemo(
    () => projects.filter((p) => (member?.projectIds ?? []).includes(p.id)),
    [member],
  );
  const roles = inviteableRoles({ role: editorRole }).filter((r) => apiRoleFor[r]);
  const [projectId, setProjectId] = useState("");
  const [role, setRole] = useState<Role>("worker");
  const [rate, setRate] = useState("");
  const [loadingRow, setLoadingRow] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Start on the member's first project each time the dialog opens.
  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- resets the form when a different member is opened
    setProjectId(memberProjects[0]?.id ?? "");
    setError(null);
  }, [memberProjects]);

  // Prefill role + rate from the selected project's current membership.
  useEffect(() => {
    if (!member || !projectId) return;
    let cancelled = false;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    setLoadingRow(true);
    getApi(`/api/app/team/project-members/${encodeURIComponent(projectId)}`)
      .then((rows) => {
        if (cancelled) return;
        const row = (Array.isArray(rows) ? (rows as ProjectMemberRow[]) : []).find((r) => r.userId === member.id);
        const current = (row?.role ?? "").toLowerCase();
        setRole(current === "admin" || current === "manager" ? "admin" : current === "client" ? "client" : "worker");
        setRate(row?.hourlyRate != null ? String(row.hourlyRate) : "");
      })
      .catch(() => {
        if (!cancelled) setRate("");
      })
      .finally(() => {
        if (!cancelled) setLoadingRow(false);
      });
    return () => {
      cancelled = true;
    };
  }, [member, projectId]);

  async function save() {
    if (!member || !projectId) return;
    const hourlyRate = rate.trim() === "" ? 0 : Number(rate);
    if (!Number.isFinite(hourlyRate) || hourlyRate < 0 || hourlyRate > 10000) {
      setError("Hourly rate must be a number between 0 and 10,000.");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await putApi(
        `/api/app/team/member?projectId=${encodeURIComponent(projectId)}&userId=${encodeURIComponent(member.id)}`,
        { role: apiRoleFor[role] ?? "Worker", hourlyRate },
      );
      toast({ tone: "success", title: "Member updated", description: `${member.name}'s role and rate were saved.` });
      onSaved();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Couldn't save the change.");
    } finally {
      setSaving(false);
    }
  }

  return (
    <Modal open={!!member} onClose={onClose} title={member ? `Edit ${member.name}` : "Edit member"} description="Role and hourly rate apply per project.">
      {member && memberProjects.length === 0 ? (
        <div className="space-y-4">
          <p className="text-sm text-muted-foreground">
            {member.name} isn&apos;t on any project yet. Invite them to a project first — role and rate are set per project.
          </p>
          <div className="flex justify-end">
            <Button variant="ghost" onClick={onClose}>Close</Button>
          </div>
        </div>
      ) : (
        <div className="space-y-4">
          <div className="space-y-1.5">
            <label htmlFor="edit-project" className="text-sm font-medium">Project</label>
            <Select id="edit-project" value={projectId} onChange={(e) => setProjectId(e.target.value)}>
              {memberProjects.map((p) => (
                <option key={p.id} value={p.id}>{p.title}</option>
              ))}
            </Select>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <label htmlFor="edit-role" className="text-sm font-medium">Role</label>
              <Select id="edit-role" value={role} onChange={(e) => setRole(e.target.value as Role)} disabled={loadingRow}>
                {roles.map((r) => (
                  <option key={r} value={r}>
                    {r === "worker" ? "Member" : r.charAt(0).toUpperCase() + r.slice(1)}
                  </option>
                ))}
              </Select>
            </div>
            <div className="space-y-1.5">
              <label htmlFor="edit-rate" className="text-sm font-medium">Hourly rate (USD)</label>
              <Input
                id="edit-rate"
                type="number"
                min={0}
                max={10000}
                step="0.01"
                value={rate}
                onChange={(e) => setRate(e.target.value)}
                placeholder="0"
                disabled={loadingRow}
              />
            </div>
          </div>
          {error && <p className="rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger">{error}</p>}
          <div className="flex justify-end gap-2 pt-2">
            <Button variant="ghost" onClick={onClose}>Cancel</Button>
            <Button onClick={save} disabled={saving || loadingRow || !projectId}>
              {saving ? "Saving…" : "Save changes"}
            </Button>
          </div>
        </div>
      )}
    </Modal>
  );
}
