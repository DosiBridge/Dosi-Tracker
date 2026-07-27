"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import Link from "next/link";
import { DollarSign, Building2, Users, Sparkles, ArrowRight, Receipt, CheckCircle2 } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { StatCard } from "@/components/dashboard/stat-card";
import { ProjectDonut } from "@/components/dashboard/charts";
import { useSession } from "@/components/session-provider";
import { brand } from "@/lib/brand";
import { planBreakdown, platformOverview } from "@/lib/host-data";
import { colorFromString } from "@/lib/utils";
import { getApi } from "@/hooks/useApi";

const usd = (n: number) => `$${Math.round(n).toLocaleString()}`;

interface PlanRow {
  name: string;
  subscribers: number;
  color: string;
}
interface Overview {
  tenants: number;
  activeSubs: number;
  trialing: number;
  mrr: number;
  paidSeats: number;
  totalInvoiced: number;
  pendingInvoices: number;
  plans: PlanRow[];
}

interface ApiOverview {
  tenantCount: number;
  activeSubscriptions: number;
  trialingSubscriptions: number;
  mrr: number;
  paidSeats: number;
  totalInvoiced: number;
  pendingInvoices: number;
  plans: { name: string; subscriberCount: number }[];
}

export default function HostOverviewPage() {
  const { workspaces } = useSession();

  const [live, setLive] = useState<Overview | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);

  const loadLive = useCallback(async () => {
    setLoading(true);
    setError(false);
    try {
      const o = (await getApi("/api/app/platform/overview")) as ApiOverview;
      setLive({
        tenants: o.tenantCount,
        activeSubs: o.activeSubscriptions,
        trialing: o.trialingSubscriptions,
        mrr: o.mrr,
        paidSeats: o.paidSeats,
        totalInvoiced: o.totalInvoiced,
        pendingInvoices: o.pendingInvoices,
        plans: (o.plans ?? []).map((p) => ({ name: p.name, subscribers: p.subscriberCount, color: colorFromString(p.name) })),
      });
    } catch {
      setLive(null);
      setError(true);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (typeof window === "undefined" || !localStorage.getItem("dosi-token")) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    loadLive();
  }, [loadLive]);

  const demo = useMemo<Overview>(() => {
    const o = platformOverview(workspaces);
    const b = planBreakdown(workspaces);
    return {
      tenants: o.totalTenants,
      activeSubs: o.active,
      trialing: o.trialing,
      mrr: o.mrr,
      paidSeats: o.totalSeats,
      totalInvoiced: 0,
      pendingInvoices: o.pastDue,
      plans: b.map((p) => ({ name: p.plan.name, subscribers: p.tenants, color: p.color })),
    };
  }, [workspaces]);

  const o = live ?? demo;
  const donut = o.plans.filter((p) => p.subscribers > 0).map((p) => ({ name: p.name, value: p.subscribers, color: p.color }));

  return (
    <PageStack>
      <PageHeader
        eyebrow="Host"
        title={
          <span className="inline-flex flex-wrap items-center gap-2">
            Platform Overview
            <Badge tone="success" className="gap-1.5 align-middle">
              <span className="h-1.5 w-1.5 rounded-full bg-success live-dot" />
              {o.activeSubs} active
            </Badge>
          </span>
        }
        description="Health of the whole Dosi-Tracker platform across every workspace."
        actions={
          <Link
            href="/host/tenants"
            className="inline-flex h-10 items-center justify-center gap-2 rounded-xl bg-primary px-4 text-sm font-medium text-primary-foreground transition-all hover:brightness-[1.04]"
          >
            <Building2 className="h-4 w-4" /> Manage tenants <ArrowRight className="h-4 w-4" />
          </Link>
        }
      />

      {loading && live === null && (
        <div className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">Loading platform metrics…</div>
      )}
      {error && live === null && (
        <div className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">
          Live metrics unavailable right now — showing demo data.
        </div>
      )}

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">
        <StatCard label="Monthly recurring revenue" value={usd(o.mrr)} icon={DollarSign} accent={brand.primary} sub={`${usd(o.mrr * 12)} ARR`} />
        <StatCard label="Total tenants" value={String(o.tenants)} icon={Building2} accent={brand.info} sub={`${o.activeSubs} active · ${o.trialing} trialing`} />
        <StatCard label="Paid seats" value={o.paidSeats.toLocaleString()} icon={Users} accent={brand.success} sub="occupied across tenants" />
        <StatCard label="Trials in progress" value={String(o.trialing)} icon={Sparkles} accent={brand.warning} sub={o.trialing > 0 ? "converting soon" : "none active"} />
        <StatCard label="Total invoiced" value={usd(o.totalInvoiced)} icon={Receipt} accent={brand.ink} sub="all-time, all tenants" />
        <StatCard label="Pending invoices" value={String(o.pendingInvoices)} icon={CheckCircle2} accent={brand.pink} sub={o.pendingInvoices > 0 ? "awaiting payment" : "all settled"} />
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <Card className="lg:col-span-1">
          <CardHeader><CardTitle>Tenants by plan</CardTitle></CardHeader>
          <CardContent>
            <ProjectDonut data={donut} />
            <div className="mt-3 space-y-2">
              {o.plans.map((p) => (
                <div key={p.name} className="flex items-center justify-between text-sm">
                  <span className="flex items-center gap-2">
                    <span className="h-2.5 w-2.5 rounded-full" style={{ background: p.color }} />
                    <span className="text-muted-foreground">{p.name}</span>
                  </span>
                  <span className="font-medium">{p.subscribers}</span>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>

        <Card className="lg:col-span-2">
          <CardHeader><CardTitle>Manage the platform</CardTitle></CardHeader>
          <CardContent className="grid grid-cols-1 gap-3 sm:grid-cols-3">
            {[
              { href: "/host/tenants", label: "Tenants", icon: Building2 },
              { href: "/host/billing", label: "Billing", icon: DollarSign },
              { href: "/host/users", label: "Users", icon: Users },
            ].map(({ href, label, icon: Icon }) => (
              <Link key={href} href={href} className="flex items-center gap-3 rounded-xl border border-border p-3 transition-colors hover:bg-muted/60">
                <span className="flex h-9 w-9 items-center justify-center rounded-lg bg-accent text-accent-foreground"><Icon className="h-4 w-4" /></span>
                <span className="text-sm font-medium">{label}</span>
                <ArrowRight className="ml-auto h-4 w-4 text-muted-foreground" />
              </Link>
            ))}
          </CardContent>
        </Card>
      </div>
    </PageStack>
  );
}
