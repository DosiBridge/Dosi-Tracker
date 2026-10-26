"use client";

import { useEffect, useId, useMemo, useState } from "react";
import { Sparkles } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Progress } from "@/components/ui/progress";
import { Switch } from "@/components/ui/switch";
import { toast } from "@/components/toast";
import { useSession } from "@/components/session-provider";
import { requestErrorStatus, requestErrorText } from "@/components/layout/request-error";
import { getPlatformAiOverview, updatePlatformAiStatus } from "@/hooks/ai-api";
import type { PlatformAiOverviewDto, PlatformAiProviderState } from "@/lib/ai-contract";
import { aiMeter, formatRate, formatUsd } from "@/lib/ai-format";
import { demoPlatformAiOverview } from "@/lib/host-data";

/** What each provider state means, in the console's words: never a credential or a configuration key. */
const PROVIDER_STATE: Record<PlatformAiProviderState, { label: string; tone: "success" | "muted" | "warning" }> = {
  configured: { label: "Connected", tone: "success" },
  disabled: { label: "Turned off in the server configuration", tone: "muted" },
  "missing-api-key": { label: "The AI provider connection isn't set up", tone: "warning" },
  "missing-model": { label: "The AI provider connection names no model", tone: "warning" },
  "invalid-endpoint": { label: "The AI provider connection's address isn't valid", tone: "warning" },
  "unsupported-provider": { label: "This AI provider isn't supported", tone: "warning" },
};

const BREAKER: Record<PlatformAiOverviewDto["breakerState"], string> = {
  closed: "Normal",
  open: "Paused after repeated provider failures",
  "half-open": "Trying the provider again",
};

/** "1 Oct": the first day of the month the figures cover (a UTC instant). */
function monthStartLabel(iso: string): string | null {
  const at = Date.parse(iso);
  return Number.isFinite(at) ? new Intl.DateTimeFormat("en-GB", { timeZone: "UTC", day: "numeric", month: "short" }).format(at) : null;
}

const tokens = (n: number) => Math.max(0, Math.round(n)).toLocaleString("en-US");

function Figure({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="rounded-xl border border-border p-3">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="mt-0.5 text-lg font-semibold tabular-nums">{value}</dd>
      {sub && <dd className="text-[11px] text-muted-foreground">{sub}</dd>}
    </div>
  );
}

function Budget({ label, used, limit, format }: { label: string; used: number | null; limit: number | null; format: (n: number) => string }) {
  if (limit === null || used === null) return null;
  const meter = aiMeter(used, limit);
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between gap-3 text-xs">
        <span className="font-medium">{label}</span>
        <span className="tabular-nums text-muted-foreground">
          {format(used)} of {format(limit)}
        </span>
      </div>
      {meter.percent !== null && <Progress value={meter.percent} aria-label={`${label} used this month`} />}
    </div>
  );
}

/**
 * The host console's AI assist card (AI assist spec §3.1, §18.5): the
 * provider's state, whether AI is enabled in the configuration and at
 * runtime, the circuit breaker, this month's requests, tokens and estimated
 * cost against the budgets, the fallback and error rates, the workspaces
 * using it most, and the runtime switch — which stops every workspace's AI
 * requests on their next call, so it asks first. A demo shows a fixed sample
 * and changes nothing.
 */
export function PlatformAiCard() {
  const headingId = useId();
  const { isLive, workspaces } = useSession();
  const demo = useMemo(() => (isLive ? null : demoPlatformAiOverview(workspaces)), [isLive, workspaces]);
  const [loaded, setLoaded] = useState<PlatformAiOverviewDto | null>(null);
  const [demoEnabled, setDemoEnabled] = useState<boolean | null>(null);
  const [loadError, setLoadError] = useState<{ message: string; missing: boolean } | null>(null);
  const [reload, setReload] = useState(0);
  const [confirming, setConfirming] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isLive) return;
    let current = true;
    getPlatformAiOverview().then(
      (overview) => {
        if (!current) return;
        setLoaded(overview);
        setLoadError(null);
      },
      (err: unknown) => {
        if (!current) return;
        const missing = requestErrorStatus(err) === 404;
        setLoadError({ message: missing ? "This server has no AI assist." : requestErrorText(err, "Couldn't load AI assist's figures."), missing });
      },
    );
    return () => {
      current = false;
    };
  }, [isLive, reload]);

  const overview = isLive ? loaded : demo && { ...demo, runtimeEnabled: demoEnabled ?? demo.runtimeEnabled };

  async function apply(enabled: boolean) {
    setError(null);
    if (!isLive) {
      setDemoEnabled(enabled);
      setConfirming(null);
      toast({ tone: "info", title: "Demo platform", description: "Nothing was changed: the demo has no AI provider." });
      return;
    }
    setBusy(true);
    try {
      const saved = await updatePlatformAiStatus({ enabled });
      setLoaded((o) => (o ? { ...o, runtimeEnabled: saved?.enabled ?? enabled } : o));
      setConfirming(null);
      toast({
        tone: "success",
        title: enabled ? "AI assist is on for the platform" : "AI assist is off for the platform",
        description: enabled ? "Workspaces that opted in can use it again." : "Every workspace's AI requests stop on their next call.",
      });
    } catch (err) {
      setConfirming(null);
      setError(requestErrorText(err, "Couldn't change the platform's AI switch."));
    } finally {
      setBusy(false);
    }
  }

  const state = overview ? PROVIDER_STATE[overview.providerState] : null;
  const usedTokens = overview ? overview.inputTokens + overview.outputTokens : 0;
  const since = overview ? monthStartLabel(overview.monthStart) : null;

  return (
    <Card>
      <section aria-labelledby={headingId}>
        <CardHeader>
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <Sparkles aria-hidden className="h-4 w-4 shrink-0 text-primary" />
            <CardTitle id={headingId}>AI assist</CardTitle>
            {overview && (
              <Badge tone={overview.configEnabled && overview.runtimeEnabled ? "success" : "muted"}>
                {overview.configEnabled && overview.runtimeEnabled ? "On" : "Off"}
              </Badge>
            )}
            {!isLive && <Badge tone="muted">Sample</Badge>}
          </div>
        </CardHeader>
        <CardContent className="space-y-5">
          {isLive && !overview && !loadError && (
            <p role="status" className="text-sm text-muted-foreground">
              Loading AI assist&apos;s figures…
            </p>
          )}
          {loadError &&
            (loadError.missing ? (
              <p className="text-sm text-muted-foreground">{loadError.message}</p>
            ) : (
              <div role="alert" className="flex flex-wrap items-center gap-3 rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
                <span className="min-w-0 flex-1">{loadError.message}</span>
                <Button size="sm" variant="outline" onClick={() => setReload((n) => n + 1)}>
                  Try again
                </Button>
              </div>
            ))}
          {overview && state && (
            <>
              <dl className="grid grid-cols-1 gap-x-6 gap-y-2 text-sm sm:grid-cols-2">
                <div className="flex flex-wrap items-center gap-2">
                  <dt className="text-muted-foreground">Provider</dt>
                  <dd className="flex flex-wrap items-center gap-2 font-medium">
                    {overview.providerDisplayName ?? "None"}
                    <Badge tone={state.tone}>{state.label}</Badge>
                  </dd>
                </div>
                {overview.defaultModel && (
                  <div className="flex flex-wrap items-center gap-2">
                    <dt className="text-muted-foreground">Model</dt>
                    <dd className="font-medium">{overview.defaultModel}</dd>
                  </div>
                )}
                <div className="flex flex-wrap items-center gap-2">
                  <dt className="text-muted-foreground">Server configuration</dt>
                  <dd className="font-medium">{overview.configEnabled ? "Enabled" : "Disabled"}</dd>
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  <dt className="text-muted-foreground">Circuit breaker</dt>
                  <dd className="font-medium">{BREAKER[overview.breakerState]}</dd>
                </div>
              </dl>

              <div className="space-y-2 rounded-xl border border-border p-3">
                <Switch
                  checked={overview.runtimeEnabled}
                  onChange={(v) => setConfirming(v)}
                  disabled={busy}
                  label="Allow AI requests on this platform"
                  description={
                    overview.configEnabled
                      ? "Takes effect on each workspace's next request. Workspace settings and earlier results are kept."
                      : "Has no effect until AI assist is enabled in the server configuration."
                  }
                />
                {error && (
                  <p role="alert" className="text-xs text-danger-on-tint">
                    {error}
                  </p>
                )}
              </div>

              <div>
                <h3 className="text-sm font-semibold">This month{since ? `, since ${since}` : ""}</h3>
                <dl className="mt-2 grid grid-cols-2 gap-3 lg:grid-cols-4">
                  <Figure label="Requests" value={tokens(overview.requests)} />
                  <Figure label="Tokens" value={tokens(usedTokens)} sub={`${tokens(overview.inputTokens)} in · ${tokens(overview.outputTokens)} out`} />
                  <Figure label="Estimated cost" value={formatUsd(overview.estimatedCostUsd)} />
                  <Figure label="Fallback · error rate" value={`${formatRate(overview.fallbackRate)} · ${formatRate(overview.errorRate)}`} />
                </dl>
                <div className="mt-3 space-y-3">
                  <Budget label="Token budget" used={usedTokens} limit={overview.budgetTokens} format={tokens} />
                  <Budget label="Cost budget" used={overview.estimatedCostUsd} limit={overview.budgetUsd} format={formatUsd} />
                </div>
              </div>

              <div>
                <h3 className="text-sm font-semibold">Top workspaces</h3>
                {overview.topWorkspaces.length === 0 ? (
                  <p className="mt-1 text-sm text-muted-foreground">No workspace has used AI assist this month.</p>
                ) : (
                  <div className="mt-2 overflow-x-auto">
                    <table className="w-full min-w-[420px] text-sm">
                      <caption className="sr-only">Workspaces using AI assist most this month</caption>
                      <thead>
                        <tr className="border-b border-border text-left text-xs text-muted-foreground">
                          <th scope="col" className="py-2 pr-3 font-medium">
                            Workspace
                          </th>
                          <th scope="col" className="py-2 pr-3 text-right font-medium">
                            Requests
                          </th>
                          <th scope="col" className="py-2 pr-3 text-right font-medium">
                            Tokens
                          </th>
                          <th scope="col" className="py-2 text-right font-medium">
                            Estimated cost
                          </th>
                        </tr>
                      </thead>
                      <tbody>
                        {overview.topWorkspaces.map((w) => (
                          <tr key={w.workspaceId} className="border-b border-border/60 last:border-0">
                            <th scope="row" className="py-2 pr-3 text-left font-medium">
                              {w.workspaceName}
                            </th>
                            <td className="py-2 pr-3 text-right tabular-nums">{tokens(w.requests)}</td>
                            <td className="py-2 pr-3 text-right tabular-nums">{tokens(w.tokens)}</td>
                            <td className="py-2 text-right tabular-nums">{formatUsd(w.estimatedCostUsd)}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )}
              </div>
            </>
          )}
        </CardContent>
      </section>
      <ConfirmDialog
        open={confirming !== null}
        onCancel={() => setConfirming(null)}
        onConfirm={() => void apply(confirming === true)}
        busy={busy}
        tone={confirming ? "primary" : "danger"}
        title={confirming ? "Turn AI assist back on?" : "Turn off AI assist for the platform?"}
        description={
          confirming
            ? "Workspaces that opted in can use AI assist again on their next request."
            : "Every workspace's AI requests stop on their next call, until you turn it back on. Workspace settings and earlier results are kept."
        }
        confirmLabel={confirming ? "Turn on AI assist" : "Turn off AI assist"}
      />
    </Card>
  );
}
