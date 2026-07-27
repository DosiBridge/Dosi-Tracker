"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { DollarSign, TrendingUp, AlertTriangle, Download, Landmark } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { PageHeader, PageStack, SectionLabel } from "@/components/ui/page-header";
import { DataTable, type Column } from "@/components/ui/data-table";
import { StatCard } from "@/components/dashboard/stat-card";
import { toast } from "@/components/toast";
import { useSession } from "@/components/session-provider";
import { brand } from "@/lib/brand";
import { platformInvoices, platformOverview } from "@/lib/host-data";
import { getApi } from "@/hooks/useApi";

const usd = (n: number) => `$${Math.round(n).toLocaleString()}`;

/** Unified invoice row for live (API) + demo (fallback) views. */
interface Row {
  id: string;
  tenant: string;
  amount: number;
  status: string;
  date: string;
}

interface ApiInvoice {
  id: string;
  tenantName?: string | null;
  amount: number;
  status: string;
  dueDate: string;
}

const statusTone = (s: string): "success" | "warning" | "muted" =>
  s === "paid" ? "success" : s === "pending" || s === "due" ? "warning" : "muted";

export default function HostBillingPage() {
  const { workspaces } = useSession();

  const [live, setLive] = useState<{ rows: Row[]; mrr: number } | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);

  const loadLive = useCallback(async () => {
    setLoading(true);
    setError(false);
    try {
      const [overview, invoices] = await Promise.all([
        getApi("/api/app/platform/overview") as Promise<{ mrr?: number }>,
        getApi("/api/app/platform/invoices?MaxResultCount=500") as Promise<{ items?: ApiInvoice[] }>,
      ]);
      const items = Array.isArray(invoices?.items) ? invoices.items : [];
      setLive({
        mrr: overview?.mrr ?? 0,
        rows: items.map((i) => ({
          id: i.id,
          tenant: i.tenantName || "Host",
          amount: i.amount,
          status: i.status,
          date: (i.dueDate ?? "").slice(0, 10),
        })),
      });
    } catch {
      setLive(null); // fall back to demo
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
      platformInvoices(workspaces).map((i) => ({
        id: i.id,
        tenant: i.workspaceName,
        amount: i.amount,
        status: i.status === "due" ? "pending" : i.status === "upcoming" ? "upcoming" : "paid",
        date: i.date,
      })),
    [workspaces]
  );
  const mockMrr = useMemo(() => platformOverview(workspaces).mrr, [workspaces]);

  const rows = live?.rows ?? mockRows;
  const mrr = live?.mrr ?? mockMrr;
  const collected = rows.filter((i) => i.status === "paid").reduce((s, i) => s + i.amount, 0);
  const outstanding = rows.filter((i) => i.status === "pending").reduce((s, i) => s + i.amount, 0);

  function exportCsv() {
    const header = ["Invoice", "Tenant", "Date", "Amount", "Status"];
    const lines = rows.map((i) => [i.id, i.tenant, i.date, i.amount, i.status].join(","));
    const blob = new Blob([[header.join(","), ...lines].join("\n")], { type: "text/csv" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "platform-invoices.csv";
    a.click();
    URL.revokeObjectURL(url);
    toast({ title: "Export ready", description: "Platform invoices downloaded.", tone: "success" });
  }

  const columns: Column<Row>[] = [
    { key: "date", header: "Date", sortValue: (i) => i.date, render: (i) => <span className="text-muted-foreground">{i.date}</span> },
    { key: "tenant", header: "Tenant", sortValue: (i) => i.tenant.toLowerCase(), render: (i) => <span className="font-medium">{i.tenant}</span> },
    { key: "amount", header: "Amount", align: "right", sortValue: (i) => i.amount, render: (i) => <span className="font-medium">{usd(i.amount)}</span> },
    { key: "status", header: "Status", align: "right", sortValue: (i) => i.status, render: (i) => <Badge tone={statusTone(i.status)} className="capitalize">{i.status}</Badge> },
  ];

  return (
    <PageStack>
      <PageHeader
        eyebrow="Host"
        title="Platform Billing"
        description="Recurring revenue and invoices across all tenants."
        actions={
          <Button variant="outline" onClick={exportCsv}>
            <Download className="h-4 w-4" /> Export invoices
          </Button>
        }
      />

      {loading && live === null && (
        <div className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">Loading billing…</div>
      )}
      {error && live === null && (
        <div className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">
          Live billing unavailable right now — showing demo data.
        </div>
      )}

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <StatCard label="MRR" value={usd(mrr)} icon={DollarSign} accent={brand.primary} sub={`${usd(mrr * 12)} ARR`} />
        <StatCard label="Collected" value={usd(collected)} icon={TrendingUp} accent={brand.success} sub="paid invoices" />
        <StatCard label="Outstanding" value={usd(outstanding)} icon={AlertTriangle} accent={brand.danger} sub="pending invoices" />
        <StatCard label="Invoices" value={String(rows.length)} icon={Landmark} accent={brand.info} sub="all tenants" />
      </div>

      <SectionLabel>All invoices</SectionLabel>
      <Card className="p-2">
        <DataTable columns={columns} rows={rows} initialSort={{ key: "date", dir: "desc" }} emptyText="No invoices yet." />
      </Card>
    </PageStack>
  );
}
