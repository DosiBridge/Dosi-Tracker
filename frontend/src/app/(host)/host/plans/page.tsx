"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Users, Clock } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { DataTable, type Column } from "@/components/ui/data-table";
import { useSession } from "@/components/session-provider";
import { planBreakdown } from "@/lib/host-data";
import { colorFromString } from "@/lib/utils";
import { getApi } from "@/hooks/useApi";

const usd = (n: number) => `$${Math.round(n).toLocaleString()}`;

/** Unified plan row for live (API) + demo (fallback) views. */
interface Row {
  id: string;
  name: string;
  pricePerUser: number;
  maxSeats: number;
  trialDays: number;
  subscribers: number;
  color: string;
}

interface ApiPlan {
  id: string;
  name: string;
  pricePerUser: number;
  maxSeats: number;
  trialDays: number;
}
interface ApiPlanSubs {
  planId: string;
  subscriberCount: number;
}

export default function HostPlansPage() {
  const { workspaces } = useSession();

  const [live, setLive] = useState<Row[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);

  const loadLive = useCallback(async () => {
    setLoading(true);
    setError(false);
    try {
      const [plansRes, overview] = await Promise.all([
        getApi("/api/app/platform/plans") as Promise<{ items?: ApiPlan[] }>,
        getApi("/api/app/platform/overview") as Promise<{ plans?: ApiPlanSubs[] }>,
      ]);
      const plans = Array.isArray(plansRes?.items) ? plansRes.items : [];
      const subs = new Map((overview?.plans ?? []).map((p) => [p.planId, p.subscriberCount]));
      setLive(
        plans.map((p) => ({
          id: p.id,
          name: p.name,
          pricePerUser: p.pricePerUser,
          maxSeats: p.maxSeats,
          trialDays: p.trialDays,
          subscribers: subs.get(p.id) ?? 0,
          color: colorFromString(p.name),
        }))
      );
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

  const mockRows = useMemo<Row[]>(
    () =>
      planBreakdown(workspaces).map((b) => ({
        id: b.plan.id,
        name: b.plan.name,
        pricePerUser: b.plan.pricePerUser ?? 0,
        maxSeats: typeof b.plan.seats === "number" ? b.plan.seats : 0,
        trialDays: 0,
        subscribers: b.tenants,
        color: b.color,
      })),
    [workspaces]
  );

  const rows = live ?? mockRows;

  const columns: Column<Row>[] = [
    { key: "name", header: "Plan", render: (b) => (
      <span className="flex items-center gap-2 font-medium">
        <span className="h-2.5 w-2.5 rounded-full" style={{ background: b.color }} /> {b.name}
      </span>
    ) },
    { key: "price", header: "Price / user", align: "right", sortValue: (b) => b.pricePerUser, render: (b) => (b.pricePerUser === 0 ? "Free" : `${usd(b.pricePerUser)}/mo`) },
    { key: "seats", header: "Seat limit", align: "right", sortValue: (b) => b.maxSeats, render: (b) => (b.maxSeats > 0 ? b.maxSeats : "—") },
    { key: "trial", header: "Trial", align: "right", sortValue: (b) => b.trialDays, render: (b) => (b.trialDays > 0 ? `${b.trialDays} days` : "—") },
    { key: "subscribers", header: "Subscribers", align: "right", sortValue: (b) => b.subscribers, render: (b) => b.subscribers },
  ];

  return (
    <PageStack>
      <PageHeader
        eyebrow="Host"
        title="Plans & Editions"
        description="Pricing tiers and their live adoption across tenants."
      />

      {loading && live === null && (
        <div className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">Loading plans…</div>
      )}
      {error && live === null && (
        <div className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">
          Live plans unavailable right now — showing demo data.
        </div>
      )}

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {rows.map((b) => (
          <Card key={b.id}>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <span className="h-2.5 w-2.5 rounded-full" style={{ background: b.color }} /> {b.name}
              </CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="flex items-baseline gap-1">
                <span className="font-display text-3xl font-bold tracking-tight">{b.pricePerUser === 0 ? "Free" : `$${b.pricePerUser}`}</span>
                {b.pricePerUser !== 0 && <span className="text-sm text-muted-foreground">/user/mo</span>}
              </div>
              <div className="grid grid-cols-3 gap-2 rounded-xl border border-border p-2 text-center">
                <Stat label="Subscribers" value={String(b.subscribers)} />
                <Stat label="Seat limit" value={b.maxSeats > 0 ? String(b.maxSeats) : "—"} />
                <Stat label="Trial" value={b.trialDays > 0 ? `${b.trialDays}d` : "—"} />
              </div>
              <div className="flex flex-wrap gap-3 text-xs text-muted-foreground">
                <span className="inline-flex items-center gap-1"><Users className="h-3.5 w-3.5" /> {b.subscribers} tenants</span>
                {b.trialDays > 0 && <span className="inline-flex items-center gap-1"><Clock className="h-3.5 w-3.5" /> {b.trialDays}-day trial</span>}
              </div>
            </CardContent>
          </Card>
        ))}
      </div>

      <Card className="p-2">
        <div className="p-3 pb-1">
          <h2 className="text-sm font-semibold">All plans</h2>
          <Badge tone="muted" className="mt-1">Read-only — editing arrives with the plan-management API</Badge>
        </div>
        <DataTable columns={columns} rows={rows} initialSort={{ key: "price", dir: "asc" }} />
      </Card>
    </PageStack>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-sm font-bold">{value}</div>
      <div className="text-[10px] text-muted-foreground">{label}</div>
    </div>
  );
}
