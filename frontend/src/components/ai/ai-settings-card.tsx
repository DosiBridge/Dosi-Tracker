"use client";

import { useEffect, useId, useState } from "react";
import dynamic from "next/dynamic";
import { Sparkles } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { Switch } from "@/components/ui/switch";
import { toast } from "@/components/toast";
import { useAiAssistant } from "@/components/ai/ai-provider";
import { AiUnavailableNote } from "@/components/ai/ai-unavailable-note";
import { getAiSettings, getAiUsage, updateAiSettings } from "@/hooks/ai-api";
import { useBrowserTimeZone } from "@/hooks/use-reporting-zone";
import { isAiFeatureId, type AiUsageSummaryDto, type AiWorkspaceSettingsDto } from "@/lib/ai-contract";
import {
  AI_FEATURE_LABELS,
  aiAcknowledgementReasons,
  aiConsentCurrent,
  aiEffectivelyOn,
  aiErrorState,
  aiSettingsFlags,
  aiSettingsUpdate,
  aiSetupNotice,
  aiStatusLine,
  formatAiDate,
  type AiAcknowledgementReason,
  type AiSettingsFlags,
} from "@/lib/ai-assist";
import { demoAiSettings, demoAiUsage } from "@/lib/ai-demo-settings";
import { aiMeter, formatUsd } from "@/lib/ai-format";

// The disclosure dialog downloads only when the owner first needs it.
const AiEnableDialog = dynamic(() => import("@/components/ai/ai-enable-dialog").then((m) => m.AiEnableDialog), { ssr: false });

interface Pending {
  next: AiSettingsFlags;
  reasons: AiAcknowledgementReason[];
}

function UsageMeter({ label, used, limit }: { label: string; used: number; limit: number | null }) {
  const meter = aiMeter(used, limit);
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between gap-3 text-xs">
        <span className="font-medium">{label}</span>
        <span className="tabular-nums text-muted-foreground">{meter.text}</span>
      </div>
      {meter.percent !== null && <Progress value={meter.percent} aria-label={`${label} used this month`} />}
    </div>
  );
}

function Usage({ usage, timeZone }: { usage: AiUsageSummaryDto; timeZone: string }) {
  const renews = formatAiDate(usage.resetsAt, timeZone);
  const byFeature = (usage.byFeature ?? []).filter((f) => f.requests > 0);
  return (
    <div className="space-y-3 rounded-xl border border-border p-3">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold">This month</h3>
        {renews && <span className="text-xs text-muted-foreground">Renews {renews}</span>}
      </div>
      <UsageMeter label="AI requests" used={usage.requestsUsed} limit={usage.requestsLimit} />
      <UsageMeter label="Tokens" used={usage.tokensUsed} limit={usage.tokensLimit} />
      {usage.estimatedCostUsd !== undefined && usage.estimatedCostUsd !== null && (
        <p className="text-xs text-muted-foreground">
          Estimated cost: <span className="font-medium text-foreground">{formatUsd(usage.estimatedCostUsd)}</span>
        </p>
      )}
      {byFeature.length > 0 && (
        <dl className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-4 gap-y-1 text-xs">
          {byFeature.map((f) => (
            <div key={f.featureId} className="contents">
              <dt className="text-muted-foreground">{isAiFeatureId(f.featureId) ? AI_FEATURE_LABELS[f.featureId] : f.featureId}</dt>
              <dd className="text-right tabular-nums">
                {f.requests.toLocaleString("en-US")} {f.requests === 1 ? "request" : "requests"}
              </dd>
            </div>
          ))}
        </dl>
      )}
    </div>
  );
}

/**
 * The owner's AI assist settings (Settings > Workspace; AI assist spec
 * §18.6): the state, the master switch, who may use it and what may be
 * included, and this month's usage. Turning it on, re-accepting a changed
 * disclosure, or including window titles or task notes first shows the
 * server's disclosure (AiEnableDialog); the save then carries the version the
 * owner acknowledged. A server without AI says so instead of offering
 * switches that would only fail. A demo keeps its changes on the page.
 */
export function AiSettingsCard({ demoNote }: { demoNote: string }) {
  const headingId = useId();
  const { status, statusLoading, live, role, refreshStatus } = useAiAssistant();
  const timeZone = useBrowserTimeZone();
  const notice = aiSetupNotice(status, role);
  const unavailable = live && notice === "server-unavailable";
  const checking = live && statusLoading && !status;

  const [settings, setSettings] = useState<AiWorkspaceSettingsDto | null>(() => (live ? null : demoAiSettings()));
  const [usage, setUsage] = useState<AiUsageSummaryDto | null>(() => (live ? null : demoAiUsage(role)));
  const [loadError, setLoadError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const [pending, setPending] = useState<Pending | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dialogError, setDialogError] = useState<string | null>(null);

  const ready = live && !unavailable && !checking;
  useEffect(() => {
    if (!ready) return;
    let current = true;
    getAiSettings().then(
      (loaded) => {
        if (!current) return;
        setSettings(loaded);
        setLoadError(null);
      },
      (err: unknown) => {
        if (current) setLoadError(aiErrorState(err, { fallback: "Couldn't load the AI settings." }).message);
      },
    );
    // Usage is secondary: without it the switches still work.
    getAiUsage().then(
      (loaded) => {
        if (current) setUsage(loaded);
      },
      () => {
        if (current) setUsage(null);
      },
    );
    return () => {
      current = false;
    };
  }, [ready, reload]);

  async function save(next: AiSettingsFlags, acknowledged: string | null, inDialog: boolean) {
    if (!settings) return;
    if (!live) {
      setSettings({ ...settings, ...next, consentVersion: acknowledged ?? settings.consentVersion });
      setPending(null);
      toast({ tone: "info", title: "Demo workspace", description: demoNote });
      return;
    }
    setSaving(true);
    setError(null);
    setDialogError(null);
    try {
      const saved = await updateAiSettings(aiSettingsUpdate(next, acknowledged));
      setSettings(saved);
      setPending(null);
      refreshStatus();
      toast({
        tone: "success",
        title: "AI settings saved",
        description: saved.isEnabled ? "Members can see what AI assist sends in Settings > Privacy." : "AI assist is off. Nothing is sent to an AI provider.",
      });
    } catch (err) {
      const state = aiErrorState(err, { fallback: "Couldn't save the AI settings." });
      if (inDialog) setDialogError(state.message);
      else setError(state.message);
      // The disclosure changed while the owner read it: load the new one, so they accept what is current.
      if (state.kind === "disclosure") setReload((n) => n + 1);
    } finally {
      setSaving(false);
    }
  }

  function change(patch: Partial<AiSettingsFlags>) {
    if (!settings) return;
    const current = aiSettingsFlags(settings);
    const next = { ...current, ...patch };
    const reasons = aiAcknowledgementReasons(current, next, aiConsentCurrent(settings));
    if (reasons.length > 0) {
      setDialogError(null);
      setPending({ next, reasons });
      return;
    }
    void save(next, null, false);
  }

  const line = aiStatusLine(status, { demo: !live });
  const on = settings ? aiEffectivelyOn(settings) : false;
  const stale = !!settings && settings.isEnabled && !aiConsentCurrent(settings);
  const enabledOn = settings?.enabledAt ? formatAiDate(settings.enabledAt, timeZone) : null;

  return (
    <Card>
      <section aria-labelledby={headingId}>
        <CardHeader>
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <Sparkles aria-hidden className="h-4 w-4 shrink-0 text-primary" />
              <CardTitle id={headingId}>AI assist</CardTitle>
              {!checking && <Badge tone={line.tone}>{line.label}</Badge>}
            </div>
            <p className="mt-0.5 text-xs text-muted-foreground">
              Optional summaries written from your tracker data. Off until you turn it on; nothing is generated until someone presses Generate.
            </p>
          </div>
        </CardHeader>
        <CardContent className="space-y-5">
          {checking ? (
            <p role="status" className="text-sm text-muted-foreground">
              Checking AI assist…
            </p>
          ) : unavailable ? (
            <AiUnavailableNote />
          ) : (
            <>
              <p className="text-sm text-muted-foreground">{line.text}</p>
              {!live && <AiUnavailableNote message={`${demoNote} AI output in the demo is sample text; nothing is sent anywhere.`} />}
              {loadError && (
                <div role="alert" className="flex flex-wrap items-center gap-3 rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
                  <span className="min-w-0 flex-1">{loadError}</span>
                  <Button size="sm" variant="outline" onClick={() => setReload((n) => n + 1)}>
                    Try again
                  </Button>
                </div>
              )}
              {live && !settings && !loadError && (
                <p role="status" className="text-sm text-muted-foreground">
                  Loading AI settings…
                </p>
              )}
              {settings && (
                <>
                  {stale && (
                    <div role="note" className="flex flex-wrap items-center gap-3 rounded-lg bg-warning/10 px-3 py-2 text-xs text-warning-on-tint">
                      <span className="min-w-0 flex-1">
                        The AI disclosure changed (now version {settings.disclosure.version}). AI assist is paused until you review it.
                      </span>
                      <Button size="sm" variant="outline" disabled={saving} onClick={() => change({ isEnabled: true })}>
                        Review and turn on
                      </Button>
                    </div>
                  )}
                  <Switch
                    checked={on}
                    onChange={(v) => change({ isEnabled: v })}
                    disabled={saving}
                    label="Use AI assist in this workspace"
                    description={
                      on && enabledOn
                        ? `On since ${enabledOn}${settings.enabledByName ? `, turned on by ${settings.enabledByName}` : ""}.`
                        : "You'll see what is sent, and to whom, before anything is turned on."
                    }
                  />
                  <div className="space-y-4 border-t border-border pt-4">
                    <Switch
                      checked={settings.allowManagers}
                      onChange={(v) => change({ allowManagers: v })}
                      disabled={saving}
                      label="Managers can use AI assist"
                      description="Summaries of the team's time, for anyone who may already see it."
                    />
                    <Switch
                      checked={settings.allowMemberSelfService}
                      onChange={(v) => change({ allowMemberSelfService: v })}
                      disabled={saving}
                      label="Members can use AI on their own data"
                      description="A member's summaries only ever cover their own time."
                    />
                    <Switch
                      checked={settings.allowWindowTitles}
                      onChange={(v) => change({ allowWindowTitles: v })}
                      disabled={saving}
                      label="Include window titles (off by default)"
                      description="Only for projects that record them. Titles can name files, pages and people."
                    />
                    <Switch
                      checked={settings.allowNotes}
                      onChange={(v) => change({ allowNotes: v })}
                      disabled={saving}
                      label="Include task notes (off by default)"
                      description="Notes are written by members and can hold private details."
                    />
                  </div>
                  {error && (
                    <p role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
                      {error}
                    </p>
                  )}
                  {saving && !pending && (
                    <p role="status" className="text-xs text-muted-foreground">
                      Saving…
                    </p>
                  )}
                  {usage && <Usage usage={usage} timeZone={timeZone} />}
                </>
              )}
            </>
          )}
        </CardContent>
      </section>
      {pending && settings && (
        <AiEnableDialog
          disclosure={settings.disclosure}
          reasons={pending.reasons}
          busy={saving}
          error={dialogError}
          onCancel={() => setPending(null)}
          onConfirm={(version) => void save(pending.next, version, true)}
        />
      )}
    </Card>
  );
}
