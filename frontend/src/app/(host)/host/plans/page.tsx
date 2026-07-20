"use client";

import { useMemo, useState } from "react";
import { Check, Star, Pencil } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { DataTable, type Column } from "@/components/ui/data-table";
import { toast } from "@/components/toast";
import { useSession } from "@/components/session-provider";
import { planBreakdown, planColor, type PlanBreakdown } from "@/lib/host-data";
import { fmtLimit, plans } from "@/lib/saas-data";

const usd = (n: number) => `$${Math.round(n).toLocaleString()}`;

export default function HostPlansPage() {
  const { workspaces } = useSession();

  // Local (demo) price overrides — a real host would persist these.
  const [prices, setPrices] = useState<Record<string, number | null>>({});
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  const breakdown = useMemo(() => planBreakdown(workspaces), [workspaces, prices]);

  function priceOf(b: PlanBreakdown): number | null {
    return b.plan.id in prices ? prices[b.plan.id] : b.plan.pricePerUser;
  }

  function savePrice(id: string) {
    const n = Number(draft);
    if (!Number.isNaN(n)) {
      setPrices((p) => ({ ...p, [id]: n }));
      const plan = plans.find((p) => p.id === id);
      if (plan) {
        plan.pricePerUser = n;
      }
      toast({ title: "Plan updated", description: `New price: $${n}/user/mo.`, tone: "success" });
    }
    setEditing(null);
  }

  const totalMrr = breakdown.reduce((s, b) => s + b.mrr, 0);

  const matrixCols: Column<PlanBreakdown>[] = [
    { key: "plan", header: "Plan", render: (b) => (
      <span className="flex items-center gap-2 font-medium">
        <span className="h-2.5 w-2.5 rounded-full" style={{ background: b.color }} />
        {b.plan.name}
      </span>
    ) },
    { key: "seats", header: "Seat limit", align: "right", render: (b) => fmtLimit(b.plan.seats) },
    { key: "projects", header: "Projects", align: "right", render: (b) => fmtLimit(b.plan.projects) },
    { key: "storage", header: "Storage", align: "right", render: (b) => `${fmtLimit(b.plan.storageGb)} GB` },
    { key: "retention", header: "Retention", align: "right", render: (b) => `${b.plan.retentionDays} days` },
    { key: "tenants", header: "Tenants", align: "right", sortValue: (b) => b.tenants, render: (b) => b.tenants },
    { key: "mrr", header: "MRR", align: "right", sortValue: (b) => b.mrr, render: (b) => <span className="font-medium">{usd(b.mrr)}</span> },
  ];

  return (
    <PageStack>
      <PageHeader
        eyebrow="Host"
        title="Plans & Editions"
        description={`Pricing tiers, limits and adoption · ${usd(totalMrr)} total MRR.`}
      />

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {breakdown.map((b) => {
          const price = priceOf(b);
          return (
            <Card key={b.plan.id} variant={b.plan.highlight ? "interactive" : "default"} className={b.plan.highlight ? "border-primary/40 ring-1 ring-primary/20" : ""}>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <span className="h-2.5 w-2.5 rounded-full" style={{ background: b.color }} />
                  {b.plan.name}
                </CardTitle>
                {b.plan.highlight && <Badge tone="primary" className="gap-1"><Star className="h-3 w-3" /> Popular</Badge>}
              </CardHeader>
              <CardContent className="space-y-4">
                <div>
                  {editing === b.plan.id ? (
                    <div className="flex items-center gap-2">
                      <Input value={draft} onChange={(e) => setDraft(e.target.value)} className="h-8 w-20" autoFocus />
                      <Button size="sm" onClick={() => savePrice(b.plan.id)}>Save</Button>
                    </div>
                  ) : (
                    <div className="flex items-baseline gap-1">
                      <span className="font-display text-3xl font-bold tracking-tight">{price === null ? "Custom" : `$${price}`}</span>
                      {price !== null && <span className="text-sm text-muted-foreground">/user/mo</span>}
                      {price !== null && (
                        <button
                          onClick={() => { setEditing(b.plan.id); setDraft(String(price)); }}
                          className="ml-1 text-muted-foreground hover:text-foreground"
                          title="Edit price"
                        >
                          <Pencil className="h-3.5 w-3.5" />
                        </button>
                      )}
                    </div>
                  )}
                  <p className="mt-1 text-xs text-muted-foreground">{b.plan.tagline}</p>
                </div>

                <div className="grid grid-cols-3 gap-2 rounded-xl border border-border p-2 text-center">
                  <Stat label="Tenants" value={String(b.tenants)} />
                  <Stat label="Seats" value={String(b.seats)} />
                  <Stat label="MRR" value={usd(b.mrr)} />
                </div>

                <ul className="space-y-1.5">
                  {b.plan.features.slice(0, 4).map((f) => (
                    <li key={f} className="flex items-start gap-2 text-xs text-muted-foreground">
                      <Check className="mt-0.5 h-3.5 w-3.5 shrink-0 text-success" /> {f}
                    </li>
                  ))}
                </ul>
              </CardContent>
            </Card>
          );
        })}
      </div>

      <Card className="p-2">
        <div className="p-3 pb-1">
          <h2 className="text-sm font-semibold">Limits &amp; adoption</h2>
        </div>
        <DataTable columns={matrixCols} rows={breakdown} initialSort={{ key: "mrr", dir: "desc" }} />
      </Card>

      <Card>
        <CardHeader><CardTitle>Revenue by plan</CardTitle></CardHeader>
        <CardContent className="space-y-3">
          {breakdown.map((b) => (
            <div key={b.plan.id}>
              <div className="mb-1 flex items-center justify-between text-sm">
                <span className="flex items-center gap-2">
                  <span className="h-2.5 w-2.5 rounded-full" style={{ background: b.color }} />
                  {b.plan.name}
                </span>
                <span className="text-muted-foreground">{usd(b.mrr)}</span>
              </div>
              <div className="h-2 overflow-hidden rounded-full bg-muted">
                <div className="h-full rounded-full" style={{ width: `${totalMrr ? (b.mrr / totalMrr) * 100 : 0}%`, background: planColor(b.plan.id) }} />
              </div>
            </div>
          ))}
        </CardContent>
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
