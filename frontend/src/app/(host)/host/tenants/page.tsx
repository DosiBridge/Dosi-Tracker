"use client";

import { useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import {
  Plus,
  Building2,
  LogIn,
  Ban,
  CheckCircle2,
  Trash2,
  Users,
  FolderKanban,
  DollarSign,
  Gauge,
  HardDrive,
  Settings2,
} from "lucide-react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { Modal } from "@/components/ui/modal";
import { Drawer } from "@/components/ui/drawer";
import { Avatar } from "@/components/ui/avatar";
import { Progress } from "@/components/ui/progress";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { SearchField, Toolbar } from "@/components/ui/toolbar";
import { DataTable, type Column } from "@/components/ui/data-table";
import { toast } from "@/components/toast";
import { useSession } from "@/components/session-provider";
import {
  ownerOf,
  planColor,
  tenantMetrics,
  type TenantMetrics,
} from "@/lib/host-data";
import {
  fmtLimit,
  invoicesFor,
  planById,
  plans,
  usagePct,
  workspaceUsage,
  type PlanId,
  type SubscriptionStatus,
  type Workspace,
} from "@/lib/saas-data";

const usd = (n: number | null) => (n === null ? "Custom" : `$${Math.round(n).toLocaleString()}`);

const statusTone: Record<SubscriptionStatus, "success" | "warning" | "danger"> = {
  active: "success",
  trialing: "warning",
  past_due: "danger",
};
const hostStatusLabel: Record<SubscriptionStatus, string> = {
  active: "Active",
  trialing: "Trial",
  past_due: "Suspended",
};

export default function HostTenantsPage() {
  const router = useRouter();
  const { workspaces, createWorkspace, updateWorkspace, deleteWorkspace, impersonate } = useSession();

  const [query, setQuery] = useState("");
  const [status, setStatus] = useState<"all" | SubscriptionStatus>("all");
  const [plan, setPlan] = useState<"all" | PlanId>("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [newName, setNewName] = useState("");
  const [newPlan, setNewPlan] = useState<PlanId>("starter");

  const rows = useMemo(() => {
    return workspaces
      .filter((w) => (status === "all" ? true : w.status === status))
      .filter((w) => (plan === "all" ? true : w.planId === plan))
      .filter((w) => {
        if (!query.trim()) return true;
        const q = query.toLowerCase();
        return w.name.toLowerCase().includes(q) || w.slug.toLowerCase().includes(q);
      })
      .map((w) => ({ ws: w, m: tenantMetrics(w) }));
  }, [workspaces, status, plan, query]);

  const selected = workspaces.find((w) => w.id === selectedId) ?? null;

  function doImpersonate(ws: Workspace) {
    impersonate(ws.id);
    toast({ title: `Entering ${ws.name}`, description: "You're now viewing this tenant as its owner.", tone: "info" });
    setTimeout(() => router.push("/dashboard"), 300);
  }

  function onCreate(e: React.FormEvent) {
    e.preventDefault();
    if (!newName.trim()) return;
    const ws = createWorkspace(newName.trim(), newPlan);
    setCreateOpen(false);
    setNewName("");
    toast({ title: "Tenant created", description: `${ws.name} is now on the ${planById(newPlan).name} plan.`, tone: "success" });
    setSelectedId(ws.id);
  }

  const columns: Column<{ ws: Workspace; m: TenantMetrics }>[] = [
    {
      key: "name",
      header: "Tenant",
      sortValue: (r) => r.ws.name.toLowerCase(),
      render: (r) => (
        <button onClick={() => setSelectedId(r.ws.id)} className="flex items-center gap-2.5 text-left">
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-xs font-bold text-white" style={{ background: r.ws.color }}>
            {r.ws.name.charAt(0)}
          </span>
          <span className="min-w-0">
            <span className="block truncate font-medium hover:text-primary">{r.ws.name}</span>
            <span className="block truncate text-xs text-muted-foreground">{r.ws.slug}.dositracker.app</span>
          </span>
        </button>
      ),
    },
    {
      key: "plan",
      header: "Plan",
      sortValue: (r) => r.ws.planId,
      render: (r) => (
        <span className="inline-flex items-center gap-1.5">
          <span className="h-2 w-2 rounded-full" style={{ background: planColor(r.ws.planId) }} />
          {planById(r.ws.planId).name}
        </span>
      ),
    },
    {
      key: "status",
      header: "Status",
      sortValue: (r) => r.ws.status,
      render: (r) => <Badge tone={statusTone[r.ws.status]}>{hostStatusLabel[r.ws.status]}</Badge>,
    },
    { key: "seats", header: "Seats", align: "right", sortValue: (r) => r.m.seats, render: (r) => r.m.seats.toLocaleString() },
    { key: "members", header: "Members", align: "right", sortValue: (r) => r.m.members, render: (r) => r.m.members },
    { key: "mrr", header: "MRR", align: "right", sortValue: (r) => r.m.mrr ?? 999999, render: (r) => <span className="font-medium">{usd(r.m.mrr)}</span> },
    { key: "created", header: "Created", align: "right", sortValue: (r) => r.ws.createdAt, render: (r) => <span className="text-muted-foreground">{r.ws.createdAt}</span> },
    {
      key: "actions",
      header: "",
      align: "right",
      render: (r) => (
        <div className="flex items-center justify-end gap-1">
          <button
            onClick={() => doImpersonate(r.ws)}
            title="Login as tenant"
            className="inline-flex h-8 w-8 items-center justify-center rounded-lg border border-border text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <LogIn className="h-4 w-4" />
          </button>
          <button
            onClick={() => setSelectedId(r.ws.id)}
            title="Manage"
            className="inline-flex h-8 w-8 items-center justify-center rounded-lg border border-border text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <Settings2 className="h-4 w-4" />
          </button>
        </div>
      ),
    },
  ];

  return (
    <PageStack>
      <PageHeader
        eyebrow="Platform"
        title="Tenants"
        description={`${rows.length} of ${workspaces.length} workspaces on the platform.`}
        actions={
          <Button onClick={() => setCreateOpen(true)}>
            <Plus className="h-4 w-4" /> New tenant
          </Button>
        }
      />

      <Toolbar>
        <SearchField value={query} onChange={setQuery} placeholder="Search tenant name or slug…" />
        <Select value={status} onChange={(e) => setStatus(e.target.value as typeof status)} className="w-full sm:w-auto sm:min-w-36">
          <option value="all">All statuses</option>
          <option value="active">Active</option>
          <option value="trialing">Trial</option>
          <option value="past_due">Suspended</option>
        </Select>
        <Select value={plan} onChange={(e) => setPlan(e.target.value as typeof plan)} className="w-full sm:w-auto sm:min-w-36">
          <option value="all">All plans</option>
          {plans.map((p) => (
            <option key={p.id} value={p.id}>{p.name}</option>
          ))}
        </Select>
      </Toolbar>

      <Card className="p-2">
        <DataTable columns={columns} rows={rows} initialSort={{ key: "mrr", dir: "desc" }} emptyText="No tenants match your filters." />
      </Card>

      {/* Create modal */}
      <Modal open={createOpen} onClose={() => setCreateOpen(false)} title="Create a tenant" description="Provision a new isolated workspace.">
        <form onSubmit={onCreate} className="space-y-4">
          <div className="space-y-1.5">
            <label className="text-sm font-medium">Workspace name</label>
            <Input value={newName} onChange={(e) => setNewName(e.target.value)} placeholder="Acme Corp" autoFocus required />
          </div>
          <div className="space-y-1.5">
            <label className="text-sm font-medium">Plan</label>
            <Select value={newPlan} onChange={(e) => setNewPlan(e.target.value as PlanId)}>
              {plans.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name} — {p.pricePerUser === null ? "Custom" : `$${p.pricePerUser}/user/mo`}
                </option>
              ))}
            </Select>
          </div>
          <div className="flex justify-end gap-2 pt-2">
            <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>Cancel</Button>
            <Button type="submit" disabled={!newName.trim()}>Create tenant</Button>
          </div>
        </form>
      </Modal>

      {/* Detail drawer */}
      <TenantDrawer
        ws={selected}
        onClose={() => setSelectedId(null)}
        onImpersonate={doImpersonate}
        onUpdate={updateWorkspace}
        onDelete={(id) => {
          deleteWorkspace(id);
          setSelectedId(null);
          toast({ title: "Tenant deleted", tone: "danger" });
        }}
      />
    </PageStack>
  );
}

function TenantDrawer({
  ws,
  onClose,
  onImpersonate,
  onUpdate,
  onDelete,
}: {
  ws: Workspace | null;
  onClose: () => void;
  onImpersonate: (ws: Workspace) => void;
  onUpdate: (id: string, patch: Partial<Workspace>) => void;
  onDelete: (id: string) => void;
}) {
  const [confirmDelete, setConfirmDelete] = useState(false);

  if (!ws) return null;
  const m = tenantMetrics(ws);
  const owner = ownerOf(ws);
  const usage = workspaceUsage(ws);
  const invoices = invoicesFor(ws).slice(0, 4);
  const suspended = ws.status === "past_due";

  return (
    <Drawer open={!!ws} onClose={onClose} title="Tenant details">
      <div className="space-y-6">
        {/* Header */}
        <div className="flex items-center gap-3">
          <span className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl text-lg font-bold text-white" style={{ background: ws.color }}>
            {ws.name.charAt(0)}
          </span>
          <div className="min-w-0 flex-1">
            <div className="truncate text-lg font-semibold">{ws.name}</div>
            <div className="truncate text-xs text-muted-foreground">{ws.slug}.dositracker.app · since {ws.createdAt}</div>
          </div>
          <Badge tone={statusTone[ws.status]}>{hostStatusLabel[ws.status]}</Badge>
        </div>

        {/* Quick actions */}
        <div className="grid grid-cols-2 gap-2">
          <Button onClick={() => onImpersonate(ws)} className="w-full">
            <LogIn className="h-4 w-4" /> Login as tenant
          </Button>
          {suspended ? (
            <Button variant="outline" className="w-full" onClick={() => onUpdate(ws.id, { status: "active" })}>
              <CheckCircle2 className="h-4 w-4" /> Reactivate
            </Button>
          ) : (
            <Button variant="outline" className="w-full" onClick={() => onUpdate(ws.id, { status: "past_due" })}>
              <Ban className="h-4 w-4" /> Suspend
            </Button>
          )}
        </div>

        {/* Metrics */}
        <div className="grid grid-cols-2 gap-3">
          <MiniMetric icon={Users} label="Members" value={String(m.members)} tone="#0ea5e9" />
          <MiniMetric icon={FolderKanban} label="Projects" value={String(m.projects)} tone="#6d5efc" />
          <MiniMetric icon={DollarSign} label="MRR" value={usd(m.mrr)} tone="#22c55e" />
          <MiniMetric icon={Gauge} label="Avg productivity" value={`${m.avgProductivity}%`} tone="#ec4899" />
          <MiniMetric icon={HardDrive} label="Storage" value={`${m.storageGb} GB`} tone="#8b5cf6" />
          <MiniMetric icon={Building2} label="Seats" value={String(m.seats)} tone="#f59e0b" />
        </div>

        {/* Owner */}
        {owner && (
          <div>
            <div className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">Owner</div>
            <div className="flex items-center gap-3 rounded-xl border border-border p-3">
              <Avatar name={owner.name} size="md" status={owner.status} />
              <div className="min-w-0">
                <div className="truncate text-sm font-medium">{owner.name}</div>
                <div className="truncate text-xs text-muted-foreground">{owner.email}</div>
              </div>
            </div>
          </div>
        )}

        {/* Plan */}
        <div>
          <div className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">Subscription plan</div>
          <Select value={ws.planId} onChange={(e) => onUpdate(ws.id, { planId: e.target.value as PlanId })}>
            {plans.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name} — {p.pricePerUser === null ? "Custom" : `$${p.pricePerUser}/user/mo`}
              </option>
            ))}
          </Select>
          <div className="mt-3 space-y-2.5">
            {usage.map((u) => (
              <div key={u.label}>
                <div className="mb-1 flex items-center justify-between text-xs">
                  <span className="text-muted-foreground">{u.label}</span>
                  <span className="font-medium">{u.used.toLocaleString()} / {fmtLimit(u.limit)} {u.unit}</span>
                </div>
                <Progress value={usagePct(u.used, u.limit)} />
              </div>
            ))}
          </div>
        </div>

        {/* Invoices */}
        <div>
          <div className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">Recent invoices</div>
          <div className="space-y-1.5">
            {invoices.map((inv) => (
              <div key={inv.id} className="flex items-center justify-between rounded-lg bg-muted/50 px-3 py-2 text-sm">
                <span className="text-muted-foreground">{inv.date}</span>
                <span className="flex items-center gap-2">
                  <span className="font-medium">{usd(inv.amount)}</span>
                  <Badge tone={inv.status === "paid" ? "success" : inv.status === "due" ? "warning" : "muted"}>{inv.status}</Badge>
                </span>
              </div>
            ))}
          </div>
        </div>

        {/* Danger zone */}
        <div className="rounded-xl border border-danger/30 bg-danger/5 p-3">
          <div className="text-sm font-medium text-danger">Danger zone</div>
          <p className="mt-0.5 text-xs text-muted-foreground">Deleting a tenant removes all of its data. This cannot be undone.</p>
          {confirmDelete ? (
            <div className="mt-3 flex gap-2">
              <Button variant="outline" size="sm" onClick={() => setConfirmDelete(false)}>Cancel</Button>
              <Button size="sm" className="bg-danger text-white hover:opacity-90" onClick={() => onDelete(ws.id)}>
                <Trash2 className="h-4 w-4" /> Yes, delete {ws.name}
              </Button>
            </div>
          ) : (
            <Button variant="outline" size="sm" className="mt-3 border-danger/40 text-danger hover:bg-danger/10" onClick={() => setConfirmDelete(true)}>
              <Trash2 className="h-4 w-4" /> Delete tenant
            </Button>
          )}
        </div>
      </div>
    </Drawer>
  );
}

function MiniMetric({ icon: Icon, label, value, tone }: { icon: typeof Users; label: string; value: string; tone: string }) {
  return (
    <div className="rounded-xl border border-border p-3">
      <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <Icon className="h-3.5 w-3.5" style={{ color: tone }} /> {label}
      </div>
      <div className="mt-1 text-base font-bold">{value}</div>
    </div>
  );
}
