"use client";

import { Suspense, useCallback, useEffect, useId, useMemo, useState, useSyncExternalStore } from "react";
import dynamic from "next/dynamic";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { User, Palette, Bell, Check, Monitor, Moon, Sun, Eye, Building2, KeyRound, Globe } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { toast } from "@/components/toast";
import { useTheme } from "@/components/theme-provider";
import { useSession } from "@/components/session-provider";
import { TrackingFields } from "@/components/projects/tracking-fields";
import { AiPrivacyNote } from "@/components/ai/ai-privacy-note";
import { BUILT_IN_PROJECT_DEFAULTS, normalizeNewProjectDefaults, validateTrackingSettings } from "@/components/projects/project-form";
import { requestErrorStatus, requestErrorText } from "@/components/layout/request-error";
import {
  changeMyPassword,
  getApi,
  getMyPreferences,
  getWorkspaceSettings,
  putApi,
  updateMyPreferences,
  updateWorkspaceSettings,
} from "@/hooks/useApi";
import { useBrowserTimeZone, useReportingZone } from "@/hooks/use-reporting-zone";
import { roleLabels } from "@/lib/roles";
import { formatUtcOffset, normalizeTimeZone, supportedTimeZones, timeZoneLabel, utcOffsetMinutes } from "@/lib/time-zone";
import type { NewProjectDefaultsDto } from "@/lib/api-contract";
import type { Role } from "@/lib/types";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { cn } from "@/lib/utils";

type TabKey = "profile" | "security" | "workspace" | "privacy" | "appearance" | "notifications";

const allTabs: { key: TabKey; label: string; icon: LucideIcon; roles: Role[]; soon?: boolean }[] = [
  { key: "profile", label: "Profile", icon: User, roles: ["owner", "admin", "worker", "client"] },
  { key: "security", label: "Security", icon: KeyRound, roles: ["owner", "admin", "worker", "client"] },
  { key: "workspace", label: "Workspace", icon: Building2, roles: ["owner"] },
  { key: "privacy", label: "Privacy", icon: Eye, roles: ["owner", "admin", "worker"] },
  { key: "appearance", label: "Appearance", icon: Palette, roles: ["owner", "admin", "worker", "client"] },
  { key: "notifications", label: "Notifications", icon: Bell, roles: ["owner", "admin", "worker", "client"], soon: true },
];

/** GET/PUT /api/account/my-profile shape (ABP account profile). */
interface ProfileForm {
  userName: string;
  name: string;
  surname: string;
  email: string;
  phoneNumber: string;
}

const DEMO_NOTE = "Demo workspace — nothing was saved. Sign in to your workspace to keep changes.";

// The owner's AI settings (and its disclosure dialog) download only when the Workspace tab opens.
const AiSettingsCard = dynamic(() => import("@/components/ai/ai-settings-card").then((m) => m.AiSettingsCard), {
  ssr: false,
  loading: () => (
    <p role="status" className="text-sm text-muted-foreground">
      Loading AI settings…
    </p>
  ),
});

// The journeys' cards (devices, account deletion, retention, workspace deletion) load with their tab, not with the page.
const cardLoading = (what: string) =>
  function CardLoading() {
    return (
      <p role="status" className="text-sm text-muted-foreground">
        Loading {what}…
      </p>
    );
  };
const MyDevicesCard = dynamic(() => import("@/components/settings/my-devices-card").then((m) => m.MyDevicesCard), {
  ssr: false,
  loading: cardLoading("your devices"),
});
const DeleteAccountCard = dynamic(() => import("@/components/settings/delete-account-card").then((m) => m.DeleteAccountCard), {
  ssr: false,
  loading: cardLoading("account options"),
});
const RetentionCard = dynamic(() => import("@/components/settings/retention-card").then((m) => m.RetentionCard), {
  ssr: false,
  loading: cardLoading("retention settings"),
});
const WorkspaceDangerCard = dynamic(() => import("@/components/settings/workspace-danger-card").then((m) => m.WorkspaceDangerCard), {
  ssr: false,
  loading: cardLoading("workspace options"),
});

const isTabKey = (value: string | null): value is TabKey => allTabs.some((t) => t.key === value);

function profileFormOf(p: Record<string, unknown>): ProfileForm {
  const s = (v: unknown) => (typeof v === "string" ? v : "");
  return { userName: s(p.userName), name: s(p.name), surname: s(p.surname), email: s(p.email), phoneNumber: s(p.phoneNumber) };
}

const subscribeNothing = () => () => {};

/** True once rendering in the browser (the zone list comes from the browser's own Intl data). */
function useHydrated(): boolean {
  return useSyncExternalStore(subscribeNothing, () => true, () => false);
}

export default function SettingsPage() {
  // The ?tab= preselection reads the URL; outside a Suspense boundary that would keep the whole page from prerendering.
  return (
    <Suspense fallback={<div className="py-16 text-center text-sm text-muted-foreground">Loading settings…</div>}>
      <SettingsContent />
    </Suspense>
  );
}

function SettingsContent() {
  const { user: currentUser, isLive: live, refreshSession, logout, workspace } = useSession();
  const tabs = allTabs.filter((t) => t.roles.includes(currentUser.role));
  // `?tab=workspace` (a link from AI assist, or the privacy note) opens that tab when this role has it; anything else is ignored.
  const requested = useSearchParams().get("tab");
  const [chosen, setChosen] = useState<{ tab: TabKey; requested: string | null } | null>(null);
  const allowed = (key: string | null): key is TabKey => isTabKey(key) && tabs.some((t) => t.key === key);
  // A newer link wins over an earlier click; a tab this role can't open never shows.
  const picked = chosen && chosen.requested === requested && allowed(chosen.tab) ? chosen.tab : null;
  const tab: TabKey = picked ?? (allowed(requested) ? requested : "profile");
  const setTab = (next: TabKey) => setChosen({ tab: next, requested });
  const zone = useReportingZone(live);

  function onTabKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    const keys = ["ArrowDown", "ArrowRight", "ArrowUp", "ArrowLeft", "Home", "End"];
    if (!keys.includes(event.key)) return;
    event.preventDefault();
    const index = tabs.findIndex((t) => t.key === tab);
    const step = event.key === "ArrowDown" || event.key === "ArrowRight" ? 1 : -1;
    const next = event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1 : (index + step + tabs.length) % tabs.length;
    setTab(tabs[next].key);
    document.getElementById(`settings-${tabs[next].key}-tab`)?.focus();
  }

  return (
    <PageStack>
      <PageHeader eyebrow="Workspace" title="Settings" description="Your profile, sign-in, time zone and — for owners — workspace defaults." />

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-[220px_1fr]">
        {/* Tabs: a row on phones, a column on wide screens. */}
        <div
          role="tablist"
          aria-label="Settings sections"
          aria-orientation="vertical"
          onKeyDown={onTabKeyDown}
          className="flex gap-1 overflow-x-auto pb-1 lg:flex-col lg:overflow-visible lg:pb-0"
        >
          {tabs.map((t) => {
            const Icon = t.icon;
            const active = tab === t.key;
            return (
              <button
                key={t.key}
                type="button"
                role="tab"
                id={`settings-${t.key}-tab`}
                aria-selected={active}
                aria-controls={active ? `settings-${t.key}-panel` : undefined}
                tabIndex={active ? 0 : -1}
                onClick={() => setTab(t.key)}
                className={cn(
                  "flex shrink-0 items-center gap-2.5 whitespace-nowrap rounded-lg px-3 py-2.5 text-sm font-medium transition-colors",
                  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                  active ? "bg-accent text-accent-foreground" : "text-muted-foreground hover:bg-muted hover:text-foreground",
                )}
              >
                <Icon aria-hidden className="h-[18px] w-[18px]" /> {t.label}
                {t.soon && (
                  <Badge tone="muted" className="ml-auto px-2 py-0 text-[10px]">
                    Soon
                  </Badge>
                )}
              </button>
            );
          })}
        </div>

        <div role="tabpanel" id={`settings-${tab}-panel`} aria-labelledby={`settings-${tab}-tab`} className="min-w-0 space-y-6">
          {tab === "profile" && (
            <>
              <ProfileCard live={live} onSaved={() => void refreshSession({ silent: true })} />
              <TimeZoneCard live={live} zone={zone} />
            </>
          )}
          {tab === "security" && (
            <>
              <SecurityCard live={live} />
              {currentUser.role !== "client" && (
                <MyDevicesCard live={live} userId={currentUser.id} manager={currentUser.role === "owner" || currentUser.role === "admin"} />
              )}
              <DeleteAccountCard live={live} onDeleted={logout} />
            </>
          )}
          {tab === "workspace" && (
            <>
              <WorkspaceCard live={live} onSaved={zone.refresh} />
              <RetentionCard live={live} demoNote={DEMO_NOTE} />
              <AiSettingsCard demoNote={DEMO_NOTE} />
              <WorkspaceDangerCard live={live} workspaceName={workspace.name} demoNote={DEMO_NOTE} />
            </>
          )}
          {tab === "privacy" && (
            <>
              <PrivacyCard role={currentUser.role} />
              <AiPrivacyNote />
            </>
          )}
          {tab === "appearance" && (
            <Card>
              <CardHeader>
                <CardTitle>Appearance</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <p className="text-sm text-muted-foreground">Choose how Dosi-Tracker looks to you on this device.</p>
                <ThemePicker />
              </CardContent>
            </Card>
          )}
          {tab === "notifications" && (
            <Card>
              <CardHeader>
                <CardTitle>Notifications</CardTitle>
                <Badge tone="muted">Coming soon</Badge>
              </CardHeader>
              <CardContent className="space-y-3 text-sm text-muted-foreground">
                <p>
                  Notices about your workspace — for example when you&apos;re added to a project — appear under the bell at the top of
                  every page.
                </p>
                <p>Email digests (daily and weekly summaries, low-activity alerts) are coming soon. There is nothing to set up yet.</p>
              </CardContent>
            </Card>
          )}
        </div>
      </div>
    </PageStack>
  );
}

/* ----------------------------- Profile ----------------------------- */

function ProfileCard({ live, onSaved }: { live: boolean; onSaved: () => void }) {
  const { user: currentUser } = useSession();
  // Live profile (only when a real backend session exists; otherwise demo mode).
  const [profileRaw, setProfileRaw] = useState<Record<string, unknown> | null>(null);
  const [profileForm, setProfileForm] = useState<ProfileForm | null>(null);
  const [profileLoading, setProfileLoading] = useState(live);
  const [profileError, setProfileError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const loadProfile = useCallback(async () => {
    setProfileLoading(true);
    try {
      const p = (await getApi("/api/account/my-profile")) as Record<string, unknown>;
      setProfileRaw(p);
      setProfileForm(profileFormOf(p));
      setProfileError(null);
    } catch {
      setProfileError("Couldn't load your profile from the server. Try again later.");
      setProfileForm(null);
    } finally {
      setProfileLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!live) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void loadProfile();
  }, [live, loadProfile]);

  const setProfileField = (key: keyof ProfileForm, value: string) => setProfileForm((f) => (f ? { ...f, [key]: value } : f));

  async function save() {
    if (!live) {
      toast({ tone: "info", title: "Demo workspace", description: DEMO_NOTE });
      return;
    }
    if (!profileForm) return;
    setSaving(true);
    setSaveError(null);
    setSaved(false);
    try {
      // Preserve the fields the backend returned (the concurrency stamp in particular)…
      const response = (await putApi("/api/account/my-profile", { ...(profileRaw ?? {}), ...profileForm })) as Record<string, unknown> | null;
      // …and adopt the saved profile, whose stamp the NEXT save must carry.
      if (response && typeof response === "object") {
        setProfileRaw(response);
        setProfileForm(profileFormOf(response));
      } else {
        await loadProfile();
      }
      setSaved(true);
      setTimeout(() => setSaved(false), 1600);
      toast({ tone: "success", title: "Profile saved", description: "Your changes are live." });
      // The name shown in the sidebar/topbar comes from the session: refresh it.
      onSaved();
    } catch (err) {
      setSaveError(requestErrorText(err, "Couldn't save your profile."));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Profile</CardTitle>
      </CardHeader>
      <CardContent className="space-y-5">
        <div className="flex items-center gap-4">
          <Avatar name={currentUser.name} size="lg" status={currentUser.status} />
          <div className="min-w-0">
            <div className="truncate font-medium">{currentUser.name}</div>
            <div className="mt-1 flex flex-wrap items-center gap-2">
              <Badge tone="primary">{roleLabels[currentUser.role]}</Badge>
              <span className="text-xs text-muted-foreground">Profile photos are coming soon — your initials are shown for now.</span>
            </div>
          </div>
        </div>
        <p className="rounded-lg bg-muted/60 px-3 py-2 text-xs text-muted-foreground">Your profile is shared across all your workspaces.</p>
        {live && profileLoading && (
          <p role="status" className="text-sm text-muted-foreground">
            Loading profile…
          </p>
        )}
        {live && profileError && (
          <div role="alert" className="flex flex-wrap items-center gap-3 text-xs text-danger-on-tint">
            <span>{profileError}</span>
            <Button size="sm" variant="outline" onClick={() => void loadProfile()}>
              Try again
            </Button>
          </div>
        )}
        {live && !profileLoading && profileForm ? (
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <Field label="First name" value={profileForm.name} onChange={(v) => setProfileField("name", v)} autoComplete="given-name" />
            <Field label="Surname" value={profileForm.surname} onChange={(v) => setProfileField("surname", v)} autoComplete="family-name" />
            <Field label="Email" value={profileForm.email} readOnly autoComplete="email" type="email" />
            <Field label="Username" value={profileForm.userName} onChange={(v) => setProfileField("userName", v)} autoComplete="username" />
            <Field label="Phone number" value={profileForm.phoneNumber} onChange={(v) => setProfileField("phoneNumber", v)} autoComplete="tel" type="tel" />
          </div>
        ) : !live ? (
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <Field label="Full name" defaultValue={currentUser.name} autoComplete="name" />
            <Field label="Email" value={currentUser.email} readOnly autoComplete="email" type="email" />
            <Field label="Designation" defaultValue={currentUser.designation} autoComplete="organization-title" />
          </div>
        ) : null}
        <div className="flex flex-wrap items-center justify-end gap-3">
          {saveError && (
            <span role="alert" className="text-sm text-danger-on-tint">
              {saveError}
            </span>
          )}
          {saved && (
            <span role="status" className="flex items-center gap-1.5 text-sm text-success-on-tint">
              <Check aria-hidden className="h-4 w-4" /> Saved
            </span>
          )}
          <Button onClick={() => void save()} disabled={saving || (live && !profileForm)}>
            {saving ? "Saving…" : "Save changes"}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function Field({
  label,
  defaultValue,
  value,
  onChange,
  autoComplete,
  type = "text",
  readOnly = false,
}: {
  label: string;
  defaultValue?: string;
  value?: string;
  onChange?: (value: string) => void;
  autoComplete?: string;
  type?: string;
  /** Shown but not editable (the email: it identifies the account everywhere). */
  readOnly?: boolean;
}) {
  // Generated id keeps the label programmatically associated with its input,
  // so assistive tech announces the field by name.
  const id = useId();
  return (
    <div className="space-y-1.5">
      <label htmlFor={id} className="text-sm font-medium">
        {label}
      </label>
      {readOnly ? (
        <Input id={id} type={type} value={value ?? ""} readOnly aria-readonly="true" className="bg-muted/50" autoComplete={autoComplete} />
      ) : onChange ? (
        <Input id={id} type={type} value={value ?? ""} onChange={(e) => onChange(e.target.value)} autoComplete={autoComplete} />
      ) : (
        <Input id={id} type={type} defaultValue={defaultValue} autoComplete={autoComplete} />
      )}
    </div>
  );
}

/* ----------------------------- Time zone ----------------------------- */

/** A select of every time zone the browser knows, with its current UTC offset; "" is the `unsetLabel` choice. */
function TimeZoneSelect({
  id,
  value,
  onChange,
  unsetLabel,
  disabled,
  describedBy,
}: {
  id: string;
  value: string;
  onChange: (zone: string) => void;
  unsetLabel: string;
  disabled?: boolean;
  describedBy?: string;
}) {
  const hydrated = useHydrated();
  // Offsets as of when the list was opened (a DST change mid-visit doesn't matter for picking a zone).
  const [openedAt] = useState(() => Date.now());
  const options = useMemo(() => {
    if (!hydrated) return [];
    return supportedTimeZones().map((z) => ({ value: z, label: `${z.replace(/_/g, " ")} (${formatUtcOffset(utcOffsetMinutes(openedAt, z))})` }));
  }, [hydrated, openedAt]);
  const known = !value || options.some((o) => o.value === value);
  return (
    <Select id={id} value={value} onChange={(e) => onChange(e.target.value)} disabled={disabled} aria-describedby={describedBy}>
      <option value="">{unsetLabel}</option>
      {!known && <option value={value}>{value}</option>}
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </Select>
  );
}

const sourceText = { user: "your preference", workspace: "the workspace default", browser: "this browser" } as const;

function TimeZoneCard({ live, zone }: { live: boolean; zone: ReturnType<typeof useReportingZone> }) {
  const id = useId();
  const browser = useBrowserTimeZone();
  const [value, setValue] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!live) return;
    let cancelled = false;
    getMyPreferences()
      .then((p) => {
        if (!cancelled) setValue(normalizeTimeZone(p?.timeZone) ?? "");
      })
      .catch((err) => {
        if (cancelled) return;
        setValue("");
        setLoadError(
          requestErrorStatus(err) === 404
            ? "This server can't store a personal time zone yet, so reports use the workspace default or this browser."
            : requestErrorText(err, "Couldn't load your time zone."),
        );
      });
    return () => {
      cancelled = true;
    };
  }, [live]);

  async function save() {
    if (!live) {
      toast({ tone: "info", title: "Demo workspace", description: DEMO_NOTE });
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await updateMyPreferences({ timeZone: value ? value : null });
      zone.refresh();
      toast({ tone: "success", title: "Time zone saved", description: "Reports now use it to decide which day each hour belongs to." });
    } catch (err) {
      setError(requestErrorText(err, "Couldn't save your time zone."));
    } finally {
      setSaving(false);
    }
  }

  const loading = live && value === null;
  return (
    <Card>
      <CardHeader>
        <div>
          <CardTitle>Time zone</CardTitle>
          <p className="mt-0.5 text-xs text-muted-foreground">Reports, timesheets and dashboards count each day in this zone.</p>
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-1.5">
          <label htmlFor={`${id}-zone`} className="text-sm font-medium">
            Time zone
          </label>
          <TimeZoneSelect
            id={`${id}-zone`}
            value={live ? (value ?? "") : ""}
            onChange={setValue}
            unsetLabel={`Workspace default, else this browser (${timeZoneLabel(browser)})`}
            disabled={loading || !live || !!loadError}
            describedBy={`${id}-zone-hint`}
          />
          <p id={`${id}-zone-hint`} className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <Globe aria-hidden className="h-3.5 w-3.5" />
            {zone.ready ? (
              <span>
                Reports now use <span className="font-medium text-foreground">{timeZoneLabel(zone.timeZone)}</span>, from{" "}
                {sourceText[zone.source]}.
              </span>
            ) : (
              <span>Checking your time zone…</span>
            )}
          </p>
          {!live && <p className="text-xs text-muted-foreground">In the demo workspace, reports use this browser&apos;s time zone.</p>}
          {loadError && <p className="text-xs text-warning-on-tint">{loadError}</p>}
          {error && (
            <p role="alert" className="text-xs text-danger-on-tint">
              {error}
            </p>
          )}
        </div>
        {live && (
          <div className="flex justify-end">
            <Button onClick={() => void save()} disabled={saving || loading || !!loadError}>
              {saving ? "Saving…" : "Save time zone"}
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

/* ----------------------------- Security ----------------------------- */

type Problem = "missing" | "short" | "same" | "mismatch" | null;

/** The shortest new value accepted here; the server's own policy may ask for more and says so. */
const MIN_LENGTH = 6;

function SecurityCard({ live }: { live: boolean }) {
  const id = useId();
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [problem, setProblem] = useState<Problem>(null);
  const [serverError, setServerError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    setDone(false);
    setServerError(null);
    const found: Problem = !current || !next || !confirm ? "missing" : next.length < MIN_LENGTH ? "short" : next === current ? "same" : next !== confirm ? "mismatch" : null;
    setProblem(found);
    if (found || !live) return;
    setBusy(true);
    try {
      await changeMyPassword({ currentPassword: current, newPassword: next });
      setCurrent("");
      setNext("");
      setConfirm("");
      setDone(true);
    } catch (err) {
      setServerError(requestErrorText(err, "Couldn't change it. Check the current one and try again."));
    } finally {
      setBusy(false);
    }
  }

  const errorId = `${id}-error`;
  return (
    <Card>
      <CardHeader>
        <div>
          <CardTitle>Change password</CardTitle>
          <p className="mt-0.5 text-xs text-muted-foreground">
            Use it to replace the one-time password you were given. The desktop agent asks you to sign in again afterwards.
          </p>
        </div>
      </CardHeader>
      <CardContent>
        {!live ? (
          <p className="rounded-xl bg-muted/60 px-3 py-3 text-sm text-muted-foreground">
            The demo workspace has no accounts to change. Sign in to your own workspace to change your password.
          </p>
        ) : (
          <form onSubmit={(e) => void submit(e)} className="max-w-md space-y-4" noValidate>
            <div className="space-y-1.5">
              <label htmlFor={`${id}-current`} className="text-sm font-medium">
                Current password
              </label>
              <Input
                id={`${id}-current`}
                type="password"
                autoComplete="current-password"
                value={current}
                onChange={(e) => setCurrent(e.target.value)}
                aria-invalid={problem === "missing" && !current ? true : undefined}
                aria-describedby={problem || serverError ? errorId : undefined}
              />
            </div>
            <div className="space-y-1.5">
              <label htmlFor={`${id}-next`} className="text-sm font-medium">
                New password
              </label>
              <Input
                id={`${id}-next`}
                type="password"
                autoComplete="new-password"
                value={next}
                onChange={(e) => setNext(e.target.value)}
                aria-invalid={problem === "short" || problem === "same" || (problem === "missing" && !next) ? true : undefined}
                aria-describedby={`${id}-rule${problem || serverError ? ` ${errorId}` : ""}`}
              />
              <p id={`${id}-rule`} className="text-xs text-muted-foreground">
                At least {MIN_LENGTH} characters. Your workspace may ask for more.
              </p>
            </div>
            <div className="space-y-1.5">
              <label htmlFor={`${id}-confirm`} className="text-sm font-medium">
                Confirm new password
              </label>
              <Input
                id={`${id}-confirm`}
                type="password"
                autoComplete="new-password"
                value={confirm}
                onChange={(e) => setConfirm(e.target.value)}
                aria-invalid={problem === "mismatch" || (problem === "missing" && !confirm) ? true : undefined}
                aria-describedby={problem || serverError ? errorId : undefined}
              />
            </div>
            {(problem || serverError) && (
              <p id={errorId} role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
                {problem === "missing" && <>Fill in all three fields.</>}
                {problem === "short" && <>The new password needs at least {MIN_LENGTH} characters.</>}
                {problem === "same" && <>Choose a new password that differs from the current one.</>}
                {problem === "mismatch" && <>The two new passwords don&apos;t match.</>}
                {!problem && serverError}
              </p>
            )}
            {done && (
              <p role="status" className="flex items-center gap-1.5 text-sm text-success-on-tint">
                <Check aria-hidden className="h-4 w-4" /> Your password was changed.
              </p>
            )}
            <Button type="submit" disabled={busy}>
              <KeyRound className="h-4 w-4" /> {busy ? "Changing…" : <>Change password</>}
            </Button>
          </form>
        )}
      </CardContent>
    </Card>
  );
}

/* ----------------------------- Workspace defaults ----------------------------- */

interface WorkspaceForm {
  defaultTimeZone: string;
  newProjectDefaults: NewProjectDefaultsDto;
}

function WorkspaceCard({ live, onSaved }: { live: boolean; onSaved: () => void }) {
  const id = useId();
  const browser = useBrowserTimeZone();
  const [form, setForm] = useState<WorkspaceForm | null>(live ? null : { defaultTimeZone: "", newProjectDefaults: BUILT_IN_PROJECT_DEFAULTS });
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      const s = await getWorkspaceSettings();
      setForm({
        defaultTimeZone: normalizeTimeZone(s?.defaultTimeZone) ?? "",
        newProjectDefaults: normalizeNewProjectDefaults(s?.newProjectDefaults),
      });
    } catch (err) {
      const status = requestErrorStatus(err);
      setLoadError(
        status === 404
          ? "This server doesn't support workspace defaults yet. New projects start with the standard settings."
          : status === 403
            ? "Only the workspace owner can change these settings."
            : requestErrorText(err, "Couldn't load the workspace settings."),
      );
    }
  }, []);

  useEffect(() => {
    if (!live) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void load();
  }, [live, load]);

  const errors = form ? validateTrackingSettings(form.newProjectDefaults) : {};

  async function save() {
    if (!form) return;
    if (!live) {
      toast({ tone: "info", title: "Demo workspace", description: DEMO_NOTE });
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const saved = await updateWorkspaceSettings({
        defaultTimeZone: form.defaultTimeZone || null,
        newProjectDefaults: form.newProjectDefaults,
      });
      if (saved && typeof saved === "object") {
        setForm({
          defaultTimeZone: normalizeTimeZone(saved.defaultTimeZone) ?? "",
          newProjectDefaults: normalizeNewProjectDefaults(saved.newProjectDefaults),
        });
      }
      onSaved();
      toast({ tone: "success", title: "Workspace defaults saved", description: "New projects start with these settings." });
    } catch (err) {
      setError(requestErrorText(err, "Couldn't save the workspace settings."));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <div>
          <CardTitle>Workspace defaults</CardTitle>
          <p className="mt-0.5 text-xs text-muted-foreground">
            The time zone reports use for anyone who hasn&apos;t chosen their own, and what a new project starts with. Existing projects
            keep their settings — change those on each project&apos;s page.
          </p>
        </div>
      </CardHeader>
      <CardContent className="space-y-6">
        {live && !form && !loadError && (
          <p role="status" className="text-sm text-muted-foreground">
            Loading workspace settings…
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
        {form && (
          <>
            <div className="space-y-1.5">
              <label htmlFor={`${id}-zone`} className="text-sm font-medium">
                Default time zone
              </label>
              <TimeZoneSelect
                id={`${id}-zone`}
                value={form.defaultTimeZone}
                onChange={(z) => setForm((f) => (f ? { ...f, defaultTimeZone: z } : f))}
                unsetLabel={`None — each person's browser (yours: ${timeZoneLabel(browser)})`}
                describedBy={`${id}-zone-hint`}
              />
              <p id={`${id}-zone-hint`} className="text-xs text-muted-foreground">
                People can still pick their own in Settings → Profile.
              </p>
            </div>
            <div className="space-y-2">
              <h3 className="text-sm font-semibold">New projects start with</h3>
              <TrackingFields
                draft={form.newProjectDefaults}
                errors={errors}
                onChange={(patch) => setForm((f) => (f ? { ...f, newProjectDefaults: { ...f.newProjectDefaults, ...patch } } : f))}
              />
            </div>
            {error && (
              <p role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
                {error}
              </p>
            )}
            <div className="flex flex-wrap items-center justify-between gap-3">
              <p className="text-xs text-muted-foreground">
                Existing projects:{" "}
                <Link href="/projects" className="font-medium text-primary-strong underline underline-offset-2">
                  edit each on its page
                </Link>
                .
              </p>
              <Button onClick={() => void save()} disabled={saving || Object.keys(errors).length > 0}>
                {saving ? "Saving…" : "Save workspace defaults"}
              </Button>
            </div>
          </>
        )}
      </CardContent>
    </Card>
  );
}

/* ----------------------------- Privacy ----------------------------- */

function PrivacyCard({ role }: { role: Role }) {
  const items: [string, string][] = [
    [
      "Screen captures",
      "When a project turns screenshots on: one capture per tracked block, at a random moment. A project can require blurring, and then the agent blurs the image before it leaves your computer.",
    ],
    [
      "Active app and window title",
      "When a project allows it: which app is in front and its window title — not what is inside it. Titles of other open windows are never uploaded.",
    ],
    [
      "Keyboard and mouse activity",
      "Counts only — never what you type. They give the activity %: the share of minutes with any input. Input injected by software (mouse jigglers, remote tools) is left out and shown separately.",
    ],
    [
      "Idle time",
      "After the project's idle limit with no input, the block ends. When you're back, the agent asks whether to keep the away time as work; if you don't answer, it's discarded.",
    ],
    ["Lock, sleep and breaks", "Nothing is tracked while your screen is locked, the computer sleeps, or you're paused or on a break."],
    [
      "Live status and device",
      "While it runs, the agent reports every minute whether it's tracking, paused or idle, plus your computer's name, operating system, agent version and time zone — so your team lead can see who is working now and keep agents up to date.",
    ],
    ["Webcam", "Off by default. Only when a project turns it on: one photo per block."],
  ];
  return (
    <Card>
      <CardHeader>
        <CardTitle>What the desktop agent collects</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">
          Each project decides what is captured — the workspace owner sets it, and the agent follows the project you track on.
        </p>
        <ul className="space-y-3 text-sm">
          {items.map(([title, body]) => (
            <li key={title} className="rounded-xl border border-border p-3">
              <div className="font-medium">{title}</div>
              <div className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{body}</div>
            </li>
          ))}
        </ul>
        {role === "worker" ? (
          <p className="text-xs text-muted-foreground">
            Questions about what your team enables? Ask your workspace owner. You can see each project&apos;s settings on its page.
          </p>
        ) : (
          <p className="text-xs text-muted-foreground">
            Change a project&apos;s capture settings with <span className="font-medium text-foreground">Edit project</span> on its page.
          </p>
        )}
      </CardContent>
    </Card>
  );
}

/* ----------------------------- Appearance ----------------------------- */

function ThemePicker() {
  const { theme, setTheme } = useTheme();
  const options = [
    { key: "light" as const, label: "Light", hint: "Always light", icon: Sun },
    { key: "dark" as const, label: "Dark", hint: "Always dark", icon: Moon },
    { key: "system" as const, label: "Match system", hint: "Use your computer's current setting", icon: Monitor },
  ];

  return (
    <div role="group" aria-label="Theme" className="grid grid-cols-1 gap-3 sm:grid-cols-3">
      {options.map((o) => {
        const Icon = o.icon;
        const activeChoice = o.key === "system" ? false : theme === o.key;
        return (
          <button
            key={o.key}
            type="button"
            aria-pressed={o.key === "system" ? undefined : activeChoice}
            onClick={() => {
              if (o.key === "system") {
                const sys = window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
                setTheme(sys);
              } else setTheme(o.key);
            }}
            className={cn(
              "flex flex-col items-center gap-1.5 rounded-xl border p-4 transition-colors",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              activeChoice ? "border-primary bg-primary/5" : "border-border hover:bg-muted/60",
            )}
          >
            <Icon aria-hidden className="h-6 w-6" />
            <span className="text-sm font-medium">{o.label}</span>
            <span className="text-[11px] text-muted-foreground">{o.hint}</span>
          </button>
        );
      })}
    </div>
  );
}
