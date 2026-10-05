"use client";

import { useCallback, useEffect, useId, useMemo, useState } from "react";
import { Users, Clock, Plus, Pencil, Trash2 } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Modal } from "@/components/ui/modal";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { DataTable, type Column } from "@/components/ui/data-table";
import { requestErrorCode, requestErrorText } from "@/components/layout/request-error";
import { toast } from "@/components/toast";
import { useSession } from "@/components/session-provider";
import { planBreakdown } from "@/lib/host-data";
import { colorFromString } from "@/lib/utils";
import { delApi, getApi, postApi, putApi } from "@/hooks/useApi";
import { asItems } from "@/lib/live-dataset";
import type { CreateUpdatePlanDto, PlanDto } from "@/lib/api-contract";

const usd = (n: number) => `$${Math.round(n * 100) / 100}`;

/** POST (host): create a plan. */
const PLAN_CREATE_ROUTE = "/api/app/platform/plan";
/** PUT / DELETE (host): one plan (ABP puts the id before the action name). */
const planRoute = (planId: string) => `/api/app/platform/${encodeURIComponent(planId)}/plan`;

/** The server's limits for a plan (CreateUpdatePlanDto). */
const LIMITS = { name: 64, price: 100_000, seats: 100_000, trialDays: 365 } as const;

/** Unified plan row for live (API) + demo (fallback) views. */
interface Row {
  id: string;
  name: string;
  /** Per user per month; null = custom pricing (demo Enterprise). */
  pricePerUser: number | null;
  /** 0 = unlimited (demo only). */
  maxSeats: number;
  trialDays: number;
  subscribers: number;
  color: string;
}

interface ApiPlanSubs {
  planId: string;
  subscriberCount: number;
}

interface PlanForm {
  name: string;
  price: string;
  seats: string;
  trial: string;
}

type PlanFormErrors = Partial<Record<keyof PlanForm, string>>;

function wholeNumber(text: string): number | null {
  return /^\d+$/.test(text.trim()) ? Number(text.trim()) : null;
}

/** The form as the request body, or the field errors the server would return. */
function readPlanForm(form: PlanForm): { dto: CreateUpdatePlanDto | null; errors: PlanFormErrors } {
  const errors: PlanFormErrors = {};
  const name = form.name.trim();
  if (!name) errors.name = "Give the plan a name.";
  else if (name.length > LIMITS.name) errors.name = `Use ${LIMITS.name} characters or fewer.`;
  const price = /^\d+(\.\d{1,2})?$/.test(form.price.trim()) ? Number(form.price.trim()) : null;
  if (price === null || price > LIMITS.price) errors.price = `A price from 0 to ${LIMITS.price.toLocaleString("en")}, with at most two decimals.`;
  const seats = wholeNumber(form.seats);
  if (seats === null || seats < 1 || seats > LIMITS.seats) errors.seats = `A whole number from 1 to ${LIMITS.seats.toLocaleString("en")}.`;
  const trial = wholeNumber(form.trial);
  if (trial === null || trial > LIMITS.trialDays) errors.trial = `A whole number of days from 0 to ${LIMITS.trialDays}.`;
  if (Object.keys(errors).length > 0) return { dto: null, errors };
  return { dto: { name, pricePerUser: price as number, maxSeats: seats as number, trialDays: trial as number }, errors };
}

export default function HostPlansPage() {
  const { workspaces, isLive } = useSession();

  const [live, setLive] = useState<Row[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);
  const [editing, setEditing] = useState<Row | "new" | null>(null);
  const [deleting, setDeleting] = useState<Row | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  // Demo platform: plan changes made on this page (until the next reload).
  const [demoRows, setDemoRows] = useState<Row[] | null>(null);

  const loadLive = useCallback(async () => {
    setLoading(true);
    setError(false);
    try {
      const [plansRes, overview] = await Promise.all([
        getApi("/api/app/platform/plans"),
        getApi("/api/app/platform/overview") as Promise<{ plans?: ApiPlanSubs[] }>,
      ]);
      // getApi already unwrapped `items` — read the rows, not `.items` again.
      const plans = asItems<PlanDto>(plansRes);
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
        })),
      );
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
      planBreakdown(workspaces).map((b) => ({
        id: b.plan.id,
        name: b.plan.name,
        pricePerUser: b.plan.pricePerUser,
        maxSeats: typeof b.plan.seats === "number" && Number.isFinite(b.plan.seats) ? b.plan.seats : 0,
        trialDays: 0,
        subscribers: b.tenants,
        color: b.color,
      })),
    [workspaces],
  );

  // Live: the backend's plans only; a failed load shows an error, never demo plans.
  const rows = isLive ? (live ?? []) : (demoRows ?? mockRows);

  async function savePlan(dto: CreateUpdatePlanDto, existing: Row | null) {
    if (!isLive) {
      const base = demoRows ?? mockRows;
      const next: Row = {
        id: existing?.id ?? `demo-plan-${base.length + 1}`,
        name: dto.name,
        pricePerUser: dto.pricePerUser,
        maxSeats: dto.maxSeats,
        trialDays: dto.trialDays,
        subscribers: existing?.subscribers ?? 0,
        color: existing?.color ?? colorFromString(dto.name),
      };
      setDemoRows(existing ? base.map((r) => (r.id === existing.id ? next : r)) : [...base, next]);
      toast({ tone: "success", title: existing ? "Plan updated" : "Plan created", description: "Demo platform — this change lasts until you reload." });
      return;
    }
    try {
      if (existing) await putApi(planRoute(existing.id), dto);
      else await postApi(PLAN_CREATE_ROUTE, dto);
    } catch (err) {
      throw new Error(requestErrorText(err, existing ? "Couldn't save the plan." : "Couldn't create the plan."));
    }
    toast({
      tone: "success",
      title: existing ? `${dto.name} updated` : `${dto.name} created`,
      description: existing ? "Tenants on the plan are billed at the new price from their next invoice." : "Tenants can now be moved to it.",
    });
    await loadLive();
  }

  async function deletePlan(row: Row) {
    if (!isLive) {
      setDemoRows((demoRows ?? mockRows).filter((r) => r.id !== row.id));
      setDeleting(null);
      toast({ title: `${row.name} deleted`, description: "Demo platform — this change lasts until you reload." });
      return;
    }
    setDeleteBusy(true);
    try {
      await delApi(planRoute(row.id));
      setDeleting(null);
      toast({ title: `${row.name} deleted` });
      await loadLive();
    } catch (err) {
      toast({
        tone: "danger",
        title: `Couldn't delete ${row.name}`,
        description:
          requestErrorCode(err) === "Tracker:PlanInUse"
            ? "Tenants are still on this plan. Move them to another plan first."
            : requestErrorText(err, "The plan wasn't deleted."),
      });
    } finally {
      setDeleteBusy(false);
    }
  }

  const priceText = (r: Row) => (r.pricePerUser === null ? "Custom" : r.pricePerUser === 0 ? "Free" : `${usd(r.pricePerUser)}/mo`);
  const columns: Column<Row>[] = [
    {
      key: "name",
      header: "Plan",
      sortValue: (b) => b.name.toLowerCase(),
      render: (b) => (
        <span className="flex items-center gap-2 font-medium">
          <span aria-hidden className="h-2.5 w-2.5 rounded-full" style={{ background: b.color }} /> {b.name}
        </span>
      ),
    },
    { key: "price", header: "Price / user", align: "right", sortValue: (b) => b.pricePerUser ?? Number.MAX_SAFE_INTEGER, render: priceText },
    { key: "seats", header: "Seat limit", align: "right", sortValue: (b) => b.maxSeats, render: (b) => (b.maxSeats > 0 ? b.maxSeats.toLocaleString() : "Unlimited") },
    { key: "trial", header: "Trial", align: "right", sortValue: (b) => b.trialDays, render: (b) => (b.trialDays > 0 ? `${b.trialDays} days` : "—") },
    { key: "subscribers", header: "Subscribers", align: "right", sortValue: (b) => b.subscribers, render: (b) => b.subscribers },
    {
      key: "actions",
      header: "",
      align: "right",
      render: (b) => (
        <div className="flex items-center justify-end gap-1">
          <Button size="icon" variant="ghost" className="h-8 w-8" onClick={() => setEditing(b)} aria-label={`Edit ${b.name}`} title="Edit plan">
            <Pencil className="h-4 w-4" />
          </Button>
          <Button
            size="icon"
            variant="ghost"
            className="h-8 w-8 text-muted-foreground hover:text-danger-on-tint"
            onClick={() => setDeleting(b)}
            disabled={b.subscribers > 0}
            aria-label={`Delete ${b.name}`}
            title={b.subscribers > 0 ? "Move its tenants to another plan before deleting it" : "Delete plan"}
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        </div>
      ),
    },
  ];

  return (
    <PageStack>
      <PageHeader
        eyebrow="Host"
        title="Plans & Editions"
        description="Pricing tiers and their live adoption across tenants."
        actions={
          <Button onClick={() => setEditing("new")} disabled={isLive && !live}>
            <Plus className="h-4 w-4" /> New plan
          </Button>
        }
      />

      {loading && live === null && (
        <div role="status" className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">
          Loading plans…
        </div>
      )}
      {error && live === null && (
        <div role="alert" className="flex items-center gap-3 rounded-lg border border-danger/30 bg-danger/5 px-3 py-2 text-xs text-danger-on-tint">
          <span className="flex-1">Couldn&apos;t load plans from the server.</span>
          <button onClick={() => void loadLive()} className="font-medium text-primary-strong hover:underline">
            Try again
          </button>
        </div>
      )}

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {rows.map((b) => (
          <Card key={b.id}>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <span aria-hidden className="h-2.5 w-2.5 rounded-full" style={{ background: b.color }} /> {b.name}
              </CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="flex items-baseline gap-1">
                <span className="font-display text-3xl font-bold tracking-tight">
                  {b.pricePerUser === null ? "Custom" : b.pricePerUser === 0 ? "Free" : usd(b.pricePerUser)}
                </span>
                {b.pricePerUser !== null && b.pricePerUser !== 0 && <span className="text-sm text-muted-foreground">/user/mo</span>}
              </div>
              <dl className="grid grid-cols-3 gap-2 rounded-xl border border-border p-2 text-center">
                <Stat label="Subscribers" value={String(b.subscribers)} />
                <Stat label="Seat limit" value={b.maxSeats > 0 ? String(b.maxSeats) : "∞"} />
                <Stat label="Trial" value={b.trialDays > 0 ? `${b.trialDays}d` : "—"} />
              </dl>
              <div className="flex flex-wrap gap-3 text-xs text-muted-foreground">
                <span className="inline-flex items-center gap-1">
                  <Users aria-hidden className="h-3.5 w-3.5" /> {b.subscribers} {b.subscribers === 1 ? "tenant" : "tenants"}
                </span>
                {b.trialDays > 0 && (
                  <span className="inline-flex items-center gap-1">
                    <Clock aria-hidden className="h-3.5 w-3.5" /> {b.trialDays}-day trial
                  </span>
                )}
              </div>
            </CardContent>
          </Card>
        ))}
      </div>

      <Card className="p-2">
        <div className="p-3 pb-1">
          <h2 className="text-sm font-semibold">All plans</h2>
          <p className="mt-0.5 text-xs text-muted-foreground">A plan with tenants on it can&apos;t be deleted — move them to another plan first.</p>
        </div>
        <DataTable columns={columns} rows={rows} initialSort={{ key: "price", dir: "asc" }} emptyText={isLive && loading ? "Loading plans…" : "No plans yet."} />
      </Card>

      {editing && (
        <PlanModal
          plan={editing === "new" ? null : editing}
          onClose={() => setEditing(null)}
          onSave={(dto) => savePlan(dto, editing === "new" ? null : editing)}
        />
      )}
      <ConfirmDialog
        open={!!deleting}
        onCancel={() => setDeleting(null)}
        onConfirm={() => deleting && void deletePlan(deleting)}
        title={deleting ? `Delete ${deleting.name}?` : "Delete plan?"}
        description="New workspaces can no longer choose it. Invoices already raised on it are kept."
        confirmLabel="Delete plan"
        busy={deleteBusy}
      />
    </PageStack>
  );
}

function PlanModal({ plan, onClose, onSave }: { plan: Row | null; onClose: () => void; onSave: (dto: CreateUpdatePlanDto) => Promise<void> }) {
  const id = useId();
  const [form, setForm] = useState<PlanForm>({
    name: plan?.name ?? "",
    price: plan?.pricePerUser != null ? String(plan.pricePerUser) : "",
    seats: plan && plan.maxSeats > 0 ? String(plan.maxSeats) : "",
    trial: plan ? String(plan.trialDays) : "0",
  });
  const [submitted, setSubmitted] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { dto, errors } = readPlanForm(form);
  const shown = submitted ? errors : {};

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setSubmitted(true);
    if (!dto) return;
    setBusy(true);
    setError(null);
    try {
      await onSave(dto);
      onClose();
    } catch (err) {
      setError(err instanceof Error && err.message ? err.message : "Couldn't save the plan.");
      setBusy(false);
    }
  }

  const field = (key: keyof PlanForm, label: string, props: React.InputHTMLAttributes<HTMLInputElement>, hint?: string) => (
    <div className="space-y-1.5">
      <label htmlFor={`${id}-${key}`} className="text-sm font-medium">
        {label}
      </label>
      <Input
        id={`${id}-${key}`}
        value={form[key]}
        onChange={(e) => setForm((f) => ({ ...f, [key]: e.target.value }))}
        aria-invalid={shown[key] ? true : undefined}
        aria-describedby={shown[key] ? `${id}-${key}-error` : hint ? `${id}-${key}-hint` : undefined}
        {...props}
      />
      {shown[key] ? (
        <p id={`${id}-${key}-error`} className="text-xs text-danger-on-tint">
          {shown[key]}
        </p>
      ) : hint ? (
        <p id={`${id}-${key}-hint`} className="text-xs text-muted-foreground">
          {hint}
        </p>
      ) : null}
    </div>
  );

  return (
    <Modal open onClose={onClose} title={plan ? `Edit ${plan.name}` : "New plan"} description="Prices are per user per month, in US dollars.">
      <form onSubmit={(e) => void submit(e)} className="space-y-4" noValidate>
        {field("name", "Name", { maxLength: LIMITS.name, autoComplete: "off" })}
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
          {field("price", "Price / user", { inputMode: "decimal", placeholder: "6" }, "0 for a free plan")}
          {field("seats", "Seat limit", { inputMode: "numeric", placeholder: "25" })}
          {field("trial", "Trial days", { inputMode: "numeric", placeholder: "14" })}
        </div>
        {plan && plan.subscribers > 0 && (
          <p className="rounded-lg bg-warning/10 px-3 py-2 text-xs text-warning-on-tint">
            {plan.subscribers} {plan.subscribers === 1 ? "tenant is" : "tenants are"} on this plan: a new price applies from their next invoice, and a lower seat
            limit stops new members once reached.
          </p>
        )}
        {error && (
          <p role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2 pt-1">
          <Button type="button" variant="ghost" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" disabled={busy}>
            {busy ? "Saving…" : plan ? "Save plan" : "Create plan"}
          </Button>
        </div>
      </form>
    </Modal>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  // The label comes first for assistive tech; the value is drawn above it.
  return (
    <div className="flex flex-col-reverse">
      <dt className="text-[10px] text-muted-foreground">{label}</dt>
      <dd className="text-sm font-bold">{value}</dd>
    </div>
  );
}
