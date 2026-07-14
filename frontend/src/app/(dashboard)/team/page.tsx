"use client";

import { useMemo, useState } from "react";
import { Plus, Search, Mail, Clock, FolderKanban, Send } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Ring } from "@/components/ui/ring";
import { Modal } from "@/components/ui/modal";
import { Input, Select } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { projects, users } from "@/lib/tenant-data";
import { useSession } from "@/components/session-provider";
import { fmtLimit, planById, usagePct } from "@/lib/saas-data";
import type { Role } from "@/lib/types";
import { cn, formatDuration } from "@/lib/utils";
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

export default function TeamPage() {
  const { user, workspace } = useSession();
  const [query, setQuery] = useState("");
  const [role, setRole] = useState<Role | "all">("all");
  const [invite, setInvite] = useState(false);

  const plan = planById(workspace.planId);
  const seatsUsed = users.filter((u) => u.role !== "client").length;
  const seatPct = usagePct(seatsUsed, plan.seats);
  const seatsFull = plan.seats !== Number.POSITIVE_INFINITY && seatsUsed >= plan.seats;
  const canManage = user.role === "owner" || user.role === "admin";

  const filtered = useMemo(
    () =>
      users.filter(
        (u) =>
          (role === "all" || u.role === role) &&
          (u.name.toLowerCase().includes(query.toLowerCase()) ||
            u.email.toLowerCase().includes(query.toLowerCase()))
      ),
    [query, role]
  );

  const projectCount = (userId: string) => projects.filter((p) => p.memberIds.includes(userId)).length;

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Team</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {users.filter((u) => u.role !== "client").length} members · {users.filter((u) => u.status === "active").length} active now
          </p>
        </div>
        {canManage && (
          <Button onClick={() => setInvite(true)} disabled={seatsFull} title={seatsFull ? "Seat limit reached — upgrade your plan" : undefined}>
            <Plus className="h-4 w-4" /> Invite member
          </Button>
        )}
      </div>

      {/* Seats (plan usage) */}
      <Card className="p-4">
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
          <Progress value={seatPct} color={seatsFull ? "#ef4444" : seatPct >= 80 ? "#f59e0b" : undefined} />
        </div>
      </Card>

      {/* Controls */}
      <div className="flex flex-wrap items-center gap-3">
        <div className="relative min-w-0 flex-1">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search members…"
            className="h-10 w-full rounded-lg border border-border bg-card pl-9 pr-3 text-sm focus:outline-none focus:ring-2 focus:ring-ring"
          />
        </div>
        <div className="flex flex-wrap gap-1 rounded-lg border border-border bg-card p-1">
          {filters.map((f) => (
            <button
              key={f.key}
              onClick={() => setRole(f.key)}
              className={cn(
                "rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
                role === f.key ? "bg-primary text-primary-foreground" : "text-muted-foreground hover:text-foreground"
              )}
            >
              {f.label}
            </button>
          ))}
        </div>
      </div>

      {/* Grid */}
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">
        {filtered.map((u) => (
          <Card key={u.id} className="p-5">
            <div className="flex items-start justify-between">
              <div className="flex items-center gap-3">
                <Avatar name={u.name} size="lg" status={u.status} />
                <div>
                  <h3 className="font-semibold leading-tight">{u.name}</h3>
                  <p className="text-xs text-muted-foreground">{u.designation}</p>
                </div>
              </div>
              <Badge tone={roleTone[u.role]} className="capitalize">{u.role}</Badge>
            </div>

            <div className="mt-4 flex items-center gap-1.5 text-xs text-muted-foreground">
              <Mail className="h-3.5 w-3.5" />
              <span className="truncate">{u.email}</span>
            </div>

            <div className="mt-4 flex items-center justify-between rounded-lg bg-muted/50 p-3">
              <div className="space-y-2">
                <div className="flex items-center gap-1.5 text-sm">
                  <Clock className="h-4 w-4 text-muted-foreground" />
                  <span className="font-medium">{u.role === "client" ? "—" : formatDuration(u.trackedToday)}</span>
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

      {/* Invite modal */}
      <InviteModal open={invite} onClose={() => setInvite(false)} />
    </div>
  );
}

function InviteModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [emails, setEmails] = useState("");
  const [role, setRole] = useState<Role>("worker");
  const [project, setProject] = useState(projects[0]?.id ?? "");
  const [sent, setSent] = useState(false);

  function send() {
    setSent(true);
    setTimeout(() => {
      setSent(false);
      setEmails("");
      onClose();
    }, 1200);
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
      ) : (
        <div className="space-y-4">
          <div className="space-y-1.5">
            <label className="text-sm font-medium">Email addresses</label>
            <Input value={emails} onChange={(e) => setEmails(e.target.value)} placeholder="jane@company.com, john@company.com" />
            <p className="text-xs text-muted-foreground">Separate multiple emails with commas.</p>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <label className="text-sm font-medium">Role</label>
              <Select value={role} onChange={(e) => setRole(e.target.value as Role)}>
                <option value="worker">Worker</option>
                <option value="owner">Owner</option>
                <option value="client">Client</option>
                <option value="admin">Admin</option>
              </Select>
            </div>
            <div className="space-y-1.5">
              <label className="text-sm font-medium">Project</label>
              <Select value={project} onChange={(e) => setProject(e.target.value)}>
                {projects.filter((p) => !p.archived).map((p) => (
                  <option key={p.id} value={p.id}>{p.title}</option>
                ))}
              </Select>
            </div>
          </div>
          <div className="flex justify-end gap-2 pt-2">
            <Button variant="ghost" onClick={onClose}>Cancel</Button>
            <Button onClick={send} disabled={!emails.trim()}>
              <Send className="h-4 w-4" /> Send invites
            </Button>
          </div>
        </div>
      )}
    </Modal>
  );
}
