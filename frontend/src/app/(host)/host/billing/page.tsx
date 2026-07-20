"use client";

import { useMemo } from "react";
import { DollarSign, TrendingUp, Receipt, AlertTriangle, Download } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { PageHeader, PageStack, SectionLabel } from "@/components/ui/page-header";
import { DataTable, type Column } from "@/components/ui/data-table";
import { StatCard } from "@/components/dashboard/stat-card";
import { MrrChart } from "@/components/host/host-charts";
import { toast } from "@/components/toast";
import { useSession } from "@/components/session-provider";
import { brand } from "@/lib/brand";
import { growthTrend, platformInvoices, platformOverview, type PlatformInvoice } from "@/lib/host-data";

const usd = (n: number) => `$${Math.round(n).toLocaleString()}`;

export default function HostBillingPage() {
  const { workspaces } = useSession();
  const overview = useMemo(() => platformOverview(workspaces), [workspaces]);
  const invoices = useMemo(() => platformInvoices(workspaces), [workspaces]);
  const growth = useMemo(() => growthTrend(workspaces), [workspaces]);

  const collected = invoices.filter((i) => i.status === "paid").reduce((s, i) => s + i.amount, 0);
  const outstanding = invoices.filter((i) => i.status === "due").reduce((s, i) => s + i.amount, 0);
  const upcoming = invoices.filter((i) => i.status === "upcoming").reduce((s, i) => s + i.amount, 0);

  function exportCsv() {
    const header = ["Invoice", "Tenant", "Date", "Plan", "Amount", "Status"];
    const lines = invoices.map((i) => [i.id, i.workspaceName, i.date, i.plan, i.amount, i.status].join(","));
    const blob = new Blob([[header.join(","), ...lines].join("\n")], { type: "text/csv" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "platform-invoices.csv";
    a.click();
    URL.revokeObjectURL(url);
    toast({ title: "Export ready", description: "Platform invoices downloaded.", tone: "success" });
  }

  const columns: Column<PlatformInvoice>[] = [
    { key: "date", header: "Date", sortValue: (i) => i.date, render: (i) => <span className="text-muted-foreground">{i.date}</span> },
    {
      key: "tenant",
      header: "Tenant",
      sortValue: (i) => i.workspaceName.toLowerCase(),
      render: (i) => (
        <span className="inline-flex items-center gap-1.5 font-medium">
          <span className="h-2.5 w-2.5 rounded-full" style={{ background: i.workspaceColor }} />
          {i.workspaceName}
        </span>
      ),
    },
    { key: "plan", header: "Plan", render: (i) => i.plan },
    { key: "amount", header: "Amount", align: "right", sortValue: (i) => i.amount, render: (i) => <span className="font-medium">{usd(i.amount)}</span> },
    {
      key: "status",
      header: "Status",
      align: "right",
      sortValue: (i) => i.status,
      render: (i) => <Badge tone={i.status === "paid" ? "success" : i.status === "due" ? "warning" : "muted"}>{i.status}</Badge>,
    },
  ];

  return (
    <PageStack>
      <PageHeader
        eyebrow="Host"
        title="Platform Billing"
        description="Revenue, invoices and dunning across all tenants."
        actions={
          <Button variant="outline" onClick={exportCsv}>
            <Download className="h-4 w-4" /> Export invoices
          </Button>
        }
      />

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <StatCard label="MRR" value={usd(overview.mrr)} icon={DollarSign} trend={14} accent={brand.primary} sub={`${usd(overview.arr)} ARR`} />
        <StatCard label="Collected (recent)" value={usd(collected)} icon={TrendingUp} trend={9} accent={brand.success} sub="paid invoices" />
        <StatCard label="Upcoming" value={usd(upcoming)} icon={Receipt} accent={brand.info} sub="trial conversions" />
        <StatCard label="Outstanding" value={usd(outstanding)} icon={AlertTriangle} accent={brand.danger} sub={overview.pastDue > 0 ? `${overview.pastDue} suspended` : "all current"} />
      </div>

      <Card>
        <CardHeader><CardTitle>MRR trend</CardTitle><Badge tone="muted">Last 12 months</Badge></CardHeader>
        <CardContent><MrrChart data={growth} /></CardContent>
      </Card>

      <SectionLabel>All invoices</SectionLabel>
      <Card className="p-2">
        <div className="flex items-center justify-between px-3 pb-1 pt-2">
          <span className="text-xs text-muted-foreground">{invoices.length} records</span>
        </div>
        <DataTable columns={columns} rows={invoices} initialSort={{ key: "date", dir: "desc" }} emptyText="No invoices yet." />
      </Card>
    </PageStack>
  );
}
