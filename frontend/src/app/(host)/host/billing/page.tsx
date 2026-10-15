"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { DollarSign, TrendingUp, AlertTriangle, Download, Landmark, CheckCircle2 } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { PageHeader, PageStack, SectionLabel } from "@/components/ui/page-header";
import { DataTable, type Column } from "@/components/ui/data-table";
import { StatCard } from "@/components/dashboard/stat-card";
import { requestErrorText } from "@/components/layout/request-error";
import { toast } from "@/components/toast";
import { useSession } from "@/components/session-provider";
import { brand } from "@/lib/brand";
import { platformInvoices, platformOverview } from "@/lib/host-data";
import { exportRecords } from "@/lib/export";
import { getApi, markInvoicePaid } from "@/hooks/useApi";
import { asItems, normalizeUtcIso } from "@/lib/live-dataset";
import type { PlatformInvoiceDto } from "@/lib/api-contract";

const usd = (n: number) => `$${Math.round(n).toLocaleString()}`;

/** Unified invoice row for live (API) + demo (fallback) views. */
interface Row {
  id: string;
  tenant: string;
  amount: number;
  status: string;
  /** Due date, "YYYY-MM-DD". */
  date: string;
  /** "Oct 2026" — the billed month, when the server says (newer servers). */
  period: string | null;
}

const statusTone = (s: string): "success" | "warning" | "danger" | "muted" =>
  s === "paid" ? "success" : s === "failed" ? "danger" : s === "pending" || s === "due" ? "warning" : "muted";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"] as const;

/** "Oct 2026" for the start of a billing period (a UTC month start), the same in every browser. */
function periodLabel(iso: string | null | undefined): string | null {
  const at = normalizeUtcIso(iso);
  if (!at) return null;
  const d = new Date(at);
  return `${MONTHS[d.getUTCMonth()]} ${d.getUTCFullYear()}`;
}

export default function HostBillingPage() {
  const { workspaces, isLive } = useSession();

  const [live, setLive] = useState<{ rows: Row[]; mrr: number } | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);
  const [paying, setPaying] = useState<Row | null>(null);
  const [payBusy, setPayBusy] = useState(false);
  // Demo platform: invoices marked paid on this page (until the next reload).
  const [demoPaid, setDemoPaid] = useState<string[]>([]);

  const loadLive = useCallback(async () => {
    setLoading(true);
    setError(false);
    try {
      const [overview, invoices] = await Promise.all([
        getApi("/api/app/platform/overview") as Promise<{ mrr?: number }>,
        getApi("/api/app/platform/invoices?MaxResultCount=1000"),
      ]);
      // getApi already unwrapped `items` — read the rows, not `.items` again.
      const items = asItems<PlatformInvoiceDto>(invoices);
      setLive({
        mrr: overview?.mrr ?? 0,
        rows: items.map((i) => ({
          id: i.id,
          tenant: i.workspaceName || "Deleted workspace",
          amount: Number(i.amount) || 0,
          status: i.status,
          date: (i.dueDate ?? "").slice(0, 10),
          period: periodLabel(i.billingPeriodStart),
        })),
      });
    } catch {
      setLive(null);
      setError(true);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!isLive) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    loadLive();
  }, [loadLive, isLive]);

  const mockRows = useMemo<Row[]>(
    () =>
      platformInvoices(workspaces).map((i) => ({
        id: i.id,
        tenant: i.workspaceName,
        amount: i.amount,
        status: demoPaid.includes(i.id) ? "paid" : i.status === "due" ? "pending" : i.status === "upcoming" ? "upcoming" : "paid",
        date: i.date,
        period: null,
      })),
    [workspaces, demoPaid],
  );
  const mockMrr = useMemo(() => platformOverview(workspaces).mrr, [workspaces]);

  // Live: real invoices only; a failed load shows an error, never demo revenue.
  const rows = isLive ? (live?.rows ?? []) : mockRows;
  const mrr = isLive ? (live?.mrr ?? 0) : mockMrr;
  const collected = rows.filter((i) => i.status === "paid").reduce((s, i) => s + i.amount, 0);
  const outstanding = rows.filter((i) => i.status === "pending").reduce((s, i) => s + i.amount, 0);
  const hasPeriods = rows.some((r) => r.period);

  function exportCsv() {
    // Through lib/export: quoted cells and formula-safe text (a tenant can name itself "=HYPERLINK(…)").
    exportRecords("platform-invoices", rows, [
      { header: "Invoice", value: (r) => r.id },
      { header: "Tenant", value: (r) => r.tenant },
      { header: "Billing period", value: (r) => r.period ?? "" },
      { header: "Due date", value: (r) => r.date },
      { header: "Amount USD", value: (r) => r.amount },
      { header: "Status", value: (r) => r.status },
    ]);
    toast({ title: "Export ready", description: "Platform invoices downloaded.", tone: "success" });
  }

  async function confirmPaid(row: Row) {
    if (!isLive) {
      setDemoPaid((ids) => [...ids, row.id]);
      setPaying(null);
      toast({ tone: "success", title: "Invoice marked paid", description: "Demo platform — this change lasts until you reload." });
      return;
    }
    setPayBusy(true);
    try {
      await markInvoicePaid(row.id);
      setPaying(null);
      toast({ tone: "success", title: "Invoice marked paid", description: `${row.tenant} · ${usd(row.amount)}` });
      await loadLive();
    } catch (err) {
      toast({ tone: "danger", title: "Couldn't mark the invoice paid", description: requestErrorText(err, "The change wasn't saved.") });
    } finally {
      setPayBusy(false);
    }
  }

  const columns: Column<Row>[] = [
    { key: "date", header: "Due", sortValue: (i) => i.date, render: (i) => <span className="text-muted-foreground">{i.date}</span> },
    ...(hasPeriods
      ? [{ key: "period", header: "Period", sortValue: (i: Row) => i.date, render: (i: Row) => <span className="text-muted-foreground">{i.period ?? "—"}</span> }]
      : []),
    { key: "tenant", header: "Tenant", sortValue: (i) => i.tenant.toLowerCase(), render: (i) => <span className="font-medium">{i.tenant}</span> },
    { key: "amount", header: "Amount", align: "right", sortValue: (i) => i.amount, render: (i) => <span className="font-medium">{usd(i.amount)}</span> },
    {
      key: "status",
      header: "Status",
      align: "right",
      sortValue: (i) => i.status,
      render: (i) => (
        <Badge tone={statusTone(i.status)} className="capitalize">
          {i.status}
        </Badge>
      ),
    },
    {
      key: "actions",
      header: "",
      align: "right",
      render: (i) =>
        i.status === "pending" || i.status === "failed" ? (
          <Button size="sm" variant="outline" onClick={() => setPaying(i)} aria-label={`Mark ${i.tenant}'s ${usd(i.amount)} invoice due ${i.date} as paid`}>
            <CheckCircle2 className="h-3.5 w-3.5" /> Mark paid
          </Button>
        ) : null,
    },
  ];

  return (
    <PageStack>
      <PageHeader
        eyebrow="Host"
        title="Platform Billing"
        description="Recurring revenue and invoices across all tenants."
        actions={
          <Button variant="outline" onClick={exportCsv} disabled={rows.length === 0}>
            <Download className="h-4 w-4" /> Export invoices
          </Button>
        }
      />

      {loading && live === null && (
        <div role="status" className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">
          Loading billing…
        </div>
      )}
      {error && live === null && (
        <div role="alert" className="flex items-center gap-3 rounded-lg border border-danger/30 bg-danger/5 px-3 py-2 text-xs text-danger-on-tint">
          <span className="flex-1">Couldn&apos;t load billing from the server.</span>
          <button onClick={() => void loadLive()} className="font-medium text-primary-strong hover:underline">
            Try again
          </button>
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

      <ConfirmDialog
        open={!!paying}
        onCancel={() => setPaying(null)}
        onConfirm={() => paying && void confirmPaid(paying)}
        title="Mark this invoice paid?"
        description={
          paying
            ? `${paying.tenant} · ${usd(paying.amount)}${paying.period ? ` for ${paying.period}` : ""} · due ${paying.date}. Record it only once the payment has arrived.`
            : ""
        }
        confirmLabel="Mark paid"
        tone="primary"
        busy={payBusy}
      />
    </PageStack>
  );
}
