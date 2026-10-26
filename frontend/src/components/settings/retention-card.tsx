"use client";

import { useCallback, useEffect, useId, useState } from "react";
import { AlertTriangle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { toast } from "@/components/toast";
import { requestErrorText } from "@/components/layout/request-error";
import {
  getActivityRetentionApi,
  getCaptureRetentionApi,
  getWorkspaceSettings,
  updateActivityRetentionApi,
  updateCaptureRetentionApi,
  updateWorkspaceSettings,
} from "@/hooks/useApi";
import { MEMBER_CAPTURE_DELETE_LIMITS, RETENTION_LIMITS } from "@/lib/api-contract";
import { currentWorkspaceId } from "@/lib/live-session";
import {
  memberWindowLabel,
  memberWindowMinutes,
  retentionDays,
  retentionDeletesMore,
  retentionDraftOf,
  retentionProblem,
  type RetentionDraft,
  type RetentionKind,
} from "@/lib/retention";

interface Form {
  capture: RetentionDraft;
  activity: RetentionDraft;
  /** Minutes members may delete their own captures (as typed); null when the server doesn't have the setting. */
  memberWindow: string | null;
}

interface Loaded {
  saved: Form;
  /** Whether the server runs the sweeps at all (per policy); null when it can't say. */
  enforced: { capture: boolean | null; activity: boolean | null };
  /** Which policies this server has (an older one may lack activity retention). */
  supports: { capture: boolean; activity: boolean };
}

const DEMO_FORM: Form = {
  capture: retentionDraftOf("capture", null),
  activity: retentionDraftOf("activity", null),
  memberWindow: String(MEMBER_CAPTURE_DELETE_LIMITS.default),
};

const COPY: Record<RetentionKind, { title: string; what: string; keeps: string }> = {
  capture: {
    title: "Screenshots and webcam frames",
    what: "Delete captures older than",
    keeps: "Time blocks stay; only their images go.",
  },
  activity: {
    title: "Tracked time",
    what: "Delete time blocks older than",
    keeps:
      "Reports and payroll lose those days. A block that still has captures is kept until they go, and recent blocks the agents may still upload are never deleted.",
  },
};

/**
 * The owner's retention choices (E2E-06): how long captures and tracked time
 * are kept, and how long members may delete their own captures (E2E-07). Off
 * until turned on; turning a policy on, or shortening it, asks first, since
 * the next daily clean-up deletes what falls outside it.
 */
export function RetentionCard({ live, demoNote }: { live: boolean; demoNote: string }) {
  const id = useId();
  const [loaded, setLoaded] = useState<Loaded | null>(live ? null : { saved: DEMO_FORM, enforced: { capture: true, activity: true }, supports: { capture: true, activity: true } });
  const [form, setForm] = useState<Form | null>(live ? null : DEMO_FORM);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const workspaceId = live ? currentWorkspaceId() : null;

  const load = useCallback(async () => {
    if (!workspaceId) {
      setLoadError("Choose a workspace first.");
      return;
    }
    setLoadError(null);
    try {
      const [capture, activity, settings] = await Promise.all([
        getCaptureRetentionApi(workspaceId),
        getActivityRetentionApi(workspaceId),
        getWorkspaceSettings().catch(() => null),
      ]);
      const window = settings?.memberCaptureDeleteMinutes;
      const saved: Form = {
        capture: retentionDraftOf("capture", capture),
        activity: retentionDraftOf("activity", activity),
        memberWindow: typeof window === "number" ? String(window) : settings && "memberCaptureDeleteMinutes" in settings ? String(MEMBER_CAPTURE_DELETE_LIMITS.default) : null,
      };
      setLoaded({
        saved,
        enforced: { capture: typeof capture?.isEnforced === "boolean" ? capture.isEnforced : null, activity: typeof activity?.isEnforced === "boolean" ? activity.isEnforced : null },
        supports: { capture: capture !== null, activity: activity !== null },
      });
      setForm(saved);
    } catch (err) {
      setLoadError(requestErrorText(err, "Couldn't load the retention settings."));
    }
  }, [workspaceId]);

  useEffect(() => {
    if (!live) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void load();
  }, [live, load]);

  const problems = form
    ? {
        capture: retentionProblem("capture", form.capture),
        activity: retentionProblem("activity", form.activity),
        memberWindow:
          form.memberWindow !== null && memberWindowMinutes(form.memberWindow) === null
            ? `Enter whole minutes from ${MEMBER_CAPTURE_DELETE_LIMITS.min} to ${MEMBER_CAPTURE_DELETE_LIMITS.max} (a week).`
            : null,
      }
    : null;
  const invalid = !!problems && Object.values(problems).some(Boolean);
  const deletesMore =
    !!form && !!loaded && (retentionDeletesMore("capture", loaded.saved.capture, form.capture) || retentionDeletesMore("activity", loaded.saved.activity, form.activity));

  async function save() {
    if (!form || !loaded || invalid) return;
    setConfirming(false);
    if (!live) {
      toast({ tone: "info", title: "Demo workspace", description: demoNote });
      return;
    }
    if (!workspaceId) return;
    setSaving(true);
    setError(null);
    try {
      const policy = (kind: RetentionKind) => ({
        isEnabled: form[kind].isEnabled,
        retentionDays: retentionDays(kind, form[kind]) ?? RETENTION_LIMITS[kind].default,
      });
      await Promise.all([
        loaded.supports.capture ? updateCaptureRetentionApi(workspaceId, policy("capture")) : null,
        loaded.supports.activity ? updateActivityRetentionApi(workspaceId, policy("activity")) : null,
      ]);
      const minutes = form.memberWindow === null ? null : memberWindowMinutes(form.memberWindow);
      if (minutes !== null && form.memberWindow !== loaded.saved.memberWindow) {
        // Read the defaults again right before writing, so an edit made in the card above isn't overwritten.
        const settings = await getWorkspaceSettings();
        await updateWorkspaceSettings({ ...settings, memberCaptureDeleteMinutes: minutes });
      }
      toast({ tone: "success", title: "Retention saved", description: "The next daily clean-up follows these settings." });
      await load();
    } catch (err) {
      setError(requestErrorText(err, "Couldn't save the retention settings."));
    } finally {
      setSaving(false);
    }
  }

  const setDraft = (kind: RetentionKind, patch: Partial<RetentionDraft>) => setForm((f) => (f ? { ...f, [kind]: { ...f[kind], ...patch } } : f));

  return (
    <Card>
      <CardHeader>
        <div>
          <CardTitle>Retention</CardTitle>
          <p className="mt-0.5 text-xs text-muted-foreground">
            How long this workspace keeps what the agents upload. Nothing is deleted automatically unless you turn it on here.
          </p>
        </div>
      </CardHeader>
      <CardContent className="space-y-6">
        {live && !form && !loadError && (
          <p role="status" className="text-sm text-muted-foreground">
            Loading retention settings…
          </p>
        )}
        {loadError && (
          <div role="alert" className="flex flex-wrap items-center gap-3 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-xs text-warning-on-tint">
            <span className="min-w-0 flex-1">{loadError}</span>
            <Button size="sm" variant="outline" onClick={() => void load()}>
              Try again
            </Button>
          </div>
        )}
        {form && loaded && problems && (
          <>
            {(["capture", "activity"] as const).map((kind) =>
              loaded.supports[kind] ? (
                <section key={kind} aria-labelledby={`${id}-${kind}-title`} className="space-y-3 rounded-xl border border-border p-4">
                  <h3 id={`${id}-${kind}-title`} className="text-sm font-semibold">
                    {COPY[kind].title}
                  </h3>
                  <Switch
                    checked={form[kind].isEnabled}
                    onChange={(on) => setDraft(kind, { isEnabled: on })}
                    label="Delete automatically after a while"
                    description={form[kind].isEnabled ? COPY[kind].keeps : "Off: kept until someone deletes them."}
                    disabled={saving}
                  />
                  {form[kind].isEnabled && (
                    <div className="space-y-1.5">
                      <label htmlFor={`${id}-${kind}-days`} className="text-sm font-medium">
                        {COPY[kind].what} (days)
                      </label>
                      <Input
                        id={`${id}-${kind}-days`}
                        inputMode="numeric"
                        value={form[kind].days}
                        onChange={(e) => setDraft(kind, { days: e.target.value })}
                        className="max-w-[10rem]"
                        aria-invalid={problems[kind] ? true : undefined}
                        aria-describedby={`${id}-${kind}-hint`}
                      />
                      <p id={`${id}-${kind}-hint`} className={problems[kind] ? "text-xs text-danger-on-tint" : "text-xs text-muted-foreground"}>
                        {problems[kind] ?? `${RETENTION_LIMITS[kind].min} to ${RETENTION_LIMITS[kind].max} days.`}
                      </p>
                    </div>
                  )}
                  {loaded.enforced[kind] === false && (
                    <p className="flex items-start gap-2 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-xs text-warning-on-tint">
                      <AlertTriangle aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                      This server&apos;s operator has turned the automatic clean-up off. Your choice is saved, but nothing is deleted until
                      they turn it on.
                    </p>
                  )}
                </section>
              ) : null,
            )}
            {form.memberWindow !== null && (
              <div className="space-y-1.5">
                <label htmlFor={`${id}-window`} className="text-sm font-medium">
                  Members may delete their own captures for (minutes after upload)
                </label>
                <Input
                  id={`${id}-window`}
                  inputMode="numeric"
                  value={form.memberWindow}
                  onChange={(e) => setForm((f) => (f ? { ...f, memberWindow: e.target.value } : f))}
                  className="max-w-[10rem]"
                  aria-invalid={problems.memberWindow ? true : undefined}
                  aria-describedby={`${id}-window-hint`}
                />
                <p id={`${id}-window-hint`} className={problems.memberWindow ? "text-xs text-danger-on-tint" : "text-xs text-muted-foreground"}>
                  {problems.memberWindow ??
                    `${memberWindowLabel(memberWindowMinutes(form.memberWindow) ?? 0)}. 0 turns it off; owners and managers can always delete captures, giving a reason.`}
                </p>
              </div>
            )}
            {error && (
              <p role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
                {error}
              </p>
            )}
            <div className="flex justify-end">
              <Button onClick={() => (deletesMore ? setConfirming(true) : void save())} disabled={saving || invalid}>
                {saving ? "Saving…" : "Save retention"}
              </Button>
            </div>
          </>
        )}
      </CardContent>
      <ConfirmDialog
        open={confirming}
        onCancel={() => setConfirming(false)}
        onConfirm={() => void save()}
        title="Delete older data automatically?"
        description="At the next daily clean-up the server deletes everything older than the days you set, for every member. Deleted captures and time blocks can't be recovered."
        confirmLabel="Save and delete older data"
        busy={saving}
      />
    </Card>
  );
}
