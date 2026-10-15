"use client";

import { useCallback, useEffect, useId, useMemo, useState } from "react";
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
  Repeat,
  BadgeCheck,
} from "lucide-react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { Modal } from "@/components/ui/modal";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Drawer } from "@/components/ui/drawer";
import { Avatar } from "@/components/ui/avatar";
import { Progress } from "@/components/ui/progress";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { SearchField, Toolbar } from "@/components/ui/toolbar";
import { DataTable, type Column } from "@/components/ui/data-table";
import { requestErrorText } from "@/components/layout/request-error";
import { toast } from "@/components/toast";
import { useSession } from "@/components/session-provider";
import {
  activateSubscription,
  changeWorkspacePlan,
  getApi,
  getWorkspaceSubscriptions,
  markInvoicePaid,
  suspendSubscription,
} from "@/hooks/useApi";
import { ownerOf, planColor, tenantMetrics, type TenantMetrics } from "@/lib/host-data";
import { asItems } from "@/lib/live-dataset";
import { subscriptionStatus } from "@/lib/live-identity";
import type { PlanDto, PlatformInvoiceDto, WorkspaceSubscriptionDto } from "@/lib/api-contract";
import {
  fmtLimit,
  invoicesFor,
  liveSeatsUsed,
  planById,
  plans,
  statusLabel,
  subscriptionNeedsAttention,
  usagePct,
  workspaceUsage,
  type PlanId,
  type SubscriptionStatus,
  type Workspace,
} from "@/lib/saas-data";
import { colorFromString } from "@/lib/utils";

const usd = (n: number | null) => (n === null ? "Custom" : `$${Math.round(n).toLocaleString()}`);

/** A live workspace as the platform lists it (from workspace-subscriptions). */
interface LiveTenant {
  id: string;
  name: string;
}

const LIVE_COLORS = ["#6d5efc", "#0ea5e9", "#ec4899", "#22c55e", "#f59e0b", "#8b5cf6"];

const slugify = (name: string) =>
  name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "") || "tenant";

/** Adapt a live ABP tenant to the Workspace shape the existing UI renders (plan and status are not known per tenant). */
const toWorkspace = (t: LiveTenant, i: number): Workspace => ({
  id: t.id,
  name: t.name,
  slug: slugify(t.name),
  planId: "free",
  status: "active",
  color: LIVE_COLORS[i % LIVE_COLORS.length],
  createdAt: "",
  cycleDays: 0,
  seatsUsed: 0,
  projectsUsed: 0,
  storageUsedGb: 0,
});

/** A table row; `sub` is the live tenant's subscription when the server reports it. */
type Row = { ws: Workspace; m: TenantMetrics; live: boolean; sub: WorkspaceSubscriptionDto | null };

const statusTone: Record<SubscriptionStatus, "success" | "warning" | "danger"> = {
  active: "success",
  trialing: "warning",
  past_due: "danger",
  suspended: "danger",
  canceled: "danger",
  expired: "danger",
};
/** Demo wording: the demo's "Suspend" puts a workspace past due. Live statuses use statusLabel. */
const hostStatusLabel: Record<SubscriptionStatus, string> = {
  active: "Active",
  trialing: "Trial",
  past_due: "Suspended",
  suspended: "Suspended",
  canceled: "Canceled",
  expired: "Expired",
};

/** The host's plans, invoices and tenant subscriptions (live), shared by the table and the drawer. */
interface Billing {
  plans: PlanDto[] | null;
  invoices: PlatformInvoiceDto[] | null;
  /** By tenant id; null when unknown (the call failed, or an older server doesn't offer it). */
  subscriptions: Map<string, WorkspaceSubscriptionDto> | null;
}

/** A live tenant's subscription status, or null when it has none (or none is reported). */
function liveStatusOf(sub: WorkspaceSubscriptionDto | null | undefined): SubscriptionStatus | null {
  return sub?.status ? subscriptionStatus(sub.status) : null;
}

export default function HostTenantsPage() {
  const router = useRouter();
  const { workspaces, createWorkspace, updateWorkspace, deleteWorkspace, impersonate, isLive: liveSession } = useSession();
  const formId = useId();

  const [query, setQuery] = useState("");
  const [status, setStatus] = useState<"all" | SubscriptionStatus>("all");
  const [plan, setPlan] = useState<"all" | PlanId>("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [newName, setNewName] = useState("");
  const [newPlan, setNewPlan] = useState<PlanId>("starter");

  // Live mode: real tenants from the backend when a session token exists.
  // null = mock mode (no token, still loading, or the tenants call failed/403'd).
  const [liveTenants, setLiveTenants] = useState<LiveTenant[] | null>(null);
  const [liveLoading, setLiveLoading] = useState(false);
  const [liveFailed, setLiveFailed] = useState(false);
  const [billing, setBilling] = useState<Billing>({ plans: null, invoices: null, subscriptions: null });

  const fetchTenants = useCallback(async () => {
    setLiveLoading(true);
    try {
      const items = (await getWorkspaceSubscriptions()) ?? [];
      setLiveTenants(items.map((w) => ({ id: w.workspaceId, name: w.workspaceName })));
      setLiveFailed(false);
    } catch {
      // Live: show an error with retry — never the demo tenants.
      setLiveTenants(null);
      setLiveFailed(true);
    } finally {
      setLiveLoading(false);
    }
  }, []);

  const fetchBilling = useCallback(async () => {
    const [plansRes, invoicesRes, subscriptionsRes] = await Promise.allSettled([
      getApi("/api/app/platform/plans"),
      getApi("/api/app/platform/invoices?MaxResultCount=1000"),
      getWorkspaceSubscriptions(),
    ]);
    setBilling({
      plans: plansRes.status === "fulfilled" ? asItems<PlanDto>(plansRes.value) : null,
      invoices: invoicesRes.status === "fulfilled" ? asItems<PlatformInvoiceDto>(invoicesRes.value) : null,
      subscriptions:
        subscriptionsRes.status === "fulfilled" && subscriptionsRes.value
          ? new Map(subscriptionsRes.value.map((s) => [s.workspaceId, s]))
          : null,
    });
  }, []);

  useEffect(() => {
    if (liveSession) {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
      void fetchTenants();
      void fetchBilling();
    }
  }, [fetchTenants, fetchBilling, liveSession]);

  const isLive = liveSession;
  // liveLoading only ever turns on when a token exists, so this stays false in demo mode.
  const initialLiveLoading = liveLoading && liveTenants === null && !liveFailed;

  const liveWorkspaces = useMemo<Workspace[]>(() => (liveTenants ?? []).map(toWorkspace), [liveTenants]);

  const allRows = isLive ? liveWorkspaces : workspaces;

  const rows = useMemo<Row[]>(() => {
    const source = isLive ? liveWorkspaces : workspaces;
    return source
      .filter((w) => (isLive || status === "all" ? true : w.status === status))
      .filter((w) => (isLive || plan === "all" ? true : w.planId === plan))
      .filter((w) => {
        if (!query.trim()) return true;
        const q = query.toLowerCase();
        return w.name.toLowerCase().includes(q) || w.slug.toLowerCase().includes(q);
      })
      .map((w) => ({ ws: w, m: tenantMetrics(w), live: isLive, sub: isLive ? (billing.subscriptions?.get(w.id) ?? null) : null }));
  }, [isLive, liveWorkspaces, workspaces, status, plan, query, billing.subscriptions]);

  const selected = allRows.find((w) => w.id === selectedId) ?? null;

  function doImpersonate(ws: Workspace) {
    impersonate(ws.id);
    toast({ title: `Entering ${ws.name}`, description: "You're now viewing this tenant as its owner.", tone: "info" });
    setTimeout(() => router.push("/dashboard"), 300);
  }

  async function onCreate(e: React.FormEvent) {
    e.preventDefault();
    if (!newName.trim()) return;
    if (isLive) return; // workspaces are created by their owners
    const ws = createWorkspace(newName.trim(), newPlan);
    setCreateOpen(false);
    setNewName("");
    toast({ title: "Tenant created", description: `${ws.name} is now on the ${planById(newPlan).name} plan.`, tone: "success" });
    setSelectedId(ws.id);
  }

  const dash = <span className="text-muted-foreground">—</span>;
  const columns: Column<Row>[] = [
    {
      key: "name",
      header: "Tenant",
      sortValue: (r) => r.ws.name.toLowerCase(),
      render: (r) => (
        <button onClick={() => setSelectedId(r.ws.id)} className="flex items-center gap-2.5 rounded-lg text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
          <span aria-hidden className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-xs font-bold text-white" style={{ background: r.ws.color }}>
            {r.ws.name.charAt(0)}
          </span>
          <span className="min-w-0">
            <span className="block truncate font-medium hover:text-primary-strong">{r.ws.name}</span>
            <span className="block truncate text-xs text-muted-foreground">{r.live ? `Sign-in name: ${r.ws.name}` : `${r.ws.slug}.dositracker.app`}</span>
          </span>
        </button>
      ),
    },
    {
      key: "plan",
      header: "Plan",
      sortValue: (r) => (r.live ? (r.sub?.planName ?? "").toLowerCase() : r.ws.planId),
      render: (r) =>
        r.live ? (
          r.sub?.planName ? (
            <span className="inline-flex items-center gap-1.5">
              <span aria-hidden className="h-2 w-2 rounded-full" style={{ background: colorFromString(r.sub.planName) }} />
              {r.sub.planName}
            </span>
          ) : (
            dash
          )
        ) : (
          <span className="inline-flex items-center gap-1.5">
            <span aria-hidden className="h-2 w-2 rounded-full" style={{ background: planColor(r.ws.planId) }} />
            {planById(r.ws.planId).name}
          </span>
        ),
    },
    {
      key: "status",
      header: "Status",
      sortValue: (r) => (r.live ? (liveStatusOf(r.sub) ?? "") : r.ws.status),
      // A live tenant without a reported subscription gets no badge rather than a guessed "Active".
      render: (r) => {
        if (!r.live) return <Badge tone={statusTone[r.ws.status]}>{hostStatusLabel[r.ws.status]}</Badge>;
        const status = liveStatusOf(r.sub);
        return status ? <Badge tone={statusTone[status]}>{statusLabel[status]}</Badge> : dash;
      },
    },
    {
      key: "seats",
      header: "Seats",
      align: "right",
      sortValue: (r) => (r.live ? (r.sub?.seatsUsed ?? -1) : r.m.seats),
      render: (r) =>
        r.live ? (
          r.sub ? (
            <span className="tabular-nums">
              {r.sub.seatsUsed.toLocaleString("en-US")}
              {typeof r.sub.maxSeats === "number" && <span className="text-muted-foreground"> / {r.sub.maxSeats.toLocaleString("en-US")}</span>}
            </span>
          ) : (
            dash
          )
        ) : (
          r.m.seats.toLocaleString()
        ),
    },
    { key: "members", header: "Members", align: "right", sortValue: (r) => r.m.members, render: (r) => (r.live ? dash : r.m.members) },
    { key: "mrr", header: "MRR", align: "right", sortValue: (r) => r.m.mrr ?? 999999, render: (r) => (r.live ? dash : <span className="font-medium">{usd(r.m.mrr)}</span>) },
    { key: "created", header: "Created", align: "right", sortValue: (r) => r.ws.createdAt, render: (r) => <span className="text-muted-foreground">{r.live ? "—" : r.ws.createdAt}</span> },
    {
      key: "actions",
      header: "",
      align: "right",
      render: (r) => (
        <div className="flex items-center justify-end gap-1">
          {/* Signing in as a tenant exists in the demo only: the platform API has no impersonation. */}
          {!r.live && (
            <button
              onClick={() => doImpersonate(r.ws)}
              title="Login as tenant"
              aria-label={`Log in as ${r.ws.name}`}
              className="inline-flex h-8 w-8 items-center justify-center rounded-lg border border-border text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <LogIn aria-hidden className="h-4 w-4" />
            </button>
          )}
          <button
            onClick={() => setSelectedId(r.ws.id)}
            title="Manage"
            aria-label={`Manage ${r.ws.name}`}
            className="inline-flex h-8 w-8 items-center justify-center rounded-lg border border-border text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <Settings2 aria-hidden className="h-4 w-4" />
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
        description={initialLiveLoading ? "Loading tenants…" : `${rows.length} of ${allRows.length} workspaces on the platform.`}
        actions={
          isLive ? undefined : (
            <Button onClick={() => setCreateOpen(true)}>
              <Plus className="h-4 w-4" /> New tenant
            </Button>
          )
        }
      />

      {liveFailed && (
        <p role="alert" className="flex items-center gap-2 text-xs text-danger-on-tint">
          Couldn&apos;t load tenants from the server.
          <button onClick={() => void fetchTenants()} className="font-medium text-primary-strong hover:underline">
            Try again
          </button>
        </p>
      )}

      <Toolbar>
        <SearchField value={query} onChange={setQuery} placeholder="Search tenant name or slug…" />
        {!isLive && (
          <>
            <Select aria-label="Status" value={status} onChange={(e) => setStatus(e.target.value as typeof status)} className="w-full sm:w-auto sm:min-w-36">
              <option value="all">All statuses</option>
              <option value="active">Active</option>
              <option value="trialing">Trial</option>
              <option value="past_due">Suspended</option>
            </Select>
            <Select aria-label="Plan" value={plan} onChange={(e) => setPlan(e.target.value as typeof plan)} className="w-full sm:w-auto sm:min-w-36">
              <option value="all">All plans</option>
              {plans.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </Select>
          </>
        )}
      </Toolbar>

      <Card className="p-2">
        {initialLiveLoading ? (
          <p role="status" className="py-10 text-center text-sm text-muted-foreground">
            Loading tenants…
          </p>
        ) : (
          <DataTable columns={columns} rows={rows} initialSort={{ key: isLive ? "name" : "mrr", dir: isLive ? "asc" : "desc" }} emptyText="No tenants match your filters." />
        )}
      </Card>

      {/* Create modal */}
      <Modal open={createOpen} onClose={() => setCreateOpen(false)} title="Create a tenant" description="Provision a new isolated workspace.">
        <form onSubmit={onCreate} className="space-y-4">
          <div className="space-y-1.5">
            <label htmlFor={`${formId}-name`} className="text-sm font-medium">
              Workspace name
            </label>
            <Input id={`${formId}-name`} value={newName} onChange={(e) => setNewName(e.target.value)} placeholder="Acme Corp" autoFocus required />
          </div>
          {(
            <div className="space-y-1.5">
              <label htmlFor={`${formId}-plan`} className="text-sm font-medium">
                Plan
              </label>
              <Select id={`${formId}-plan`} value={newPlan} onChange={(e) => setNewPlan(e.target.value as PlanId)}>
                {plans.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name} — {p.pricePerUser === null ? "Custom" : `$${p.pricePerUser}/user/mo`}
                  </option>
                ))}
              </Select>
            </div>
          )}
          <div className="flex justify-end gap-2 pt-2">
            <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={!newName.trim()}>
              Create tenant
            </Button>
          </div>
        </form>
      </Modal>

      {/* Detail drawer */}
      <TenantDrawer
        ws={selected}
        live={isLive}
        billing={billing}
        subscription={selected && isLive ? (billing.subscriptions?.get(selected.id) ?? null) : null}
        onBillingChanged={fetchBilling}
        onClose={() => setSelectedId(null)}
        onImpersonate={doImpersonate}
        onUpdate={updateWorkspace}
        onDelete={async (id) => {
          if (isLive) return;
          deleteWorkspace(id);
          setSelectedId(null);
          toast({ title: "Tenant deleted", tone: "danger" });
        }}
      />
    </PageStack>
  );
}

type PendingAction =
  | { kind: "plan"; plan: PlanDto }
  | { kind: "activate" }
  | { kind: "suspend" }
  | { kind: "paid"; invoice: PlatformInvoiceDto }
  | null;

function TenantDrawer({
  ws,
  live,
  billing,
  subscription,
  onBillingChanged,
  onClose,
  onImpersonate,
  onUpdate,
  onDelete,
}: {
  ws: Workspace | null;
  live: boolean;
  billing: Billing;
  /** The live tenant's subscription, when the server reports subscriptions. */
  subscription: WorkspaceSubscriptionDto | null;
  onBillingChanged: () => Promise<void>;
  onClose: () => void;
  onImpersonate: (ws: Workspace) => void;
  onUpdate: (id: string, patch: Partial<Workspace>) => void;
  onDelete: (id: string) => void | Promise<void>;
}) {
  const id = useId();
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  // Suspending cuts off every member of a paying workspace, so it asks first.
  const [confirmSuspend, setConfirmSuspend] = useState(false);
  const [planChoice, setPlanChoice] = useState("");
  const [pending, setPending] = useState<PendingAction>(null);
  const [busy, setBusy] = useState(false);

  if (!ws) return null;
  const m = tenantMetrics(ws);
  const owner = ownerOf(ws);
  const usage = workspaceUsage(ws);
  const invoices = invoicesFor(ws).slice(0, 4);
  const suspended = ws.status === "past_due";
  const liveStatus = live ? liveStatusOf(subscription) : null;
  // Suspend and reactivate need the tenant's subscription (reported by newer servers only).
  const canSuspendLive = live && liveStatus !== null;
  const seats = subscription?.seatsUsed ?? null;
  const suspendWho = seats === null ? "Its members" : `Its ${seats.toLocaleString("en-US")} ${seats === 1 ? "member" : "members"}`;
  const livePlans = [...(billing.plans ?? [])].sort((a, b) => a.pricePerUser - b.pricePerUser || a.name.localeCompare(b.name));
  const liveInvoices = (billing.invoices ?? [])
    .filter((inv) => inv.workspaceId === ws.id)
    .sort((a, b) => Date.parse(b.dueDate) - Date.parse(a.dueDate))
    .slice(0, 6);

  async function runPending() {
    if (!pending || !ws) return;
    setBusy(true);
    try {
      if (pending.kind === "plan") {
        await changeWorkspacePlan(ws.id, pending.plan.id);
        toast({ tone: "success", title: `${ws.name} moved to ${pending.plan.name}`, description: "Seat limits and billing follow the new plan from now on." });
        setPlanChoice("");
      } else if (pending.kind === "activate") {
        await activateSubscription(ws.id);
        toast({ tone: "success", title: `${ws.name}'s subscription is active`, description: "It is invoiced from the next billing run." });
      } else if (pending.kind === "suspend") {
        await suspendSubscription(ws.id);
        toast({ tone: "warning", title: `${ws.name} suspended`, description: "It can't record time or add people until you activate it again." });
      } else {
        await markInvoicePaid(pending.invoice.id);
        toast({ tone: "success", title: "Invoice marked paid", description: `${ws.name} · ${usd(pending.invoice.amount)}` });
      }
      setPending(null);
      await onBillingChanged();
    } catch (err) {
      toast({ tone: "danger", title: "The change wasn't saved", description: requestErrorText(err, "Something went wrong.") });
    } finally {
      setBusy(false);
    }
  }

  const chosenPlan = livePlans.find((p) => p.id === planChoice) ?? null;

  return (
    <Drawer open={!!ws} onClose={onClose} title="Tenant details">
      <div className="space-y-6">
        {/* Header */}
        <div className="flex items-center gap-3">
          <span aria-hidden className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl text-lg font-bold text-white" style={{ background: ws.color }}>
            {ws.name.charAt(0)}
          </span>
          <div className="min-w-0 flex-1">
            <div className="truncate text-lg font-semibold">{ws.name}</div>
            <div className="truncate text-xs text-muted-foreground">
              {live ? `Sign-in name: ${ws.name}` : `${ws.slug}.dositracker.app`}
              {ws.createdAt ? ` · since ${ws.createdAt}` : ""}
            </div>
          </div>
          {!live && <Badge tone={statusTone[ws.status]}>{hostStatusLabel[ws.status]}</Badge>}
          {liveStatus && <Badge tone={statusTone[liveStatus]}>{statusLabel[liveStatus]}</Badge>}
        </div>

        {/* Quick actions. Live: suspend or reactivate once the server reports the subscription (signing in
            as a tenant is a demo-only tool: the platform API has no impersonation). */}
        {live ? (
          canSuspendLive && (
            <div>
              {liveStatus && subscriptionNeedsAttention(liveStatus) ? (
                <Button variant="outline" className="w-full" onClick={() => setPending({ kind: "activate" })}>
                  <CheckCircle2 className="h-4 w-4" /> Reactivate
                </Button>
              ) : (
                <Button variant="outline" className="w-full" onClick={() => setPending({ kind: "suspend" })}>
                  <Ban className="h-4 w-4" /> Suspend
                </Button>
              )}
            </div>
          )
        ) : (
          <div className="grid grid-cols-2 gap-2">
            <Button onClick={() => onImpersonate(ws)} className="w-full">
              <LogIn className="h-4 w-4" /> Login as tenant
            </Button>
            {suspended ? (
              <Button
                variant="outline"
                className="w-full"
                onClick={() => {
                  onUpdate(ws.id, { status: "active" });
                  toast({ title: `${ws.name} reactivated`, description: "Members can sign in and track time again." });
                }}
              >
                <CheckCircle2 className="h-4 w-4" /> Reactivate
              </Button>
            ) : (
              <Button variant="outline" className="w-full" onClick={() => setConfirmSuspend(true)}>
                <Ban className="h-4 w-4" /> Suspend
              </Button>
            )}
          </div>
        )}

        {live ? (
          <>
            {/* Subscription (live): the current plan, change it, activate after payment. */}
            <section aria-labelledby={`${id}-sub`} className="space-y-3">
              <h3 id={`${id}-sub`} className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                Subscription
              </h3>
              {subscription && liveStatus ? (
                <dl className="grid grid-cols-2 gap-3 rounded-xl border border-border p-3 text-sm">
                  <div className="min-w-0">
                    <dt className="text-xs text-muted-foreground">Plan</dt>
                    <dd className="truncate font-medium">{subscription.planName ?? "Unknown plan"}</dd>
                  </div>
                  <div className="min-w-0">
                    <dt className="text-xs text-muted-foreground">Seats used</dt>
                    <dd className="font-medium tabular-nums">
                      {subscription.seatsUsed.toLocaleString("en-US")}
                      {typeof subscription.maxSeats === "number" && ` of ${subscription.maxSeats.toLocaleString("en-US")}`}
                    </dd>
                  </div>
                  {liveStatus === "trialing" && subscription.trialEndsAt && (
                    <div className="col-span-2 min-w-0">
                      <dt className="text-xs text-muted-foreground">Trial ends</dt>
                      <dd className="font-medium">{subscription.trialEndsAt.slice(0, 10)}</dd>
                    </div>
                  )}
                </dl>
              ) : (
                <p className="text-xs text-muted-foreground">
                  {billing.subscriptions
                    ? "This tenant has no subscription yet; choose a plan to start one."
                    : "This server doesn't report each tenant's current plan; its invoices below show what it was billed."}
                </p>
              )}
              <div className="space-y-1.5">
                <label htmlFor={`${id}-plan`} className="text-sm font-medium">
                  Move to plan
                </label>
                <div className="flex flex-col gap-2 sm:flex-row">
                  <Select id={`${id}-plan`} value={planChoice} onChange={(e) => setPlanChoice(e.target.value)} disabled={!billing.plans}>
                    <option value="">{billing.plans ? "Choose a plan…" : "Plans couldn't be loaded"}</option>
                    {livePlans.map((p) => (
                      <option key={p.id} value={p.id}>
                        {p.name} — {p.pricePerUser > 0 ? `$${p.pricePerUser}/user/mo` : "free"} · {p.maxSeats} seats
                      </option>
                    ))}
                  </Select>
                  <Button variant="outline" className="shrink-0" disabled={!chosenPlan} onClick={() => chosenPlan && setPending({ kind: "plan", plan: chosenPlan })}>
                    <Repeat className="h-4 w-4" /> Change plan
                  </Button>
                </div>
              </div>
              <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-border p-3">
                <div className="min-w-0 text-sm">
                  <div className="font-medium">Activate subscription</div>
                  <div className="text-xs text-muted-foreground">After payment, or to end a trial: the tenant becomes active and is invoiced.</div>
                </div>
                <Button variant="outline" size="sm" onClick={() => setPending({ kind: "activate" })}>
                  <BadgeCheck className="h-3.5 w-3.5" /> Activate
                </Button>
              </div>
            </section>

            <section aria-labelledby={`${id}-inv`} className="space-y-2">
              <h3 id={`${id}-inv`} className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                Recent invoices
              </h3>
              {billing.invoices === null ? (
                <p className="text-xs text-muted-foreground">Invoices couldn&apos;t be loaded.</p>
              ) : liveInvoices.length === 0 ? (
                <p className="text-xs text-muted-foreground">No invoices for this tenant yet.</p>
              ) : (
                <ul className="space-y-1.5">
                  {liveInvoices.map((inv) => (
                    <li key={inv.id} className="flex flex-wrap items-center justify-between gap-2 rounded-lg bg-muted/50 px-3 py-2 text-sm">
                      <span className="text-muted-foreground">Due {inv.dueDate.slice(0, 10)}</span>
                      <span className="flex items-center gap-2">
                        <span className="font-medium">{usd(inv.amount)}</span>
                        <Badge tone={inv.status === "paid" ? "success" : inv.status === "failed" ? "danger" : "warning"} className="capitalize">
                          {inv.status}
                        </Badge>
                        {inv.status !== "paid" && (
                          <Button size="sm" variant="ghost" onClick={() => setPending({ kind: "paid", invoice: inv })} aria-label={`Mark the invoice due ${inv.dueDate.slice(0, 10)} paid`}>
                            Mark paid
                          </Button>
                        )}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          </>
        ) : (
          <>
            {/* Metrics */}
            <div className="grid grid-cols-2 gap-3">
              <MiniMetric icon={Users} label="Members" value={String(m.members)} tone="#0ea5e9" />
              <MiniMetric icon={FolderKanban} label="Projects" value={String(m.projects)} tone="#6d5efc" />
              <MiniMetric icon={DollarSign} label="MRR" value={usd(m.mrr)} tone="#22c55e" />
              <MiniMetric icon={Gauge} label="Avg activity" value={`${m.avgProductivity}%`} tone="#ec4899" />
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
              <label htmlFor={`${id}-demo-plan`} className="mb-2 block text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                Subscription plan
              </label>
              <Select
                id={`${id}-demo-plan`}
                value={ws.planId}
                onChange={(e) => {
                  const next = e.target.value as PlanId;
                  onUpdate(ws.id, { planId: next });
                  // Changing a customer's plan changes what they are billed —
                  // say so rather than letting the select shift in silence.
                  toast({ title: `${ws.name} moved to ${planById(next).name}`, description: "Seat limits and billing update immediately." });
                }}
              >
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
                      <span className="font-medium">
                        {u.used.toLocaleString()} / {fmtLimit(u.limit)} {u.unit}
                      </span>
                    </div>
                    <Progress value={usagePct(u.used, u.limit)} aria-label={`${u.label} used`} />
                  </div>
                ))}
              </div>
            </div>

            {/* Invoices */}
            <div>
              <div className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">Recent invoices</div>
              <ul className="space-y-1.5">
                {invoices.map((inv) => (
                  <li key={inv.id} className="flex items-center justify-between rounded-lg bg-muted/50 px-3 py-2 text-sm">
                    <span className="text-muted-foreground">{inv.date}</span>
                    <span className="flex items-center gap-2">
                      <span className="font-medium">{usd(inv.amount)}</span>
                      <Badge tone={inv.status === "paid" ? "success" : inv.status === "due" ? "warning" : "muted"}>{inv.status}</Badge>
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          </>
        )}

        {/* Danger zone (demo only: live workspaces are deleted by their owners) */}
        {!live && (
        <div className="rounded-xl border border-danger/30 bg-danger/5 p-3">
          <div className="text-sm font-medium text-danger-on-tint">Danger zone</div>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {live
              ? "Deleting the tenant stops every sign-in to it and removes it from the platform lists. Its tracked time, captures and invoices are not erased from the database. This cannot be undone."
              : "Deleting a tenant removes the workspace and its data from this demo. This cannot be undone."}
          </p>
          {confirmDelete ? (
            <div className="mt-3 flex flex-wrap gap-2">
              <Button variant="outline" size="sm" disabled={deleting} onClick={() => setConfirmDelete(false)}>
                Cancel
              </Button>
              <Button
                size="sm"
                variant="danger"
                disabled={deleting}
                onClick={async () => {
                  setDeleting(true);
                  try {
                    await onDelete(ws.id);
                  } finally {
                    setDeleting(false);
                  }
                }}
              >
                <Trash2 className="h-4 w-4" /> {deleting ? "Deleting…" : `Yes, delete ${ws.name}`}
              </Button>
            </div>
          ) : (
            <Button variant="outline" size="sm" className="mt-3 border-danger/40 text-danger-on-tint hover:bg-danger/10" onClick={() => setConfirmDelete(true)}>
              <Trash2 className="h-4 w-4" /> Delete tenant
            </Button>
          )}
        </div>
        )}
      </div>

      <ConfirmDialog
        open={confirmSuspend}
        onCancel={() => setConfirmSuspend(false)}
        onConfirm={() => {
          onUpdate(ws.id, { status: "past_due" });
          setConfirmSuspend(false);
          toast({
            tone: "warning",
            title: `${ws.name} suspended`,
            description: "Members can no longer sign in. Reactivate any time from this panel.",
          });
        }}
        title={`Suspend ${ws.name}?`}
        description={`All ${liveSeatsUsed(ws)} members lose access until you reactivate the workspace. Tracking stops and their data is retained.`}
        confirmLabel="Suspend workspace"
      />

      <ConfirmDialog
        open={pending !== null}
        onCancel={() => setPending(null)}
        onConfirm={() => void runPending()}
        busy={busy}
        tone={pending?.kind === "suspend" ? "danger" : "primary"}
        title={
          pending?.kind === "plan"
            ? `Move ${ws.name} to ${pending.plan.name}?`
            : pending?.kind === "activate"
              ? `Activate ${ws.name}'s subscription?`
              : pending?.kind === "suspend"
                ? `Suspend ${ws.name}?`
                : "Mark this invoice paid?"
        }
        description={
          pending?.kind === "plan"
            ? `Its seat limit becomes ${pending.plan.maxSeats} and seats are billed at ${pending.plan.pricePerUser > 0 ? `$${pending.plan.pricePerUser} per user a month` : "nothing (free plan)"} from now on.`
            : pending?.kind === "activate"
              ? "Use this once the customer has paid or to end a trial early. The subscription becomes active and is invoiced from the next billing run."
              : pending?.kind === "suspend"
                ? `${suspendWho} can no longer record time, and nobody can be added, until you activate it again. Their data is kept.`
                : pending?.kind === "paid"
                  ? `${ws.name} · ${usd(pending.invoice.amount)} · due ${pending.invoice.dueDate.slice(0, 10)}. Record it only once the payment has arrived.`
                  : ""
        }
        confirmLabel={
          pending?.kind === "plan"
            ? "Change plan"
            : pending?.kind === "activate"
              ? "Activate subscription"
              : pending?.kind === "suspend"
                ? "Suspend workspace"
                : "Mark paid"
        }
      />
    </Drawer>
  );
}

function MiniMetric({ icon: Icon, label, value, tone }: { icon: typeof Users; label: string; value: string; tone: string }) {
  return (
    <div className="rounded-xl border border-border p-3">
      <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <Icon aria-hidden className="h-3.5 w-3.5" style={{ color: tone }} /> {label}
      </div>
      <div className="mt-1 text-base font-bold">{value}</div>
    </div>
  );
}
