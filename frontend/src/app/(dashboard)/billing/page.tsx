"use client";

import { useCallback, useEffect, useState } from "react";
import {
  Check,
  Crown,
  CreditCard,
  Users,
  FolderKanban,
  HardDrive,
  Clock,
  AlertTriangle,
  Building2,
  Mail,
  CircleSlash,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Progress } from "@/components/ui/progress";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ExportMenu } from "@/components/reports/export-menu";
import { linkButtonClass } from "@/components/dashboard/link-button";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { useSession } from "@/components/session-provider";
import { toast } from "@/components/toast";
import {
  billingEnabledFor,
  fmtLimit,
  invoicesFor,
  liveSeatsUsed,
  monthlyCost,
  planById,
  planNameFor,
  plans,
  statusLabel,
  subscriptionNeedsAttention,
  usagePct,
  workspaceUsage,
  type Invoice,
  type SubscriptionStatus,
} from "@/lib/saas-data";
import { SUPPORT_EMAIL } from "@/lib/brand";
import { cancelMySubscriptionApi, changeMyPlanApi, getApi } from "@/hooks/useApi";
import { billingErrorText, canCancelSubscription, fittingFreePlan, planAction, type PlanAction } from "@/lib/billing-plans";
import { normalizeUtcIso } from "@/lib/live-dataset";
import { exportRecords } from "@/lib/export";
import { cn } from "@/lib/utils";

/** GET /api/app/billing/invoices item (InvoiceDto). */
interface ApiInvoice {
  id: string;
  amount?: number | null;
  status?: string | null;
  dueDate?: string | null;
  creationTime?: string | null;
  /** Start of the billed month (newer servers). */
  billingPeriodStart?: string | null;
}

/** An invoice row, with the billed month when the server says. */
type InvoiceRow = Invoice & { period?: string | null };

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"] as const;

/** "Oct 2026" for the start of a billing period (a UTC month start), the same in every browser. */
function periodLabel(iso: string | null | undefined): string | null {
  const at = normalizeUtcIso(iso);
  if (!at) return null;
  const d = new Date(at);
  return `${MONTHS[d.getUTCMonth()]} ${d.getUTCFullYear()}`;
}

/** GET /api/app/workspace/available-plans item (PlanDto). */
interface ApiPlan {
  id: string;
  name?: string | null;
  pricePerUser?: number | null;
  maxSeats?: number | null;
  trialDays?: number | null;
}

/** One plan card, from the backend (live) or the demo plan table. */
interface PlanCard {
  key: string;
  name: string;
  pricePerUser: number | null;
  tagline: string;
  features: string[];
  highlight: boolean;
  /** Live: what the owner can do about it (switch to a free plan, ask for a paid one). */
  action?: PlanAction;
}

/** A self-service change waiting for the owner's confirmation. */
type Pending = { kind: "switch"; planId: string; name: string } | { kind: "cancel" };

/** Backend invoice statuses are paid / pending / failed; show pending ones as due once past their due date. */
function toInvoice(inv: ApiInvoice, planName: string, now: number): InvoiceRow {
  const due = Date.parse(inv.dueDate ?? "");
  const status: Invoice["status"] =
    inv.status === "paid" ? "paid" : inv.status === "failed" || (Number.isFinite(due) && due < now) ? "due" : "upcoming";
  return {
    id: inv.id,
    date: (inv.creationTime ?? inv.dueDate ?? "").slice(0, 10),
    amount: Number(inv.amount ?? 0),
    status,
    plan: planName,
    period: periodLabel(inv.billingPeriodStart),
  };
}

const usageIcon: Record<string, LucideIcon> = {
  Seats: Users,
  Projects: FolderKanban,
  Storage: HardDrive,
  "Screenshot history": Clock,
};

/** Honest copy for plan/payment actions until online payments exist. */
const PAYMENTS_NOTICE = `Online payments are coming soon. To change your plan or billing details, email ${SUPPORT_EMAIL}.`;

/**
 * What an overdue or cut-off subscription means for the team: the server
 * refuses tracked time and new people (Tracker:SubscriptionInactive) once a
 * workspace is suspended, canceled or expired, and past due after the grace
 * period.
 */
const ATTENTION_COPY: Partial<Record<SubscriptionStatus, { title: string; detail: string }>> = {
  past_due: {
    title: "Payment is overdue",
    detail: "Settle the open invoice to keep tracking: once the grace period ends, members can't record time and nobody can be added.",
  },
  suspended: {
    title: "This workspace is suspended",
    detail: "Members can't record time and nobody can be added until it's reactivated. Your data is kept.",
  },
  canceled: {
    title: "This workspace's subscription is canceled",
    detail: "Members can't record time and nobody can be added until a plan is active again. Your data is kept.",
  },
  expired: {
    title: "This workspace's subscription has expired",
    detail: "Members can't record time and nobody can be added until a plan is active again. Your data is kept.",
  },
};

function mailtoChange(workspaceName: string, planName?: string): string {
  const subject = planName ? `Change plan to ${planName} — ${workspaceName}` : `Billing — ${workspaceName}`;
  return `mailto:${SUPPORT_EMAIL}?subject=${encodeURIComponent(subject)}`;
}

export default function BillingPage() {
  const { workspace, isLive, refreshSession, status: sessionStatus } = useSession();
  // A deployment that doesn't bill (Billing:Enabled off): every workspace is active, there is nothing to show or pay.
  const billingOn = billingEnabledFor(workspace);
  const [pending, setPending] = useState<Pending | null>(null);
  const [changing, setChanging] = useState(false);

  // LIVE: real invoices and the backend's own plan list. The current plan,
  // status and seat usage come from the session's live workspace (built from
  // the real subscription). Nothing here is demo data in a live session.
  const [liveInvoices, setLiveInvoices] = useState<InvoiceRow[] | null>(null);
  const [livePlans, setLivePlans] = useState<ApiPlan[] | null>(null);
  const [liveLoading, setLiveLoading] = useState(false);
  const [liveError, setLiveError] = useState(false);

  const loadLive = useCallback(async () => {
    setLiveLoading(true);
    setLiveError(false);
    try {
      const [apiInvoices, apiPlans] = await Promise.all([
        getApi("/api/app/billing/invoices?MaxResultCount=100"),
        getApi("/api/app/workspace/available-plans").catch(() => null),
      ]);
      const now = Date.now();
      setLiveInvoices(
        (Array.isArray(apiInvoices) ? (apiInvoices as ApiInvoice[]) : []).map((inv) => toInvoice(inv, planNameFor(workspace), now)),
      );
      setLivePlans(Array.isArray(apiPlans) ? (apiPlans as ApiPlan[]) : null);
    } catch {
      setLiveInvoices([]);
      setLiveError(true);
    } finally {
      setLiveLoading(false);
    }
  }, [workspace]);

  useEffect(() => {
    // Wait for the real workspace: the placeholder shown while it loads can't say whether this deployment bills.
    if (!isLive || sessionStatus !== "ready" || !billingOn) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void loadLive();
  }, [isLive, sessionStatus, billingOn, loadLive]);

  const demoPlan = planById(workspace.planId);
  const planName = planNameFor(workspace);
  const tagline = isLive ? (workspace.live?.known ? "Your current subscription" : "Plan details unavailable") : demoPlan.tagline;
  const pricePerUser = isLive ? (workspace.live?.pricePerUser ?? null) : demoPlan.pricePerUser;
  const status = workspace.status;
  const cycleDays = workspace.cycleDays;
  const usage = workspaceUsage(workspace);
  const cost = monthlyCost(workspace);
  const invoices: InvoiceRow[] = isLive ? (liveInvoices ?? []) : invoicesFor(workspace);
  const hasPeriods = invoices.some((r) => r.period);

  const seats = liveSeatsUsed(workspace);
  const currentPlanId = workspace.live?.planId ?? null;
  const sortedPlans = [...(livePlans ?? [])].sort((a, b) => Number(a.pricePerUser ?? 0) - Number(b.pricePerUser ?? 0));
  const freeChoice = isLive ? fittingFreePlan(sortedPlans, seats, currentPlanId) : null;
  const canCancel = isLive && canCancelSubscription({ pricePerUser, status });

  /** Run the confirmed change, then re-read the subscription (the session's workspace is rebuilt from it). */
  async function applyPending() {
    if (!pending) return;
    setChanging(true);
    try {
      if (pending.kind === "switch") await changeMyPlanApi(pending.planId);
      else await cancelMySubscriptionApi();
      toast({
        tone: "success",
        title: pending.kind === "switch" ? `Moved to ${pending.name}` : "Subscription canceled",
        description:
          pending.kind === "switch"
            ? "The workspace is active on the new plan."
            : "No more invoices will be issued. Time tracking pauses until you switch to a free plan.",
      });
      setPending(null);
      await refreshSession({ silent: true, freshPlan: true });
      void loadLive();
    } catch (err) {
      toast({
        tone: "danger",
        title: pending.kind === "switch" ? "Couldn't change the plan" : "Couldn't cancel the subscription",
        description: billingErrorText(err, "Try again in a moment."),
      });
      setPending(null);
    } finally {
      setChanging(false);
    }
  }

  const planCards: PlanCard[] = isLive
    ? sortedPlans
        .map((p) => ({
          key: p.id,
          name: p.name ?? "Plan",
          pricePerUser: typeof p.pricePerUser === "number" ? p.pricePerUser : null,
          tagline: typeof p.maxSeats === "number" && p.maxSeats > 0 ? `Up to ${p.maxSeats} seats` : "Unlimited seats",
          features: [
            typeof p.maxSeats === "number" && p.maxSeats > 0 ? `${p.maxSeats} seats` : "Unlimited seats",
            p.trialDays ? `${p.trialDays}-day free trial` : "No trial needed",
          ],
          highlight: false,
          action: planAction(p, { currentPlanId, seatsUsed: seats }),
        }))
    : plans.map((p) => ({
        key: p.id,
        name: p.name,
        pricePerUser: p.pricePerUser,
        tagline: p.tagline,
        features: p.features,
        highlight: !!p.highlight,
      }));
  const currentIdx = currentPlanId
    ? planCards.findIndex((p) => p.key === currentPlanId)
    : planCards.findIndex((p) => p.name.toLowerCase() === planName.toLowerCase());

  function changePlan(name: string, isUpgrade: boolean) {
    toast({
      title: isUpgrade ? `Upgrade to ${name}` : `Switch to ${name}`,
      description: PAYMENTS_NOTICE,
      tone: "info",
    });
  }

  const invoiceColumns: Column<InvoiceRow>[] = [
    { key: "date", header: "Date", sortValue: (r) => r.date, render: (r) => (r.date ? new Date(r.date + "T00:00:00Z").toLocaleDateString("en", { year: "numeric", month: "short", day: "numeric", timeZone: "UTC" }) : "—") },
    ...(hasPeriods ? [{ key: "period", header: "Period", render: (r: InvoiceRow) => r.period ?? "—" }] : []),
    { key: "plan", header: "Plan", render: (r) => r.plan },
    { key: "amount", header: "Amount", align: "right", sortValue: (r) => r.amount, render: (r) => <span className="font-medium">${r.amount.toLocaleString()}</span> },
    {
      key: "status",
      header: "Status",
      align: "right",
      render: (r) => (
        <Badge tone={r.status === "paid" ? "success" : r.status === "due" ? "danger" : "warning"} className="capitalize">
          {r.status}
        </Badge>
      ),
    },
  ];

  function exportInvoices() {
    exportRecords(`invoices-${workspace.slug}`, invoices, [
      { header: "Date", value: (r) => r.date },
      { header: "Period", value: (r) => r.period ?? "" },
      { header: "Plan", value: (r) => r.plan },
      { header: "Amount USD", value: (r) => r.amount },
      { header: "Status", value: (r) => r.status },
    ]);
  }

  if (isLive && !billingOn) {
    return (
      <PageStack>
        <PageHeader eyebrow="Workspace" title="Billing & Plan" description={<>Billing for <span className="font-medium text-foreground">{workspace.name}</span>.</>} />
        <Card className="flex items-start gap-3 p-5">
          <CircleSlash aria-hidden className="mt-0.5 h-5 w-5 shrink-0 text-muted-foreground" />
          <div className="min-w-0 space-y-1">
            <h2 className="text-sm font-semibold">Billing is turned off on this server</h2>
            <p className="text-sm text-muted-foreground">
              Every workspace here is active, with no plans, seat limits, trials or invoices. There is nothing to pay or change.
            </p>
          </div>
        </Card>
      </PageStack>
    );
  }

  const confirmCopy =
    pending?.kind === "switch"
      ? {
          title: `Switch to ${pending.name}?`,
          description: `The workspace moves to the ${pending.name} plan now and stays active. Its seat limit applies from now on.`,
          confirmLabel: `Switch to ${pending.name}`,
        }
      : {
          title: `Cancel the ${planName} subscription?`,
          description:
            "No more invoices are issued, and time tracking pauses for everyone until you switch to a free plan or the platform reactivates the workspace. Invoices already issued are still owed. Your data is kept.",
          confirmLabel: "Cancel subscription",
        };

  return (
    <PageStack>
      <PageHeader
        eyebrow="Workspace"
        title="Billing & Plan"
        description={
          <>
            Manage the subscription for <span className="font-medium text-foreground">{workspace.name}</span>.
          </>
        }
      />

      {/* Trial banner */}
      {status === "trialing" && (
        <Card className="flex flex-wrap items-center gap-3 border-warning/40 bg-warning/10 p-4">
          <AlertTriangle className="h-5 w-5 text-warning" />
          <div className="min-w-0 flex-1">
            <div className="text-sm font-medium">Your {planName} trial ends in {cycleDays} days</div>
            <div className="text-xs text-muted-foreground">
              {isLive
                ? freeChoice
                  ? `To keep tracking after the trial, switch to ${freeChoice.name ?? "a free plan"} — or email ${SUPPORT_EMAIL} to keep ${planName}.`
                  : `To keep your plan after the trial, email ${SUPPORT_EMAIL}.`
                : "Add a payment method to keep monitoring without interruption."}
            </div>
          </div>
          {isLive ? (
            <div className="flex flex-wrap gap-2">
              {freeChoice && (
                <Button size="sm" onClick={() => setPending({ kind: "switch", planId: freeChoice.id, name: freeChoice.name ?? "the free plan" })}>
                  Switch to {freeChoice.name ?? "free"}
                </Button>
              )}
              <a href={mailtoChange(workspace.name, planName)} className={linkButtonClass(freeChoice ? "outline" : "primary", "sm")}>
                <Mail aria-hidden className="h-4 w-4" /> Contact support
              </a>
            </div>
          ) : (
            <Button size="sm" onClick={() => changePlan(planName, true)}>Add payment method</Button>
          )}
        </Card>
      )}

      {/* Overdue or cut off: what it means for the team, and who can fix it */}
      {subscriptionNeedsAttention(status) && (
        <Card role="status" className="flex flex-wrap items-center gap-3 border-danger/40 bg-danger/10 p-4">
          <AlertTriangle aria-hidden className="h-5 w-5 text-danger-on-tint" />
          <div className="min-w-0 flex-1">
            <div className="text-sm font-medium text-danger-on-tint">{ATTENTION_COPY[status]?.title}</div>
            <div className="text-xs text-muted-foreground">{ATTENTION_COPY[status]?.detail}</div>
          </div>
          {freeChoice && (
            <Button size="sm" onClick={() => setPending({ kind: "switch", planId: freeChoice.id, name: freeChoice.name ?? "the free plan" })}>
              Switch to {freeChoice.name ?? "free"}
            </Button>
          )}
          <a href={mailtoChange(workspace.name)} className={linkButtonClass(freeChoice ? "outline" : "primary", "sm")}>
            <Mail aria-hidden className="h-4 w-4" /> Contact support
          </a>
        </Card>
      )}

      {/* Current plan + payment */}
      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle>Current plan</CardTitle>
            <Badge tone={status === "active" ? "success" : status === "trialing" ? "warning" : "danger"}>
              {statusLabel[status]}
            </Badge>
          </CardHeader>
          <CardContent>
            <div className="flex flex-wrap items-end justify-between gap-4">
              <div className="flex items-center gap-3">
                <div className="flex h-12 w-12 items-center justify-center rounded-xl brand-gradient text-white">
                  <Crown className="h-6 w-6" />
                </div>
                <div>
                  <div className="text-xl font-bold">{planName}</div>
                  <div className="text-sm text-muted-foreground">{tagline}</div>
                </div>
              </div>
              <div className="text-right">
                <div className="text-2xl font-bold">
                  {cost === null ? (isLive ? "—" : "Custom") : `$${cost.toLocaleString()}`}
                  {cost !== null && <span className="text-sm font-normal text-muted-foreground">/mo</span>}
                </div>
                <div className="text-xs text-muted-foreground">
                  {pricePerUser !== null ? `$${pricePerUser}/user · ${liveSeatsUsed(workspace)} seats` : isLive ? "Price unavailable" : "Contact sales"}
                  {status === "active" && cycleDays > 0 && ` · renews in ${cycleDays}d`}
                </div>
              </div>
            </div>
            {canCancel && (
              <div className="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-border pt-4">
                <p className="text-xs text-muted-foreground">Cancel any time: no more invoices; tracking pauses until a free plan.</p>
                <Button size="sm" variant="outline" className="border-danger/40 text-danger-on-tint hover:bg-danger/10" onClick={() => setPending({ kind: "cancel" })}>
                  Cancel subscription
                </Button>
              </div>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader><CardTitle>Payment method</CardTitle></CardHeader>
          <CardContent className="space-y-3">
            {isLive ? (
              <>
                <div className="flex items-center gap-3 rounded-lg border border-dashed border-border p-3">
                  <CreditCard className="h-5 w-5 text-muted-foreground" />
                  <div className="min-w-0 flex-1 text-sm text-muted-foreground">No payment method on file.</div>
                </div>
                <p className="text-xs text-muted-foreground">{PAYMENTS_NOTICE}</p>
              </>
            ) : (
              <>
                <div className="flex items-center gap-3 rounded-lg border border-border p-3">
                  <CreditCard className="h-5 w-5 text-muted-foreground" />
                  <div className="min-w-0 flex-1">
                    <div className="text-sm font-medium">Visa ···· 4242</div>
                    <div className="text-xs text-muted-foreground">Expires 08 / 28</div>
                  </div>
                </div>
                <Button variant="outline" className="w-full" onClick={() => toast({ title: "Update payment method", description: PAYMENTS_NOTICE, tone: "info" })}>
                  Update card
                </Button>
              </>
            )}
          </CardContent>
        </Card>
      </div>

      {/* Usage */}
      <Card>
        <CardHeader>
          <CardTitle>Usage this cycle</CardTitle>
          <span className="text-xs text-muted-foreground">Limits from your {planName} plan</span>
        </CardHeader>
        <CardContent className="grid grid-cols-1 gap-5 sm:grid-cols-2 lg:grid-cols-4">
          {usage.map((m) => {
            const Icon = usageIcon[m.label] ?? Users;
            const pct = usagePct(m.used, m.limit);
            const near = pct >= 80;
            return (
              <div key={m.label}>
                <div className="mb-1.5 flex items-center gap-2 text-sm">
                  <Icon className="h-4 w-4 text-muted-foreground" />
                  <span className="font-medium">{m.label}</span>
                </div>
                <div className="mb-1 flex items-baseline justify-between text-sm">
                  <span className="font-semibold">{m.used.toLocaleString()}</span>
                  <span className="text-xs text-muted-foreground">of {fmtLimit(m.limit)} {m.unit}</span>
                </div>
                <Progress value={pct} color={near ? "#ef4444" : undefined} />
              </div>
            );
          })}
        </CardContent>
      </Card>

      {/* Plan comparison */}
      <div>
        <h2 className="mb-3 text-sm font-semibold text-muted-foreground">Available plans</h2>
        {isLive && (
          <p className="mb-3 rounded-lg border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">{PAYMENTS_NOTICE}</p>
        )}
        {isLive && liveLoading && planCards.length === 0 && (
          <p className="text-sm text-muted-foreground">Loading plans…</p>
        )}
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-4">
          {planCards.map((p, i) => {
            const isCurrent = i === currentIdx;
            const isUpgrade = currentIdx === -1 || i > currentIdx;
            return (
              <Card key={p.key} className={cn("flex flex-col p-5", p.highlight && !isCurrent && "ring-1 ring-primary/40", isCurrent && "border-primary")}>
                <div className="flex items-center justify-between">
                  <h3 className="font-semibold">{p.name}</h3>
                  {isCurrent ? <Badge tone="primary">Current</Badge> : p.highlight ? <Badge tone="info">Popular</Badge> : null}
                </div>
                <div className="mt-2">
                  <span className="text-2xl font-bold">{p.pricePerUser === null ? "Custom" : `$${p.pricePerUser}`}</span>
                  {p.pricePerUser !== null && <span className="text-sm text-muted-foreground">/user/mo</span>}
                </div>
                <p className="mt-1 text-xs text-muted-foreground">{p.tagline}</p>
                <ul className="mt-4 flex-1 space-y-2">
                  {p.features.map((f) => (
                    <li key={f} className="flex items-start gap-2 text-sm">
                      <Check className="mt-0.5 h-4 w-4 shrink-0 text-success" />
                      <span className="text-muted-foreground">{f}</span>
                    </li>
                  ))}
                </ul>
                <div className="mt-4">
                  {isCurrent ? (
                    <Button variant="outline" className="w-full" disabled>Current plan</Button>
                  ) : isLive && p.action?.kind === "switch" ? (
                    <Button variant="outline" className="w-full" onClick={() => setPending({ kind: "switch", planId: p.key, name: p.name })}>
                      Switch to {p.name}
                    </Button>
                  ) : isLive && p.action?.kind === "too-small" ? (
                    <div className="space-y-1.5">
                      <Button variant="outline" className="w-full" disabled>
                        Switch to {p.name}
                      </Button>
                      <p className="text-xs text-muted-foreground">
                        Your team uses {seats} seats; {p.name} allows {p.action.maxSeats}.
                      </p>
                    </div>
                  ) : isLive ? (
                    <a href={mailtoChange(workspace.name, p.name)} className={cn(linkButtonClass(isUpgrade ? "primary" : "outline", "md"), "w-full")} title={PAYMENTS_NOTICE}>
                      <Mail aria-hidden className="h-4 w-4" /> Request {isUpgrade ? "upgrade" : "switch"}
                    </a>
                  ) : p.pricePerUser === null ? (
                    <Button variant="outline" className="w-full" onClick={() => toast({ title: "Contact sales", description: `Email ${SUPPORT_EMAIL} about Enterprise.`, tone: "info" })}>
                      Contact sales
                    </Button>
                  ) : (
                    <Button variant={isUpgrade ? "primary" : "outline"} className="w-full" onClick={() => changePlan(p.name, isUpgrade)}>
                      {isUpgrade ? "Upgrade" : "Switch"}
                    </Button>
                  )}
                </div>
              </Card>
            );
          })}
        </div>
      </div>

      {/* Invoices */}
      <Card>
        <CardHeader>
          <CardTitle>Invoices</CardTitle>
          <ExportMenu onExportCSV={exportInvoices} />
        </CardHeader>
        <CardContent>
          {isLive && liveError ? (
            <div className="flex flex-col items-center gap-2 py-8 text-center text-sm text-muted-foreground">
              <span className="text-danger">Couldn&apos;t load invoices from the server.</span>
              <Button size="sm" variant="outline" onClick={() => void loadLive()}>Try again</Button>
            </div>
          ) : (
            <DataTable
              columns={invoiceColumns}
              rows={invoices}
              initialSort={{ key: "date", dir: "desc" }}
              emptyText={isLive && liveLoading ? "Loading invoices…" : "No invoices yet."}
            />
          )}
        </CardContent>
      </Card>

      <ConfirmDialog
        open={!!pending}
        onCancel={() => setPending(null)}
        onConfirm={() => void applyPending()}
        title={confirmCopy.title}
        description={confirmCopy.description}
        confirmLabel={confirmCopy.confirmLabel}
        tone={pending?.kind === "cancel" ? "danger" : "primary"}
        busy={changing}
      />

      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <Building2 className="h-3.5 w-3.5" />
        {isLive ? (
          <>
            Workspace: <span className="font-medium text-foreground">{workspace.name}</span> · Billing is per-workspace.
          </>
        ) : (
          <>
            Tenant: <span className="font-medium text-foreground">{workspace.slug}.dositracker.app</span> · Billing is per-workspace.
          </>
        )}
      </div>
    </PageStack>
  );
}
