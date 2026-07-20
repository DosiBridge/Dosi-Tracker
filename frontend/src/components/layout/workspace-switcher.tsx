"use client";

import { useEffect, useRef, useState } from "react";
import { Check, ChevronsUpDown, Plus, Building2 } from "lucide-react";
import { Modal } from "@/components/ui/modal";
import { Input, Select } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { useSession } from "@/components/session-provider";
import { planById, statusLabel, type PlanId } from "@/lib/saas-data";
import { toast } from "@/components/toast";
import { cn } from "@/lib/utils";

export function WorkspaceSwitcher({ collapsed }: { collapsed: boolean }) {
  const { workspace, workspaces, setWorkspaceById, createWorkspace, setUserById } = useSession();
  const [open, setOpen] = useState(false);
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

  const plan = planById(workspace.planId);

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
        onClick={() => setOpen((o) => !o)}
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
                {plan.name} · {statusLabel[workspace.status]}
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
          {workspaces.map((w) => {
            const wp = planById(w.planId);
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
                  <span className="block truncate text-[11px] text-muted-foreground">{wp.name} · {statusLabel[w.status]}</span>
                </span>
                {w.id === workspace.id && <Check className="h-4 w-4 shrink-0 text-primary" />}
              </button>
            );
          })}
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
        </div>
      )}

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
