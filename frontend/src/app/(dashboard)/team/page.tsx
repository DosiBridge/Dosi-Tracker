"use client";

import { useEffect, useMemo, useState } from "react";
import { Plus, Mail, Clock, FolderKanban, Send, Users, Copy, Check } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
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
import { projects, users } from "@/lib/tenant-data";
import { useSession } from "@/components/session-provider";
import { countSeats, inviteableRoles } from "@/lib/roles";
import { fmtLimit, planById, usagePct } from "@/lib/saas-data";
import type { Role, UserStatus } from "@/lib/types";
import { formatDuration } from "@/lib/utils";
import { getApi, postApi } from "@/hooks/useApi";
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
}

interface IdentityUserDto {
  id: string;
  userName?: string;
  name?: string;
  surname?: string;
  email?: string;
  isActive?: boolean;
}

export default function TeamPage() {
  const { user, workspace } = useSession();
  const [query, setQuery] = useState("");
  const [role, setRole] = useState<Role | "all">("all");
  const [invite, setInvite] = useState(false);

  // LIVE MODE: real roster via the identity API for owner/host when a backend
  // token is present. Any failure (e.g. 403 for non-admins) keeps the demo data.
  const [hasToken, setHasToken] = useState(false);
  const [liveMembers, setLiveMembers] = useState<MemberView[] | null>(null);
  const [liveLoading, setLiveLoading] = useState(false);
  const [liveError, setLiveError] = useState<string | null>(null);

  useEffect(() => {
    if (typeof window !== "undefined" && localStorage.getItem("dosi-token")) {
      setHasToken(true);
    }
  }, []);

  useEffect(() => {
    if (!hasToken || (user.role !== "owner" && user.role !== "host")) return;
    let cancelled = false;
    setLiveLoading(true);
    setLiveError(null);
    getApi("/api/identity/users?MaxResultCount=100")
      .then((items) => {
        if (cancelled) return;
        const list = (Array.isArray(items) ? items : []) as IdentityUserDto[];
        setLiveMembers(
          list.map((u): MemberView => {
            const memberRole: Role = u.id === user.id ? user.role : "worker";
            return {
              id: u.id,
              name: [u.name, u.surname].filter(Boolean).join(" ") || u.userName || u.email || "Member",
              designation:
                memberRole === "host" ? "Host · Super Admin" : memberRole === "owner" ? "Workspace Owner" : "Member",
              email: u.email ?? "",
              role: memberRole,
              status: u.isActive === false ? "offline" : "active",
              trackedToday: 0,
              productivity: 0,
            };
          })
        );
      })
      .catch(() => {
        if (!cancelled) {
          setLiveMembers(null); // fall back to the demo roster below
          setLiveError("Live roster unavailable — showing demo data.");
        }
      })
      .finally(() => {
        if (!cancelled) setLiveLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [hasToken, user.id, user.role]);

  const members: MemberView[] = liveMembers ?? users;

  const plan = planById(workspace.planId);
  const seatsUsed = countSeats(members);
  const seatPct = usagePct(seatsUsed, plan.seats);
  const seatsFull = plan.seats !== Number.POSITIVE_INFINITY && seatsUsed >= plan.seats;
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

  const projectCount = (userId: string) => projects.filter((p) => p.memberIds.includes(userId)).length;

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
              Seats · <span className="text-muted-foreground">{plan.name} plan</span>
            </div>
            <div className="text-xs text-muted-foreground">
              {seatsUsed} of {fmtLimit(plan.seats)} seats used
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

      {liveError && <p className="text-xs text-muted-foreground">{liveError}</p>}

      {liveLoading && !liveMembers ? (
        <p className="py-10 text-center text-sm text-muted-foreground">Loading team…</p>
      ) : filtered.length === 0 ? (
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
          <Card key={u.id} variant="interactive" className="p-5">
            <div className="flex items-start justify-between">
              <div className="flex items-center gap-3">
                <Avatar name={u.name} size="lg" status={u.status} />
                <div>
                  <h2 className="font-semibold leading-tight">{u.name}</h2>
                  <p className="text-xs text-muted-foreground">{u.designation}</p>
                </div>
              </div>
              <Badge tone={roleTone[u.role]} className="capitalize">{u.role}</Badge>
            </div>

            <div className="mt-4 flex items-center gap-1.5 text-xs text-muted-foreground">
              <Mail className="h-3.5 w-3.5" />
              <span className="truncate">{u.email}</span>
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
                  <span className="font-medium">{projectCount(u.id)}</span>
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

      <InviteModal open={invite} onClose={() => setInvite(false)} inviterRole={user.role} live={hasToken} />
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
}: {
  open: boolean;
  onClose: () => void;
  inviterRole: Role;
  live: boolean;
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
        if (!cancelled) setApiProjects(null); // fall back to the demo project list
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
              <label className="text-sm font-medium">
                Hourly rate <span className="font-normal text-muted-foreground">(optional)</span>
              </label>
              <Input
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
