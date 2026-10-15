"use client";

import { useEffect, useRef, useState } from "react";
import { Check, ChevronsUpDown, Plus, Building2, Loader2, LogOut } from "lucide-react";
import { Modal } from "@/components/ui/modal";
import { Input, Select } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { useSession } from "@/components/session-provider";
import { planById, planNameFor, statusLabel, type PlanId } from "@/lib/saas-data";
import { toast } from "@/components/toast";
import { cn } from "@/lib/utils";
import { createWorkspaceApi, getApi, getMyWorkspacesApi, leaveWorkspaceApi } from "@/hooks/useApi";
import { currentWorkspaceId, switchToWorkspace, WORKSPACE_CHOOSER_PATH } from "@/lib/live-session";
import { workspaceInitial, workspaceRoleLabel, type WorkspaceMembership } from "@/lib/sign-in";
import type { ApiPlanDto } from "@/lib/live-identity";

export function WorkspaceSwitcher({ collapsed }: { collapsed: boolean }) {
  // Live: one account, many workspaces; switching only changes X-Workspace-Id.
  const { workspace, workspaces, setWorkspaceById, createWorkspace, setUserById, isLive } = useSession();
  const [open, setOpen] = useState(false);
  const [mine, setMine] = useState<WorkspaceMembership[] | null>(null);
  const [mineError, setMineError] = useState(false);
  const [leaving, setLeaving] = useState(false);
  const [liveCreateOpen, setLiveCreateOpen] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [name, setName] = useState("");
  const [planId, setPlanId] = useState<PlanId>("starter");
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, []);


  function toggle() {
    const next = !open;
    setOpen(next);
    if (next && isLive && !mine) {
      setMineError(false);
      getMyWorkspacesApi().then(setMine, () => setMineError(true));
    }
  }

  const currentId = currentWorkspaceId();
  const current = mine?.find((w) => w.id.toLowerCase() === currentId?.toLowerCase()) ?? null;

  function pick(w: WorkspaceMembership) {
    if (w.id.toLowerCase() === currentId?.toLowerCase()) {
      setOpen(false);
      return;
    }
    switchToWorkspace(w.id);
  }

  function submitCreate(e: React.FormEvent) {
    e.preventDefault();
    if (!name.trim()) return;
    const ws = createWorkspace(name.trim(), planId);
    setUserById(`${ws.id}-owner`);
    setCreateOpen(false);
    setName("");
    toast({ title: "Workspace created", description: `${ws.name} is ready on the ${planById(ws.planId).name} plan.`, tone: "success" });
  }

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={toggle}
        aria-haspopup="menu"
        aria-expanded={open}
        className={cn(
          "flex h-12 w-full items-center gap-2.5 rounded-xl px-2 transition-colors hover:bg-muted",
          collapsed && "justify-center px-0"
        )}
        title={collapsed ? workspace.name : undefined}
      >
        <span
          className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg text-sm font-bold text-white"
          style={{ background: workspace.color }}
        >
          {workspace.name.charAt(0)}
        </span>
        {!collapsed && (
          <>
            <span className="min-w-0 flex-1 text-left">
              <span className="block truncate text-sm font-semibold">{workspace.name}</span>
              <span className="block truncate text-[11px] text-muted-foreground">
                {planNameFor(workspace)} · {statusLabel[workspace.status]}
              </span>
            </span>
            <ChevronsUpDown className="h-4 w-4 shrink-0 text-muted-foreground" />
          </>
        )}
      </button>

      {open && (
        <div className="absolute left-0 right-0 z-50 mt-1 overflow-hidden rounded-xl border border-border bg-card p-1 card-elev-lg animate-scale-in">
          <div className="px-2 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            Workspaces
          </div>
          {isLive ? (
            <LiveWorkspaceList
              items={mine}
              failed={mineError}
              currentId={currentId}
              onPick={pick}
              onCreate={() => { setOpen(false); setLiveCreateOpen(true); }}
              onLeave={current ? () => { setOpen(false); setLeaving(true); } : undefined}
            />
          ) : workspaces.map((w) => {
            return (
              <button
                key={w.id}
                onClick={() => { setWorkspaceById(w.id); setOpen(false); }}
                className="flex w-full items-center gap-2.5 rounded-lg px-2 py-2 text-left hover:bg-muted"
              >
                <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-xs font-bold text-white" style={{ background: w.color }}>
                  {w.name.charAt(0)}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-medium">{w.name}</span>
                  <span className="block truncate text-[11px] text-muted-foreground">{planNameFor(w)} · {statusLabel[w.status]}</span>
                </span>
                {w.id === workspace.id && <Check className="h-4 w-4 shrink-0 text-primary" />}
              </button>
            );
          })}
          {!isLive && (
            <>
              <div className="my-1 h-px bg-border" />
              <button
                onClick={() => { setOpen(false); setCreateOpen(true); }}
                className="flex w-full items-center gap-2.5 rounded-lg px-2 py-2 text-sm hover:bg-muted"
              >
                <span className="flex h-7 w-7 items-center justify-center rounded-md border border-dashed border-border text-muted-foreground">
                  <Plus className="h-4 w-4" />
                </span>
                Create workspace
              </button>
            </>
          )}
        </div>
      )}

      <CreateWorkspaceModal open={liveCreateOpen} onClose={() => setLiveCreateOpen(false)} />
      {current && <LeaveWorkspaceDialog open={leaving} workspace={current} onClose={() => setLeaving(false)} />}

      <Modal open={createOpen} onClose={() => setCreateOpen(false)} title="Create a workspace">
        <form onSubmit={submitCreate} className="space-y-4">
          <div className="flex items-center gap-3 rounded-lg bg-muted/50 p-3 text-sm text-muted-foreground">
            <Building2 className="h-5 w-5 shrink-0" />
            Each workspace is a fully isolated tenant with its own members, projects, and billing.
          </div>
          <div className="space-y-1.5">
            <label className="text-sm font-medium">Workspace name</label>
            <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="Acme Corp" autoFocus />
            {name && (
              <p className="text-xs text-muted-foreground">
                URL: {name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/(^-|-$)/g, "") || "workspace"}.dositracker.app
              </p>
            )}
          </div>
          <div className="space-y-1.5">
            <label className="text-sm font-medium">Plan</label>
            <Select value={planId} onChange={(e) => setPlanId(e.target.value as PlanId)}>
              <option value="free">Free — $0</option>
              <option value="starter">Starter — $6 / user / mo</option>
              <option value="business">Business — $12 / user / mo</option>
            </Select>
            {planId !== "free" && <Badge tone="info">14-day free trial included</Badge>}
          </div>
          <div className="flex justify-end gap-2 pt-2">
            <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>Cancel</Button>
            <Button type="submit" disabled={!name.trim()}>Create workspace</Button>
          </div>
        </form>
      </Modal>
    </div>
  );
}

interface ErrorShape {
  name?: string;
  status?: number;
  code?: string | null;
  serverMessage?: string | null;
}

function errorFields(err: unknown): ErrorShape {
  return (err ?? {}) as ErrorShape;
}

function LiveWorkspaceList({
  items,
  failed,
  currentId,
  onPick,
  onCreate,
  onLeave,
}: {
  items: WorkspaceMembership[] | null;
  failed: boolean;
  currentId: string | null;
  onPick: (w: WorkspaceMembership) => void;
  onCreate: () => void;
  onLeave?: () => void;
}) {
  return (
    <>
      {failed ? (
        <p role="alert" className="px-2 py-2 text-xs text-muted-foreground">Couldn&apos;t load your workspaces.</p>
      ) : !items ? (
        <p role="status" className="flex items-center gap-2 px-2 py-2 text-xs text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading…
        </p>
      ) : (
        items.map((w) => {
          const isCurrent = w.id.toLowerCase() === currentId?.toLowerCase();
          return (
            <button
              key={w.id}
              onClick={() => onPick(w)}
              aria-current={isCurrent ? "true" : undefined}
              className="flex w-full items-center gap-2.5 rounded-lg px-2 py-2 text-left hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <span aria-hidden="true" className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md brand-gradient text-xs font-bold text-white">
                {workspaceInitial(w)}
              </span>
              <span className="min-w-0 flex-1 truncate text-sm font-medium">{w.name}</span>
              <Badge tone={w.role === "owner" ? "info" : "muted"}>{workspaceRoleLabel(w.role)}</Badge>
              {isCurrent && <Check className="h-4 w-4 shrink-0 text-primary" aria-label="Current workspace" />}
            </button>
          );
        })
      )}
      <div className="my-1 h-px bg-border" />
      <button
        onClick={onCreate}
        className="flex w-full items-center gap-2.5 rounded-lg px-2 py-2 text-sm hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <span className="flex h-7 w-7 items-center justify-center rounded-md border border-dashed border-border text-muted-foreground">
          <Plus className="h-4 w-4" />
        </span>
        Create workspace
      </button>
      {onLeave && (
        <button
          onClick={onLeave}
          className="flex w-full items-center gap-2.5 rounded-lg px-2 py-2 text-sm text-danger-on-tint hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <span className="flex h-7 w-7 items-center justify-center rounded-md">
            <LogOut className="h-4 w-4" aria-hidden="true" />
          </span>
          Leave workspace
        </button>
      )}
    </>
  );
}

function ErrorLine({ id, message }: { id: string; message: string | null }) {
  if (!message) return null;
  return (
    <p id={id} role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-sm font-medium text-danger-on-tint">
      {message}
    </p>
  );
}

const LAST_OWNER_CANNOT_LEAVE_MESSAGE = "You're the last owner of this workspace. Make someone else an owner before you leave.";

function LeaveWorkspaceDialog({ open, workspace, onClose }: { open: boolean; workspace: WorkspaceMembership; onClose: () => void }) {
  const [busy, setBusy] = useState(false);

  async function leave() {
    setBusy(true);
    try {
      await leaveWorkspaceApi(workspace.id);
    } catch (err) {
      const e = errorFields(err);
      toast({
        title: "Couldn't leave the workspace",
        description: e.code === "Tracker:LastOwnerCannotLeave" ? LAST_OWNER_CANNOT_LEAVE_MESSAGE : e.serverMessage || "Try again.",
        tone: "danger",
      });
      setBusy(false);
      onClose();
      return;
    }
    window.location.assign(WORKSPACE_CHOOSER_PATH);
  }

  return (
    <ConfirmDialog
      open={open}
      onCancel={onClose}
      onConfirm={() => void leave()}
      busy={busy}
      title={`Leave ${workspace.name}?`}
      description="You'll lose access to its projects and data. Your account stays, and you can be invited back."
      confirmLabel="Leave workspace"
      tone="danger"
    />
  );
}

function cheapestPlan(plans: ApiPlanDto[]): ApiPlanDto | undefined {
  return [...plans].sort((a, b) => (a.pricePerUser ?? 0) - (b.pricePerUser ?? 0))[0];
}

/** Name + plan, then POST /api/app/workspaces and switch to it. */
export function CreateWorkspaceForm({ onCancel, submitLabel = "Create workspace" }: { onCancel?: () => void; submitLabel?: string }) {
  const [name, setName] = useState("");
  const [plans, setPlans] = useState<ApiPlanDto[]>([]);
  const [planId, setPlanId] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    getApi("/api/app/workspace/available-plans").then(
      (json: unknown) => {
        if (!alive) return;
        const raw = Array.isArray(json) ? json : (json as { items?: unknown } | null)?.items;
        const list = Array.isArray(raw) ? (raw as ApiPlanDto[]) : [];
        setPlans(list);
        setPlanId((cur) => cur || cheapestPlan(list)?.id || "");
      },
      () => {}
    );
    return () => {
      alive = false;
    };
  }, []);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    const trimmed = name.trim();
    if (!trimmed) return;
    setBusy(true);
    setError(null);
    let created: { id: string; name: string };
    try {
      created = await createWorkspaceApi(trimmed, planId || null);
    } catch (err) {
      const e = errorFields(err);
      setError(
        e.code === "Tracker:DuplicateWorkspaceName"
          ? "A workspace with that name already exists. Pick another name."
          : e.serverMessage || "Couldn't create the workspace. Try again."
      );
      setBusy(false);
      return;
    }
    switchToWorkspace(created.id);
  }

  return (
    <form onSubmit={submit} className="space-y-4">
      <div className="space-y-1.5">
        <label htmlFor="create-ws-name" className="text-sm font-medium">Workspace name</label>
        <Input id="create-ws-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Acme Corp" autoFocus required />
      </div>
      {plans.length > 0 && (
        <div className="space-y-1.5">
          <label htmlFor="create-ws-plan" className="text-sm font-medium">Plan</label>
          <Select id="create-ws-plan" value={planId} onChange={(e) => setPlanId(e.target.value)}>
            {plans.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name ?? "Plan"} — {p.pricePerUser ? `$${p.pricePerUser} / user / mo` : "$0"}
              </option>
            ))}
          </Select>
        </div>
      )}
      <ErrorLine id="create-ws-error" message={error} />
      <div className="flex justify-end gap-2 pt-2">
        {onCancel && <Button type="button" variant="outline" onClick={onCancel}>Cancel</Button>}
        <Button type="submit" disabled={busy || !name.trim()}>
          {busy && <Loader2 className="h-4 w-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />}
          {busy ? "Creating…" : submitLabel}
        </Button>
      </div>
    </form>
  );
}

function CreateWorkspaceModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  return (
    <Modal open={open} onClose={onClose} title="Create a new workspace">
      <div className="mb-4 flex items-center gap-3 rounded-lg bg-muted/50 p-3 text-sm text-muted-foreground">
        <Building2 className="h-5 w-5 shrink-0" />
        Each workspace has its own members, projects and billing. Your account stays the same.
      </div>
      {open && <CreateWorkspaceForm onCancel={onClose} />}
    </Modal>
  );
}
