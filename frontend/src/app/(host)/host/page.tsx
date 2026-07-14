"use client";

import { useMemo } from "react";
import Link from "next/link";
import {
  DollarSign,
  TrendingUp,
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
import { StatCard } from "@/components/dashboard/stat-card";
import { ProjectDonut } from "@/components/dashboard/charts";
import { MrrChart, TenantsChart } from "@/components/host/host-charts";
import { useSession } from "@/components/session-provider";
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
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-2xl font-bold tracking-tight">Platform Overview</h1>
            <Badge tone="success" className="gap-1.5">
              <span className="h-1.5 w-1.5 rounded-full bg-success live-dot" />
              {overview.active} live tenants
            </Badge>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            Health of the whole Dosi-Tracker platform across every workspace.
          </p>
        </div>
        <Link
          href="/host/tenants"
          className="inline-flex h-10 items-center gap-2 rounded-lg bg-primary px-4 text-sm font-medium text-primary-foreground hover:opacity-90"
        >
          <Building2 className="h-4 w-4" /> Manage tenants <ArrowRight className="h-4 w-4" />
        </Link>
      </div>

      {/* KPIs */}
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">
        <StatCard label="Monthly recurring revenue" value={usd(overview.mrr)} icon={DollarSign} trend={14} accent="#6d5efc" sub={`${usd(overview.arr)} ARR`} />
        <StatCard label="Total tenants" value={String(overview.totalTenants)} icon={Building2} trend={9} accent="#0ea5e9" sub={`${overview.newThisMonth} new this month`} />
        <StatCard label="Paid seats" value={overview.totalSeats.toLocaleString()} icon={Users} trend={6} accent="#22c55e" sub={`${overview.totalMembers} members total`} />
        <StatCard label="Trials in progress" value={String(overview.trialing)} icon={Sparkles} accent="#f59e0b" sub={overview.pastDue > 0 ? `${overview.pastDue} past due` : "no past-due accounts"} />
        <StatCard label="Avg productivity" value={`${overview.avgProductivity}%`} icon={Gauge} trend={-2} accent="#ec4899" sub="weighted across tenants" />
        <StatCard label="Storage used" value={`${overview.storageGb} GB`} icon={HardDrive} trend={11} accent="#8b5cf6" sub="all workspaces" />
      </div>

      {/* MRR + plan mix */}
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

      {/* Tenant growth + recent signups */}
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
                  className="flex items-center gap-3 rounded-lg p-2 transition-colors hover:bg-muted/60"
                >
                  <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg text-sm font-bold text-white" style={{ background: ws.color }}>
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
    </div>
  );
}
