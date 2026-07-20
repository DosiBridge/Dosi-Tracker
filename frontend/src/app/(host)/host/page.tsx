"use client";

import { useMemo } from "react";
import Link from "next/link";
import {
  DollarSign,
  Building2,
  Users,
  Gauge,
  HardDrive,
  ArrowRight,
  Sparkles,
  CircleDot,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { StatCard } from "@/components/dashboard/stat-card";
import { ProjectDonut } from "@/components/dashboard/charts";
import { MrrChart, TenantsChart } from "@/components/host/host-charts";
import { useSession } from "@/components/session-provider";
import { brand } from "@/lib/brand";
import {
  growthTrend,
  planBreakdown,
  planColor,
  platformOverview,
  tenantMetrics,
} from "@/lib/host-data";
import { statusLabel } from "@/lib/saas-data";

const usd = (n: number) => `$${Math.round(n).toLocaleString()}`;

const statusTone: Record<string, "success" | "warning" | "danger"> = {
  active: "success",
  trialing: "warning",
  past_due: "danger",
};

export default function HostOverviewPage() {
  const { workspaces } = useSession();
  const overview = useMemo(() => platformOverview(workspaces), [workspaces]);
  const growth = useMemo(() => growthTrend(workspaces), [workspaces]);
  const plansB = useMemo(() => planBreakdown(workspaces), [workspaces]);

  const donut = plansB
    .filter((p) => p.tenants > 0)
    .map((p) => ({ name: p.plan.name, value: p.tenants, color: p.color }));

  const recent = [...workspaces]
    .sort((a, b) => +new Date(b.createdAt) - +new Date(a.createdAt))
    .slice(0, 5);

  return (
    <PageStack>
      <PageHeader
        eyebrow="Host"
        title={
          <span className="inline-flex flex-wrap items-center gap-2">
            Platform Overview
            <Badge tone="success" className="gap-1.5 align-middle">
              <span className="h-1.5 w-1.5 rounded-full bg-success live-dot" />
              {overview.active} live tenants
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

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">
        <StatCard label="Monthly recurring revenue" value={usd(overview.mrr)} icon={DollarSign} trend={14} accent={brand.primary} sub={`${usd(overview.arr)} ARR`} />
        <StatCard label="Total tenants" value={String(overview.totalTenants)} icon={Building2} trend={9} accent={brand.info} sub={`${overview.newThisMonth} new this month`} />
        <StatCard label="Paid seats" value={overview.totalSeats.toLocaleString()} icon={Users} trend={6} accent={brand.success} sub={`${overview.totalMembers} members total`} />
        <StatCard label="Trials in progress" value={String(overview.trialing)} icon={Sparkles} accent={brand.warning} sub={overview.pastDue > 0 ? `${overview.pastDue} past due` : "no past-due accounts"} />
        <StatCard label="Avg productivity" value={`${overview.avgProductivity}%`} icon={Gauge} trend={-2} accent={brand.pink} sub="weighted across tenants" />
        <StatCard label="Storage used" value={`${overview.storageGb} GB`} icon={HardDrive} trend={11} accent={brand.ink} sub="all workspaces" />
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle>Revenue growth</CardTitle>
            <Badge tone="muted">Last 12 months</Badge>
          </CardHeader>
          <CardContent><MrrChart data={growth} /></CardContent>
        </Card>

        <Card>
          <CardHeader><CardTitle>Tenants by plan</CardTitle></CardHeader>
          <CardContent>
            <ProjectDonut data={donut} />
            <div className="mt-3 space-y-2">
              {plansB.map((p) => (
                <div key={p.plan.id} className="flex items-center justify-between text-sm">
                  <span className="flex items-center gap-2">
                    <span className="h-2.5 w-2.5 rounded-full" style={{ background: p.color }} />
                    <span className="text-muted-foreground">{p.plan.name}</span>
                  </span>
                  <span className="font-medium">{p.tenants} · {usd(p.mrr)}</span>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader><CardTitle>Tenant growth</CardTitle><Badge tone="muted">Cumulative</Badge></CardHeader>
          <CardContent><TenantsChart data={growth} /></CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Recent signups</CardTitle>
            <Link href="/host/tenants" className="text-xs text-primary hover:underline">View all</Link>
          </CardHeader>
          <CardContent className="space-y-1">
            {recent.map((ws) => {
              const m = tenantMetrics(ws);
              return (
                <Link
                  key={ws.id}
                  href="/host/tenants"
                  className="flex items-center gap-3 rounded-xl p-2 transition-colors hover:bg-muted/60"
                >
                  <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-sm font-bold text-white" style={{ background: ws.color }}>
                    {ws.name.charAt(0)}
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-sm font-medium">{ws.name}</div>
                    <div className="truncate text-xs text-muted-foreground">
                      {m.members} members · {m.projects} projects · joined {ws.createdAt}
                    </div>
                  </div>
                  <span className="flex items-center gap-1.5 text-xs">
                    <CircleDot className="h-3 w-3" style={{ color: planColor(ws.planId) }} />
                    <Badge tone={statusTone[ws.status]}>{statusLabel[ws.status]}</Badge>
                  </span>
                </Link>
              );
            })}
          </CardContent>
        </Card>
      </div>
    </PageStack>
  );
}
