"use client";

import { useEffect, useState } from "react";
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
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { DataTable, type Column } from "@/components/ui/data-table";
import { ExportMenu } from "@/components/reports/report-shell";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { useSession } from "@/components/session-provider";
import { toast } from "@/components/toast";
import {
  fmtLimit,
  invoicesFor,
  liveSeatsUsed,
  monthlyCost,
  planById,
  plans,
  statusLabel,
  usagePct,
  workspaceUsage,
  type Invoice,
  type PlanId,
  type SubscriptionStatus,
} from "@/lib/saas-data";
import { getApi } from "@/hooks/useApi";
import { exportRecords } from "@/lib/export";
import { cn } from "@/lib/utils";

/** Live billing state pulled from the ABP backend, when a real session exists. */
interface LiveBilling {
  planId: PlanId;
  planName: string;
  status: SubscriptionStatus;
  cycleDays: number;
  invoices: Invoice[];
}

async function fetchLiveBilling(): Promise<LiveBilling | null> {
  const [subscription, apiPlans, apiInvoices] = await Promise.all([
    getApi("/api/app/workspace/current-subscription").catch(() => null),
    getApi("/api/app/workspace/available-plans").catch(() => []),
    getApi("/api/app/billing/invoices").catch(() => []),
  ]);
  if (!subscription?.planId) return null;

  const planRow = (Array.isArray(apiPlans) ? apiPlans : []).find((p: any) => p.id === subscription.planId);
  const planName: string = planRow?.name ?? "Free";
  const planId = (planName.toLowerCase() as PlanId) ?? "free";

  const status: SubscriptionStatus =
    subscription.status === "trialing" ? "trialing" : subscription.status === "past_due" ? "past_due" : "active";
  const cycleDays = subscription.trialEndsAt
    ? Math.max(0, Math.ceil((new Date(subscription.trialEndsAt).getTime() - Date.now()) / 86400_000))
    : 0;

  const now = Date.now();
  const invoices: Invoice[] = (Array.isArray(apiInvoices) ? apiInvoices : []).map((inv: any) => ({
    id: inv.id,
    date: (inv.creationTime ?? inv.dueDate ?? "").slice(0, 10),
    amount: Number(inv.amount ?? 0),
    status:
      inv.status === "paid"
        ? "paid"
        : new Date(inv.dueDate).getTime() < now
          ? "due"
          : "upcoming",
    plan: planName,
  }));

  return { planId, planName, status, cycleDays, invoices };
}

const usageIcon: Record<string, LucideIcon> = {
  Seats: Users,
  Projects: FolderKanban,
  Storage: HardDrive,
  "Screenshot history": Clock,
};

export default function BillingPage() {
  const { workspace } = useSession();
  const [live, setLive] = useState<LiveBilling | null>(null);

  useEffect(() => {
    if (typeof window !== "undefined" && localStorage.getItem("dosi-token")) {
      fetchLiveBilling().then(setLive).catch(() => setLive(null));
    }
  }, []);

  const plan = planById(live?.planId ?? workspace.planId);
  const status: SubscriptionStatus = live?.status ?? workspace.status;
  const cycleDays = live?.cycleDays ?? workspace.cycleDays;
  const usage = workspaceUsage(workspace);
  const cost = monthlyCost(workspace);
  const invoices = live?.invoices ?? invoicesFor(workspace);
  const planOrder = plans.map((p) => p.id);
  const currentIdx = planOrder.indexOf(plan.id);

  function changePlan(name: string, isUpgrade: boolean) {
    toast({
      title: isUpgrade ? `Upgrade to ${name}` : `Switch to ${name}`,
      description: "Demo mode — connect billing (Stripe/ABP SaaS) to complete.",
      tone: "info",
    });
  }

  const invoiceColumns: Column<Invoice>[] = [
    { key: "date", header: "Date", sortValue: (r) => r.date, render: (r) => new Date(r.date + "T00:00:00Z").toLocaleDateString("en", { year: "numeric", month: "short", day: "numeric", timeZone: "UTC" }) },
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
      { header: "Plan", value: (r) => r.plan },
      { header: "Amount USD", value: (r) => r.amount },
      { header: "Status", value: (r) => r.status },
    ]);
  }

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
            <div className="text-sm font-medium">Your {plan.name} trial ends in {cycleDays} days</div>
            <div className="text-xs text-muted-foreground">Add a payment method to keep monitoring without interruption.</div>
          </div>
          <Button size="sm" onClick={() => changePlan(plan.name, true)}>Add payment method</Button>
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
                  <div className="text-xl font-bold">{plan.name}</div>
                  <div className="text-sm text-muted-foreground">{plan.tagline}</div>
                </div>
              </div>
              <div className="text-right">
                <div className="text-2xl font-bold">
                  {cost === null ? "Custom" : `$${cost.toLocaleString()}`}
                  {cost !== null && <span className="text-sm font-normal text-muted-foreground">/mo</span>}
                </div>
                <div className="text-xs text-muted-foreground">
                  {plan.pricePerUser !== null ? `$${plan.pricePerUser}/user · ${liveSeatsUsed(workspace)} seats` : "Contact sales"}
                  {status === "active" && cycleDays > 0 && ` · renews in ${cycleDays}d`}
                </div>
              </div>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader><CardTitle>Payment method</CardTitle></CardHeader>
          <CardContent className="space-y-3">
            <div className="flex items-center gap-3 rounded-lg border border-border p-3">
              <CreditCard className="h-5 w-5 text-muted-foreground" />
              <div className="min-w-0 flex-1">
                <div className="text-sm font-medium">Visa ···· 4242</div>
                <div className="text-xs text-muted-foreground">Expires 08 / 28</div>
              </div>
            </div>
            <Button variant="outline" className="w-full" onClick={() => toast({ title: "Update payment method", description: "Demo mode.", tone: "info" })}>
              Update card
            </Button>
          </CardContent>
        </Card>
      </div>

      {/* Usage */}
      <Card>
        <CardHeader>
          <CardTitle>Usage this cycle</CardTitle>
          <span className="text-xs text-muted-foreground">Limits from your {plan.name} plan</span>
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
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-4">
          {plans.map((p, i) => {
            const isCurrent = p.id === plan.id;
            const isUpgrade = i > currentIdx;
            return (
              <Card key={p.id} className={cn("flex flex-col p-5", p.highlight && !isCurrent && "ring-1 ring-primary/40", isCurrent && "border-primary")}>
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
                  ) : p.pricePerUser === null ? (
                    <Button variant="outline" className="w-full" onClick={() => toast({ title: "Contact sales", description: "We'll reach out about Enterprise.", tone: "info" })}>
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
          <DataTable columns={invoiceColumns} rows={invoices} initialSort={{ key: "date", dir: "desc" }} emptyText="No invoices yet." />
        </CardContent>
      </Card>

      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <Building2 className="h-3.5 w-3.5" />
        Tenant: <span className="font-medium text-foreground">{workspace.slug}.dositracker.app</span> · Billing is per-workspace.
      </div>
    </PageStack>
  );
}
