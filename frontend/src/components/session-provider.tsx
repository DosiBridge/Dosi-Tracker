"use client";

import { createContext, useCallback, useContext, useEffect, useMemo, useReducer, useRef, useState, useSyncExternalStore } from "react";
import {
  datasetFor,
  demoDatasetsInstalled,
  dropLiveDatasets,
  installEmptyLiveDataset,
  installLiveDataset,
  setActiveWorkspace,
} from "@/lib/tenant-data";
import { demoWorkspaces as seededDemoWorkspaces, primaryDemoWorkspace, type PlanId, type Workspace } from "@/lib/saas-data";
import { runtimeConfig } from "@/lib/runtime-config";
import type { ActivityCoverage, User } from "@/lib/types";
import { getApi, getPagedApi, loginApi, logoutApi, refreshAccessToken } from "@/hooks/useApi";
import {
  SESSION_UNAUTHORIZED_EVENT,
  TOKEN_EXPIRES_STORAGE_KEY,
  TOKEN_STORAGE_KEY,
  WORKSPACE_STORAGE_KEY,
  canRenewSession,
  clearCurrentWorkspace,
  currentWorkspaceId,
  isLiveSession,
  readStorage,
  redirectToLogin,
  redirectToWorkspaceChooser,
  reloadPage,
  removeStorage,
  tokenExpiresAt,
  writeStorage,
} from "@/lib/live-session";
import { LIVE_PENDING_WORKSPACE_ID } from "@/lib/live-identity";
import {
  describeWarnings,
  fetchLiveIdentity,
  isSameIdentity,
  loadLiveSession,
  SessionExpiredError,
  WorkspaceRequiredError,
  type LiveIdentity,
  type LiveSessionData,
} from "@/lib/live-loader";
import { classifySignInError, type SignInFailure } from "@/lib/sign-in";

/** The platform operator. Not a member of any tenant. */
export const hostUser: User = {
  id: "host",
  name: "Platform Admin",
  email: "ops@dositracker.app",
  role: "host",
  designation: "Host · Super Admin",
  status: "active",
  timezone: "UTC",
  trackedToday: 0,
  productivity: 0,
  joinedAt: "2024-01-01",
};

/**
 * Where the session stands:
 *  - "unknown"    first render, before browser storage is read (SSR-safe),
 *                 and, in demo mode, while the demo data loads;
 *  - "demo"       no backend token and the deployment has demo mode on
 *                 (DOSI_DEMO_MODE): the built-in demo dataset;
 *  - "anonymous"  no backend token and no demo mode: a signed-out visitor
 *                 (a stale or forged session cookie included). Pages send
 *                 them to /login; nothing of the demo ever renders;
 *  - "loading"    token present, the real identity/workspace is being fetched;
 *  - "ready"      live session: real user, real workspace, real rows;
 *  - "error"      token present but the backend couldn't be reached. An error
 *                 state is shown, NEVER the demo dataset;
 *  - "signedout"  the user signed out (here or in another tab) and the page is
 *                 on its way to /login. Nothing of the old session — and no
 *                 demo data — renders in between.
 */
export type SessionStatus = "unknown" | "demo" | "anonymous" | "loading" | "ready" | "error" | "signedout";

/**
 * Where a live session's sign-in stands. With a refresh token the session
 * renews itself before the access token runs out and this stays "ok"; when it
 * can't (no refresh token, or it was refused) it turns "expiring" a few
 * minutes ahead — time to sign in again in place — and "expired" at the end.
 */
export type SessionExpiryPhase = "ok" | "expiring" | "expired";

export interface SessionExpiry {
  phase: SessionExpiryPhase;
  /** When the access token runs out (epoch ms), when known. */
  expiresAt: number | null;
}

export interface ReauthResult {
  ok: boolean;
  /** Why the sign-in failed (see lib/sign-in). */
  failure?: SignInFailure;
}

interface SessionContextValue {
  user: User;
  setUserById: (id: string) => void;
  /** End the session and leave for /login (nothing of the session, nor demo data, renders on the way). */
  logout: () => void;
  // Multi-tenant workspace context
  workspace: Workspace;
  workspaces: Workspace[];
  setWorkspaceById: (id: string) => void;
  createWorkspace: (name: string, planId: PlanId) => Workspace;
  updateWorkspace: (id: string, patch: Partial<Workspace>) => void;
  deleteWorkspace: (id: string) => void;
  // Host / platform
  isHost: boolean;
  isImpersonating: boolean;
  impersonate: (workspaceId: string) => void;
  stopImpersonating: () => void;
  loginAsHost: () => void;
  // Live (real backend) session
  /** A real backend token is in play: show only real data or honest empty/error states. */
  isLive: boolean;
  status: SessionStatus;
  /** Why the live session couldn't load (status "error"). */
  liveError: string | null;
  /** Non-fatal: some workspace data didn't load (the rest renders). */
  liveWarning: string | null;
  /** How complete the session's activity rows are (null outside a loaded live session). */
  liveCoverage: ActivityCoverage | null;
  /**
   * The signed-in user's granted permissions (ABP `grantedPolicies`, read with
   * every load), so screens know who may see what without asking the server
   * again; null outside a loaded live session.
   */
  livePolicies: Record<string, boolean> | null;
  /**
   * (Re)build the session from the backend. Call it after a client-side
   * sign-in, before navigating. Resolves to the signed-in user, or null in demo
   * mode / when the backend couldn't be reached. `silent` keeps the current
   * screen up while reloading (after editing data, or the periodic refresh):
   * it re-reads the roster, projects and totals but only the newest rows.
   */
  refreshSession: (options?: { silent?: boolean }) => Promise<User | null>;
  /** Whether the session can still renew itself, or must be signed in again soon / now. */
  expiry: SessionExpiry;
  /** Who a re-authentication signs in as (null outside a live session). */
  reauthIdentity: { username: string; workspace: string | null } | null;
  /** Sign in again in place, keeping the page (same account and workspace). */
  reauthenticate: (password: string) => Promise<ReauthResult>;
}

const SessionContext = createContext<SessionContextValue | null>(null);
const USER_KEY = "dosi-user";
/** Shared with the live session: a demo workspace id here is never sent (only GUIDs are). */
const WS_KEY = WORKSPACE_STORAGE_KEY;
const WS_CREATED_KEY = "dosi-workspaces-created";
const WS_PATCH_KEY = "dosi-workspaces-patches";
const WS_DELETED_KEY = "dosi-workspaces-deleted";
const IMPERSONATE_KEY = "dosi-impersonating";

/** Read the live session again this often while the tab is visible. */
const PULSE_INTERVAL_MS = 2 * 60 * 1000;
/** Coming back to the tab refreshes data older than this. */
const STALE_AFTER_MS = 60 * 1000;
/** Renew a renewable session this long before its access token runs out. */
const RENEW_LEAD_MS = 2 * 60 * 1000;
/** Warn this long before a session that can't renew itself runs out. */
const WARN_LEAD_MS = 5 * 60 * 1000;
/** A renewal that failed for now (offline, server error) is retried after this. */
const RENEW_RETRY_MS = 30 * 1000;
/** Browsers cap timers at 2^31-1 ms. */
const MAX_TIMER_MS = 2_147_483_647;

/** Placeholder identity while a live session loads (least privilege, never a demo person). */
const pendingLiveUser: User = {
  id: "live-pending",
  name: "Signing in",
  email: "",
  role: "worker",
  designation: "",
  status: "offline",
  timezone: "UTC",
  trackedToday: 0,
  productivity: 0,
  joinedAt: "",
};

const pendingLiveWorkspace: Workspace = {
  id: LIVE_PENDING_WORKSPACE_ID,
  name: "Workspace",
  slug: "workspace",
  planId: "enterprise",
  status: "active",
  color: "#0d9488",
  createdAt: "",
  cycleDays: 0,
  seatsUsed: 0,
  projectsUsed: 0,
  storageUsedGb: 0,
  // Not loaded yet: no plan name or seat limit is claimed.
  live: { known: false, planName: null, pricePerUser: null, seatLimit: null, trialEndsAt: null },
};

/** Shown while the page leaves for /login: a neutral identity, never a demo person. */
const signedOutUser: User = { ...pendingLiveUser, id: "signed-out", name: "Signed out" };

/** No session and no demo: no workspace at all (an empty, non-live dataset). */
const noWorkspace: Workspace = { ...pendingLiveWorkspace, id: "", name: "" };

interface LiveState {
  user: User;
  workspace: Workspace;
  identity: LiveIdentity;
}

/* ----------------------------- Demo persistence ----------------------------- */
// The demo session (who you are, which demo workspace, workspaces you created)
// is remembered in localStorage and read through useSyncExternalStore: server
// rendering and hydration use the defaults, the client then reads storage, and
// another tab's changes arrive through the `storage` event. Writes the browser
// refuses (blocked site data) are kept for this page view in `overlay`.

const DEMO_KEYS = [USER_KEY, WS_KEY, WS_CREATED_KEY, WS_PATCH_KEY, WS_DELETED_KEY, IMPERSONATE_KEY] as const;
type DemoKey = (typeof DEMO_KEYS)[number];
/** Same-tab notice that the session wrote storage (the `storage` event only reaches OTHER tabs). */
const DEMO_EVENT = "dosi-session-storage";
const ABSENT = "\u0000";

function subscribeDemo(onChange: () => void): () => void {
  window.addEventListener("storage", onChange);
  window.addEventListener(DEMO_EVENT, onChange);
  return () => {
    window.removeEventListener("storage", onChange);
    window.removeEventListener(DEMO_EVENT, onChange);
  };
}

const subscribeNothing = () => () => {};
const onClient = () => true;
const onServer = () => false;

/** The stored demo values as one comparable string (a stable snapshot for useSyncExternalStore). */
function demoSnapshot(): string {
  return JSON.stringify(DEMO_KEYS.map((key) => readStorage(key) ?? ABSENT));
}

const serverDemoSnapshot = () => "";

interface DemoState {
  userId: string | null;
  workspaceId: string | null;
  created: Workspace[];
  patches: Record<string, Partial<Workspace>>;
  deleted: string[];
  impersonating: boolean;
}

/** JSON of the expected shape, or the fallback: storage is user-editable, so wrong-type values must never crash a render. */
function parseJson<T>(raw: string | null, valid: (value: unknown) => boolean, fallback: T): T {
  if (!raw) return fallback;
  try {
    const parsed: unknown = JSON.parse(raw);
    return valid(parsed) ? (parsed as T) : fallback;
  } catch {
    return fallback;
  }
}

function parseDemoState(snapshot: string, overlay: Partial<Record<DemoKey, string | null>>): DemoState {
  let stored: unknown[] = [];
  try {
    stored = snapshot ? (JSON.parse(snapshot) as unknown[]) : [];
  } catch {
    stored = [];
  }
  const value = (key: DemoKey): string | null => {
    if (key in overlay) return overlay[key] ?? null;
    const raw = stored[DEMO_KEYS.indexOf(key)];
    return typeof raw === "string" && raw !== ABSENT ? raw : null;
  };
  const isObject = (v: unknown) => !!v && typeof v === "object" && !Array.isArray(v);
  return {
    userId: value(USER_KEY) || null,
    workspaceId: value(WS_KEY) || null,
    created: parseJson<Workspace[]>(value(WS_CREATED_KEY), Array.isArray, []).filter(isObject),
    patches: parseJson<Record<string, Partial<Workspace>>>(value(WS_PATCH_KEY), isObject, {}),
    deleted: parseJson<unknown[]>(value(WS_DELETED_KEY), Array.isArray, []).filter((id): id is string => typeof id === "string"),
    impersonating: value(IMPERSONATE_KEY) === "1",
  };
}

/* ----------------------------- Provider ----------------------------- */

export function SessionProvider({ children }: { children: React.ReactNode }) {
  // Demo state lives in storage; `overlay` holds what the browser refused to store.
  const snapshot = useSyncExternalStore(subscribeDemo, demoSnapshot, serverDemoSnapshot);
  const [overlay, setOverlay] = useState<Partial<Record<DemoKey, string | null>>>({});
  const demo = useMemo(() => parseDemoState(snapshot, overlay), [snapshot, overlay]);

  // Demo mode is the deployment's explicit choice (DOSI_DEMO_MODE, lib/runtime-config),
  // off by default. Its data (lib/demo-tenants) is loaded on demand, only then.
  const demoMode = runtimeConfig().demoMode;
  const [demoReady, setDemoReady] = useState(demoDatasetsInstalled);
  const [demoFailed, setDemoFailed] = useState(false);
  const demoActive = demoMode && demoReady;
  const userId = demo.userId;
  const workspaceId = demo.workspaceId ?? (demoActive ? primaryDemoWorkspace()?.id : undefined) ?? "";

  // Where the session stands is derived, not stored: rendering on the server
  // (and hydrating) says "unknown"; on the client a stored token means a live
  // session ("loading" until its first load settles). No token means the demo
  // where the deployment has demo mode on, else a signed-out visitor: a session
  // cookie without a token (stale, or set by hand) never shows anything.
  const hydrated = useSyncExternalStore(subscribeNothing, onClient, onServer);
  const tokenPresent = useSyncExternalStore(subscribeDemo, isLiveSession, onServer);
  const [liveStatus, setLiveStatus] = useState<"idle" | "loading" | "ready" | "error">("idle");
  const [signedOut, setSignedOut] = useState(false);
  const noTokenStatus: SessionStatus = !demoMode ? "anonymous" : demoFailed ? "error" : demoReady ? "demo" : "unknown";
  const status: SessionStatus = signedOut
    ? "signedout"
    : !hydrated
      ? "unknown"
      : liveStatus !== "idle"
        ? liveStatus
        : tokenPresent
          ? "loading"
          : noTokenStatus;
  /** A real backend token is in play (also while it loads, or failed to). */
  const isLive = hydrated && !signedOut && (liveStatus !== "idle" || tokenPresent);

  // Live session state: `live` holds the REAL user/workspace once loaded.
  const [live, setLive] = useState<LiveState | null>(null);
  const [liveError, setLiveError] = useState<string | null>(null);
  const [liveWarning, setLiveWarning] = useState<string | null>(null);
  const [liveCoverage, setLiveCoverage] = useState<ActivityCoverage | null>(null);
  const [livePolicies, setLivePolicies] = useState<Record<string, boolean> | null>(null);
  const [expiry, setExpiry] = useState<SessionExpiry>({ phase: "ok", expiresAt: null });
  // Bumped whenever the stored token may have changed, so the expiry watch starts over.
  const [tokenEpoch, bumpTokenEpoch] = useReducer((n: number) => n + 1, 0);
  // Only the latest load may commit: a slow mount-time load must not overwrite
  // the session a fresh sign-in just built.
  const loadSeq = useRef(0);
  // The last successful live load: what a silent refresh merges into, and whose identity cross-tab checks compare with.
  const lastLoad = useRef<LiveSessionData | null>(null);
  const lastLoadAt = useRef(0);
  const loadedWithToken = useRef<string | null>(null);
  const pulsing = useRef(false);
  const statusRef = useRef<SessionStatus>("unknown");
  const expiryRef = useRef<SessionExpiry>(expiry);

  useEffect(() => {
    statusRef.current = status;
  }, [status]);
  useEffect(() => {
    expiryRef.current = expiry;
  }, [expiry]);

  /** Write (or, with null, forget) a demo value; a refusal keeps it for this page view. */
  const persist = useCallback((key: DemoKey, value: string | null) => {
    let stored: boolean;
    if (value === null) {
      removeStorage(key);
      stored = readStorage(key) === null;
    } else {
      stored = writeStorage(key, value);
    }
    setOverlay((current) => {
      if (stored && !(key in current)) return current;
      const next = { ...current };
      if (stored) delete next[key];
      else next[key] = value;
      return next;
    });
    if (typeof window !== "undefined") window.dispatchEvent(new Event(DEMO_EVENT));
  }, []);

  /** Forget the live session in this tab (data, identity, timers' state). */
  const clearLiveState = useCallback(() => {
    loadSeq.current++; // abandon any in-flight live load
    lastLoad.current = null;
    loadedWithToken.current = null;
    dropLiveDatasets(); // a signed-out browser keeps none of the tenant's data
    setLive(null);
    setLiveStatus("idle");
    setLiveError(null);
    setLiveWarning(null);
    setLiveCoverage(null);
    setLivePolicies(null);
    setExpiry({ phase: "ok", expiresAt: null });
  }, []);

  /** The session ended (here, by the server, or in another tab): show nothing of it and leave for /login. */
  const endSession = useCallback(() => {
    logoutApi(); // Clear the real backend bearer token, not just the mock session.
    clearLiveState();
    setSignedOut(true);
    redirectToLogin();
  }, [clearLiveState]);

  /**
   * Load the live session and commit it, unless a newer load started since
   * (`seq`). Every state update happens after the network answers. A silent
   * load merges into the previous one (see loadLiveSession's pulse).
   */
  const runLoad = useCallback(
    async (seq: number, silent: boolean): Promise<User | null> => {
      const previous = silent ? lastLoad.current : null;
      const token = readStorage(TOKEN_STORAGE_KEY);
      const now = new Date();
      try {
        const data = await loadLiveSession(getApi, now, { getPage: getPagedApi, previous, workspaceId: currentWorkspaceId() });
        if (seq !== loadSeq.current) return null;
        // Another account signed in from another tab: start over rather than mix two sessions.
        if (lastLoad.current && !isSameIdentity(lastLoad.current.identity, data.identity)) {
          reloadPage();
          return null;
        }
        lastLoad.current = data;
        lastLoadAt.current = Date.now();
        loadedWithToken.current = token;
        installLiveDataset({
          workspaceId: data.workspace.id,
          users: data.users,
          projects: data.projects,
          activities: data.activities,
          now,
          timeZone: data.timeZone,
          coverage: data.coverage,
          totals: data.totals,
        });
        setLive({ user: data.user, workspace: data.workspace, identity: data.identity });
        setLiveWarning(describeWarnings(data.warnings));
        setLiveCoverage(data.coverage);
        setLivePolicies(data.grantedPolicies);
        setLiveError(null);
        setLiveStatus("ready");
        bumpTokenEpoch();
        return data.user;
      } catch (err) {
        if (seq !== loadSeq.current) return null;
        if (err instanceof WorkspaceRequiredError) {
          clearCurrentWorkspace();
          clearLiveState();
          setLiveError(err.message);
          setLiveStatus("error");
          redirectToWorkspaceChooser();
          return null;
        }
        if (err instanceof SessionExpiredError) {
          if (silent && lastLoad.current) {
            // The token ran out under a page someone is using: keep the page and ask them to sign in again.
            setExpiry({ phase: "expired", expiresAt: tokenExpiresAt() });
            return null;
          }
          // The stored token is no longer accepted: end the session cleanly.
          endSession();
          return null;
        }
        if (silent) return null; // keep what is on screen; the next refresh retries
        clearLiveState();
        setLiveError("We couldn't load your workspace from the server. Check your connection and try again.");
        setLiveStatus("error");
        return null;
      }
    },
    [clearLiveState, endSession],
  );

  /**
   * Build the session from the backend. It reads nothing from render-time
   * closures (the old hydration captured the first render's workspace id and
   * wrote the signed-in user into the wrong tenant), so it is safe to call at
   * any time: after a client-side sign-in, on Retry, and as a silent refresh.
   */
  const refreshSession = useCallback(
    async (options?: { silent?: boolean }): Promise<User | null> => {
      const seq = ++loadSeq.current;
      const silent = options?.silent === true;
      if (!isLiveSession()) {
        const s = statusRef.current;
        // (An "error" from the demo chunk failing to load is not a live session.)
        if (lastLoad.current || s === "loading" || s === "ready" || (s === "error" && !demoFailed)) {
          // The token went away under a live tab: that is a sign-out, never a fall back to the demo.
          endSession();
          return null;
        }
        // No token: the demo (loaded again after a failed attempt) — or, after a sign-out or
        // without demo mode, the signed-out screen until the page leaves.
        clearLiveState();
        setDemoFailed(false);
        return null;
      }
      setSignedOut(false);
      if (!silent) {
        setLiveStatus("loading");
        setLiveError(null);
        setLiveWarning(null);
        installEmptyLiveDataset(LIVE_PENDING_WORKSPACE_ID);
      }
      return runLoad(seq, silent);
    },
    [clearLiveState, endSession, runLoad, demoFailed],
  );

  /** A silent refresh, one at a time, and never while the sign-in has expired. */
  const pulse = useCallback(async () => {
    if (pulsing.current || statusRef.current !== "ready" || expiryRef.current.phase === "expired") return;
    pulsing.current = true;
    try {
      await refreshSession({ silent: true });
    } finally {
      pulsing.current = false;
    }
  }, [refreshSession]);

  /** The stored token changed under this tab: a renewal (same account) or another account's sign-in. */
  const checkIdentity = useCallback(async () => {
    const current = lastLoad.current?.identity;
    if (!current) return;
    let identity;
    try {
      identity = await fetchLiveIdentity(getApi, currentWorkspaceId());
    } catch {
      return; // unreachable for now; the next check or refresh decides
    }
    if (!identity) {
      setExpiry({ phase: "expired", expiresAt: tokenExpiresAt() });
      return;
    }
    if (!isSameIdentity(identity, current)) {
      reloadPage();
      return;
    }
    loadedWithToken.current = readStorage(TOKEN_STORAGE_KEY);
    setExpiry({ phase: "ok", expiresAt: tokenExpiresAt() });
    bumpTokenEpoch();
  }, []);

  // A stored token with no load behind it yet — the page just opened, or a
  // sign-in arrived from another tab — starts the live session. (A sign-in in
  // this tab calls refreshSession() itself.)
  useEffect(() => {
    if (!tokenPresent || signedOut || liveStatus !== "idle") return;
    installEmptyLiveDataset(LIVE_PENDING_WORKSPACE_ID);
    void runLoad(++loadSeq.current, false);
  }, [tokenPresent, signedOut, liveStatus, runLoad]);

  // Demo mode with no session: load the demo tenants (a separate chunk a live deployment never fetches).
  useEffect(() => {
    if (!hydrated || tokenPresent || signedOut || !demoMode || demoReady || demoFailed) return;
    let cancelled = false;
    import("@/lib/demo-tenants")
      .then((module) => {
        module.installDemoTenants();
        if (!cancelled) setDemoReady(true);
      })
      .catch(() => {
        if (cancelled) return;
        setLiveError("The demo couldn't be loaded. Check your connection and try again.");
        setDemoFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [hydrated, tokenPresent, signedOut, demoMode, demoReady, demoFailed]);

  // Other tabs share this browser's session: follow their sign-outs, sign-ins and renewals.
  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      const wasLive = statusRef.current === "loading" || statusRef.current === "ready" || statusRef.current === "error";
      // `key` is null when another tab cleared all of storage.
      if (event.key === null) {
        if (wasLive && !isLiveSession()) endSession();
        return;
      }
      if (event.key === TOKEN_STORAGE_KEY) {
        if (!event.newValue) {
          if (wasLive) endSession(); // signed out in another tab
        } else if (!wasLive && statusRef.current !== "unknown") {
          reloadPage(); // signed in elsewhere while this tab showed the demo or the signed-out screen
        } else if (wasLive) {
          void checkIdentity();
        }
        return;
      }
      if (event.key === WS_KEY && wasLive && event.newValue && event.newValue !== event.oldValue) {
        // Another tab switched workspace: this tab's screens belong to the old one.
        reloadPage();
        return;
      }
      if (event.key === TOKEN_EXPIRES_STORAGE_KEY && wasLive) bumpTokenEpoch();
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, [checkIdentity, endSession]);

  // Coming back to the tab: catch up on a sign-out or account change we missed, and refresh stale data.
  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === "hidden") return;
      const s = statusRef.current;
      if (s !== "ready" && s !== "loading") return;
      if (!isLiveSession()) {
        endSession();
        return;
      }
      if (readStorage(TOKEN_STORAGE_KEY) !== loadedWithToken.current) {
        void checkIdentity();
        return;
      }
      if (Date.now() - lastLoadAt.current > STALE_AFTER_MS) void pulse();
    };
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("focus", onVisible);
    return () => {
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("focus", onVisible);
    };
  }, [checkIdentity, endSession, pulse]);

  // A request the session couldn't renew for: keep the page and ask for the password in place
  // (cancelling the event tells the API layer not to sign out and leave for /login).
  useEffect(() => {
    const onUnauthorized = (event: Event) => {
      if (statusRef.current !== "ready") return;
      event.preventDefault();
      setExpiry({ phase: "expired", expiresAt: tokenExpiresAt() });
    };
    window.addEventListener(SESSION_UNAUTHORIZED_EVENT, onUnauthorized);
    return () => window.removeEventListener(SESSION_UNAUTHORIZED_EVENT, onUnauthorized);
  }, []);

  // While the tab is visible, keep "today", presence and project lists current.
  useEffect(() => {
    if (status !== "ready") return;
    const id = window.setInterval(() => {
      if (document.visibilityState !== "hidden") void pulse();
    }, PULSE_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [status, pulse]);

  // Keep the sign-in alive: renew ahead of expiry when a refresh token allows,
  // otherwise warn ahead of it and ask for the password in place at the end.
  useEffect(() => {
    if (status !== "ready") return;
    let cancelled = false;
    let timer: number | undefined;
    const later = (delay: number) => {
      timer = window.setTimeout(() => void check(), Math.min(MAX_TIMER_MS, Math.max(0, delay)));
    };
    const check = async () => {
      if (cancelled) return;
      const expiresAt = tokenExpiresAt();
      if (expiresAt === null) return; // an older sign-in that didn't record its expiry: nothing to schedule
      const left = expiresAt - Date.now();
      if (canRenewSession()) {
        if (left > RENEW_LEAD_MS) {
          setExpiry({ phase: "ok", expiresAt });
          later(left - RENEW_LEAD_MS);
          return;
        }
        const renewed = await refreshAccessToken();
        if (cancelled) return;
        if (renewed) {
          loadedWithToken.current = readStorage(TOKEN_STORAGE_KEY);
          later(0);
          return;
        }
        if (canRenewSession()) {
          // Offline or a server error: the refresh token is still good, so keep retrying — a
          // renewal that gets through later closes the sign-in prompt by itself.
          setExpiry({ phase: left > 0 ? "ok" : "expired", expiresAt });
          later(left > 0 ? Math.min(left, RENEW_RETRY_MS) : RENEW_RETRY_MS);
          return;
        }
        // Refused (expired or revoked): only signing in again will do.
      }
      if (left <= 0) {
        setExpiry({ phase: "expired", expiresAt });
        return;
      }
      setExpiry({ phase: left <= WARN_LEAD_MS ? "expiring" : "ok", expiresAt });
      later(left <= WARN_LEAD_MS ? left : left - WARN_LEAD_MS);
    };
    later(0);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [status, tokenEpoch]);

  // Outside demo mode there are no demo workspaces, not even ones a demo visit once created here.
  const demoWorkspaces = demoActive
    ? [...seededDemoWorkspaces(), ...demo.created]
        .filter((w) => !demo.deleted.includes(w.id))
        .map((w) => (demo.patches[w.id] ? { ...w, ...demo.patches[w.id] } : w))
    : [];

  // LIVE: exactly one workspace (the real tenant) and the real signed-in user.
  // No demo tenants, no demo roster, no demo impersonation.
  const liveWorkspace = live?.workspace ?? pendingLiveWorkspace;
  const allWorkspaces = isLive || signedOut ? [liveWorkspace] : demoWorkspaces;
  const workspace =
    isLive || signedOut
      ? liveWorkspace
      : (demoWorkspaces.find((w) => w.id === workspaceId) ?? demoWorkspaces[0] ?? noWorkspace);

  // Point the tenant data layer at the active workspace. Combined with the
  // remount key in the dashboard layout, every page re-reads this tenant's data.
  setActiveWorkspace(workspace.id);

  // Resolve the current user from the active tenant roster (+ host).
  // Do not keep primary-tenant users in the pool when viewing another workspace.
  // No token and no demo: nobody, whatever datasets this tab happens to hold.
  const anonymous = !isLive && !signedOut && !demoActive;
  const activeUsers = anonymous ? [] : datasetFor(workspace.id).users;
  const pool: User[] = [hostUser, ...activeUsers];
  const user =
    signedOut || anonymous
      ? signedOutUser
      : isLive
        ? (live?.user ?? pendingLiveUser)
        : (pool.find((u) => u.id === userId) ?? activeUsers[0] ?? signedOutUser);

  const setUserById = (id: string) => {
    if (isLive || signedOut) return; // identity comes from the backend in a live session
    persist(USER_KEY, id);
  };

  const setWorkspaceById = (id: string) => {
    if (isLive || signedOut) return; // a live session has exactly one workspace
    persist(WS_KEY, id);
    // If the current user isn't in the target tenant, land as that tenant's owner.
    if (userId !== hostUser.id) {
      const ds = datasetFor(id);
      if (!ds.users.some((u) => u.id === userId)) {
        const owner = ds.users.find((u) => u.role === "owner") ?? ds.users[0];
        if (owner) persist(USER_KEY, owner.id);
      }
    }
  };

  const persistImpersonating = (v: boolean) => persist(IMPERSONATE_KEY, v ? "1" : "0");

  const createWorkspace = (name: string, planId: PlanId): Workspace => {
    const ws: Workspace = {
      id: `w-${Date.now()}`,
      name,
      slug: name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/(^-|-$)/g, "") || "workspace",
      planId,
      status: planId === "free" ? "active" : "trialing",
      color: "#8b5cf6",
      createdAt: new Date().toISOString().slice(0, 10),
      cycleDays: planId === "free" ? 0 : 14,
      seatsUsed: 1,
      projectsUsed: 0,
      storageUsedGb: 0,
    };
    persist(WS_CREATED_KEY, JSON.stringify([...demo.created, ws]));
    setWorkspaceById(ws.id);
    return ws;
  };

  const updateWorkspace = (id: string, patch: Partial<Workspace>) => {
    persist(WS_PATCH_KEY, JSON.stringify({ ...demo.patches, [id]: { ...demo.patches[id], ...patch } }));
  };

  const deleteWorkspace = (id: string) => {
    persist(WS_DELETED_KEY, JSON.stringify([...demo.deleted, id]));
    persist(WS_CREATED_KEY, JSON.stringify(demo.created.filter((w) => w.id !== id)));
    const primary = primaryDemoWorkspace();
    if (workspaceId === id && primary) setWorkspaceById(primary.id);
  };

  const loginAsHost = () => {
    persistImpersonating(false);
    setUserById(hostUser.id);
  };

  const impersonate = (id: string) => {
    // Demo-only: a live host cannot become a tenant's owner from the browser.
    if (isLive || signedOut) return;
    const ds = datasetFor(id);
    const owner = ds.users.find((u) => u.role === "owner") ?? ds.users[0];
    setWorkspaceById(id);
    if (owner) setUserById(owner.id);
    persistImpersonating(true);
  };

  const stopImpersonating = () => {
    persistImpersonating(false);
    setUserById(hostUser.id);
  };

  const logout = () => {
    persist(USER_KEY, null);
    persist(IMPERSONATE_KEY, "0");
    endSession();
  };

  const reauthIdentity = useMemo(
    () =>
      live
        ? {
            username: live.identity.userName || live.user.email,
            workspace: live.identity.tenantId ? live.identity.tenantName : null,
          }
        : null,
    [live],
  );

  const reauthenticate = useCallback(
    async (password: string): Promise<ReauthResult> => {
      const last = lastLoad.current;
      if (!last) return { ok: false, failure: { kind: "server" } };
      const username = last.identity.userName || last.user.email;
      try {
        await loginApi(username, password);
      } catch (err) {
        return { ok: false, failure: classifySignInError(err) };
      }
      setExpiry({ phase: "ok", expiresAt: tokenExpiresAt() });
      await refreshSession({ silent: true });
      return { ok: true };
    },
    [refreshSession],
  );

  return (
    <SessionContext.Provider
      value={{
        user,
        setUserById,
        logout,
        workspace,
        workspaces: allWorkspaces,
        setWorkspaceById,
        createWorkspace,
        updateWorkspace,
        deleteWorkspace,
        isHost: user.role === "host",
        isImpersonating: !isLive && !signedOut && demo.impersonating,
        impersonate,
        stopImpersonating,
        loginAsHost,
        isLive,
        status,
        liveError,
        liveWarning,
        liveCoverage,
        livePolicies,
        refreshSession,
        expiry,
        reauthIdentity,
        reauthenticate,
      }}
    >
      {children}
    </SessionContext.Provider>
  );
}

export function useSession() {
  const ctx = useContext(SessionContext);
  if (!ctx) throw new Error("useSession must be used within SessionProvider");
  return ctx;
}
