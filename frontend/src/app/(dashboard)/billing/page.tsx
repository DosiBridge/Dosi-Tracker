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
  planNameFor,
  plans,
  statusLabel,
  usagePct,
  workspaceUsage,
  type Invoice,
} from "@/lib/saas-data";
import { SUPPORT_EMAIL } from "@/lib/brand";
import { getApi } from "@/hooks/useApi";
import { exportRecords } from "@/lib/export";
import { cn } from "@/lib/utils";

/** GET /api/app/billing/invoices item (InvoiceDto). */
interface ApiInvoice {
  id: string;
  amount?: number | null;
  status?: string | null;
  dueDate?: string | null;
  creationTime?: string | null;
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
}

/** Backend invoice statuses are paid / pending / failed; show pending ones as due once past their due date. */
function toInvoice(inv: ApiInvoice, planName: string, now: number): Invoice {
  const due = Date.parse(inv.dueDate ?? "");
  const status: Invoice["status"] =
    inv.status === "paid" ? "paid" : inv.status === "failed" || (Number.isFinite(due) && due < now) ? "due" : "upcoming";
  return {
    id: inv.id,
    date: (inv.creationTime ?? inv.dueDate ?? "").slice(0, 10),
    amount: Number(inv.amount ?? 0),
    status,
    plan: planName,
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

function mailtoChange(workspaceName: string, planName?: string): string {
  const subject = planName ? `Change plan to ${planName} — ${workspaceName}` : `Billing — ${workspaceName}`;
  return `mailto:${SUPPORT_EMAIL}?subject=${encodeURIComponent(subject)}`;
}

export default function BillingPage() {
  const { workspace, isLive } = useSession();

  // LIVE: real invoices and the backend's own plan list. The current plan,
  // status and seat usage come from the session's live workspace (built from
  // the real subscription). Nothing here is demo data in a live session.
  const [liveInvoices, setLiveInvoices] = useState<Invoice[] | null>(null);
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
    if (!isLive) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void loadLive();
  }, [isLive, loadLive]);

  const demoPlan = planById(workspace.planId);
  const planName = planNameFor(workspace);
  const tagline = isLive ? (workspace.live?.known ? "Your current subscription" : "Plan details unavailable") : demoPlan.tagline;
  const pricePerUser = isLive ? (workspace.live?.pricePerUser ?? null) : demoPlan.pricePerUser;
  const status = workspace.status;
  const cycleDays = workspace.cycleDays;
  const usage = workspaceUsage(workspace);
  const cost = monthlyCost(workspace);
  const invoices = isLive ? (liveInvoices ?? []) : invoicesFor(workspace);

  const planCards: PlanCard[] = isLive
    ? [...(livePlans ?? [])]
        .sort((a, b) => Number(a.pricePerUser ?? 0) - Number(b.pricePerUser ?? 0))
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
        }))
    : plans.map((p) => ({
        key: p.id,
        name: p.name,
        pricePerUser: p.pricePerUser,
        tagline: p.tagline,
        features: p.features,
        highlight: !!p.highlight,
      }));
  const currentIdx = planCards.findIndex((p) => p.name.toLowerCase() === planName.toLowerCase());

  function changePlan(name: string, isUpgrade: boolean) {
    toast({
      title: isUpgrade ? `Upgrade to ${name}` : `Switch to ${name}`,
      description: PAYMENTS_NOTICE,
      tone: "info",
    });
  }

  const invoiceColumns: Column<Invoice>[] = [
    { key: "date", header: "Date", sortValue: (r) => r.date, render: (r) => (r.date ? new Date(r.date + "T00:00:00Z").toLocaleDateString("en", { year: "numeric", month: "short", day: "numeric", timeZone: "UTC" }) : "—") },
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
            <div className="text-sm font-medium">Your {planName} trial ends in {cycleDays} days</div>
            <div className="text-xs text-muted-foreground">
              {isLive ? `To keep your plan after the trial, email ${SUPPORT_EMAIL}.` : "Add a payment method to keep monitoring without interruption."}
            </div>
          </div>
          {isLive ? (
            <a href={mailtoChange(workspace.name, planName)}>
              <Button size="sm"><Mail className="h-4 w-4" /> Contact support</Button>
            </a>
          ) : (
            <Button size="sm" onClick={() => changePlan(planName, true)}>Add payment method</Button>
          )}
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
                  ) : isLive ? (
                    <a href={mailtoChange(workspace.name, p.name)} className="block">
                      <Button variant={isUpgrade ? "primary" : "outline"} className="w-full" title={PAYMENTS_NOTICE}>
                        <Mail className="h-4 w-4" /> Request {isUpgrade ? "upgrade" : "switch"}
                      </Button>
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
