"use client";

import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { users as primaryUsers } from "@/lib/mock-data";
import { datasetFor, dropLiveDatasets, installEmptyLiveDataset, installLiveDataset, setActiveWorkspace } from "@/lib/tenant-data";
import { primaryWorkspace, workspaces as seedWorkspaces, type PlanId, type Workspace } from "@/lib/saas-data";
import type { User } from "@/lib/types";
import { getApi, logoutApi } from "@/hooks/useApi";
import { isLiveSession } from "@/lib/live-session";
import { LIVE_PENDING_WORKSPACE_ID } from "@/lib/live-identity";
import { describeWarnings, loadLiveSession, SessionExpiredError } from "@/lib/live-loader";

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
 *  - "unknown"  first render, before browser storage is read (SSR-safe);
 *  - "demo"     no backend token: the built-in demo dataset;
 *  - "loading"  token present, the real identity/workspace is being fetched;
 *  - "ready"    live session: real user, real workspace, real rows;
 *  - "error"    token present but the backend couldn't be reached. An error
 *               state is shown, NEVER the demo dataset.
 */
export type SessionStatus = "unknown" | "demo" | "loading" | "ready" | "error";

interface SessionContextValue {
  user: User;
  setUserById: (id: string) => void;
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
  /**
   * (Re)build the session from the backend. Call it after a client-side
   * sign-in, before navigating. Resolves to the signed-in user, or null in demo
   * mode / when the backend couldn't be reached. `silent` keeps the current
   * screen up while reloading (e.g. after editing your own profile).
   */
  refreshSession: (options?: { silent?: boolean }) => Promise<User | null>;
}

const SessionContext = createContext<SessionContextValue | null>(null);
const USER_KEY = "dosi-user";
const WS_KEY = "dosi-workspace";
const WS_CREATED_KEY = "dosi-workspaces-created";
const WS_PATCH_KEY = "dosi-workspaces-patches";
const WS_DELETED_KEY = "dosi-workspaces-deleted";
const IMPERSONATE_KEY = "dosi-impersonating";

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
  planId: "free",
  status: "active",
  color: "#0d9488",
  createdAt: "",
  cycleDays: 0,
  seatsUsed: 0,
  projectsUsed: 0,
  storageUsedGb: 0,
};

interface LiveState {
  user: User;
  workspace: Workspace;
}

export function SessionProvider({ children }: { children: React.ReactNode }) {
  // Default to owner + primary workspace so SSR and first client render match.
  const [userId, setUserId] = useState<string>(primaryUsers[0].id);
  const [created, setCreated] = useState<Workspace[]>([]);
  const [patches, setPatches] = useState<Record<string, Partial<Workspace>>>({});
  const [deleted, setDeleted] = useState<string[]>([]);
  const [workspaceId, setWorkspaceId] = useState<string>(primaryWorkspace.id);
  const [impersonating, setImpersonating] = useState(false);

  // Live session state: `live` holds the REAL user/workspace once loaded.
  const [status, setStatus] = useState<SessionStatus>("unknown");
  const [live, setLive] = useState<LiveState | null>(null);
  const [liveError, setLiveError] = useState<string | null>(null);
  const [liveWarning, setLiveWarning] = useState<string | null>(null);
  // Only the latest load may commit: a slow mount-time load must not overwrite
  // the session a fresh sign-in just built.
  const loadSeq = useRef(0);

  /**
   * Build the session from the backend. It reads nothing from render-time
   * closures (the old hydration captured the first render's workspace id and
   * wrote the signed-in user into the wrong tenant), so it is safe to call at
   * any time: on mount, and again after a client-side sign-in.
   */
  const refreshSession = useCallback(async (options?: { silent?: boolean }): Promise<User | null> => {
    const seq = ++loadSeq.current;
    const silent = options?.silent === true;
    if (!isLiveSession()) {
      setLive(null);
      setLiveError(null);
      setLiveWarning(null);
      setStatus("demo");
      return null;
    }
    if (!silent) {
      setStatus("loading");
      setLiveError(null);
      setLiveWarning(null);
      installEmptyLiveDataset(LIVE_PENDING_WORKSPACE_ID);
    }
    try {
      const now = new Date();
      const data = await loadLiveSession(getApi, now);
      if (seq !== loadSeq.current) return null;
      installLiveDataset({
        workspaceId: data.workspace.id,
        users: data.users,
        projects: data.projects,
        activities: data.activities,
        now,
      });
      setLive({ user: data.user, workspace: data.workspace });
      setLiveWarning(describeWarnings(data.warnings));
      setStatus("ready");
      return data.user;
    } catch (err) {
      if (seq !== loadSeq.current) return null;
      if (err instanceof SessionExpiredError) {
        // The stored token is no longer accepted: end the session cleanly.
        logoutApi();
        dropLiveDatasets();
        setLive(null);
        setStatus("demo");
        if (typeof window !== "undefined" && !window.location.pathname.startsWith("/login")) {
          window.location.href = "/login";
        }
        return null;
      }
      if (silent) return null; // keep what is on screen; the next full load retries
      setLive(null);
      setLiveError("We couldn't load your workspace from the server. Check your connection and try again.");
      setStatus("error");
      return null;
    }
  }, []);

  useEffect(() => {
    const storedUser = localStorage.getItem(USER_KEY);
    if (storedUser) setUserId(storedUser);

    // Guard every parse: localStorage is user-editable, so a corrupted value
    // (wrong-type JSON like "{}", "null" or "0") must fall back to the default
    // instead of crashing the render with a spread/`.includes` TypeError.
    try {
      const rawCreated = localStorage.getItem(WS_CREATED_KEY);
      if (rawCreated) {
        const parsedCreated: unknown = JSON.parse(rawCreated);
        if (Array.isArray(parsedCreated)) setCreated(parsedCreated as Workspace[]);
      }
      const rawPatch = localStorage.getItem(WS_PATCH_KEY);
      if (rawPatch) {
        const parsedPatches: unknown = JSON.parse(rawPatch);
        if (parsedPatches && typeof parsedPatches === "object" && !Array.isArray(parsedPatches)) {
          setPatches(parsedPatches as Record<string, Partial<Workspace>>);
        }
      }
      const rawDeleted = localStorage.getItem(WS_DELETED_KEY);
      if (rawDeleted) {
        const parsedDeleted: unknown = JSON.parse(rawDeleted);
        if (Array.isArray(parsedDeleted)) setDeleted(parsedDeleted as string[]);
      }
    } catch {}

    const storedWs = localStorage.getItem(WS_KEY);
    if (storedWs) setWorkspaceId(storedWs);
    setImpersonating(localStorage.getItem(IMPERSONATE_KEY) === "1");

    // Live session (token present) or demo. Runs on every mount (a full
    // reload) and, through refreshSession(), after every client-side sign-in.
    void refreshSession();
  }, [refreshSession]);

  const isLive = status === "loading" || status === "ready" || status === "error";

  const demoWorkspaces = [...seedWorkspaces, ...created]
    .filter((w) => !deleted.includes(w.id))
    .map((w) => (patches[w.id] ? { ...w, ...patches[w.id] } : w));

  // LIVE: exactly one workspace (the real tenant) and the real signed-in user.
  // No demo tenants, no demo roster, no demo impersonation.
  const liveWorkspace = live?.workspace ?? pendingLiveWorkspace;
  const allWorkspaces = isLive ? [liveWorkspace] : demoWorkspaces;
  const workspace = isLive
    ? liveWorkspace
    : (demoWorkspaces.find((w) => w.id === workspaceId) ?? demoWorkspaces[0] ?? primaryWorkspace);

  // Point the tenant data layer at the active workspace. Combined with the
  // remount key in the dashboard layout, every page re-reads this tenant's data.
  setActiveWorkspace(workspace.id);

  // Resolve the current user from the active tenant roster (+ host).
  // Do not keep primary-tenant users in the pool when viewing another workspace.
  const activeUsers = datasetFor(workspace.id).users;
  const pool: User[] = [hostUser, ...activeUsers];
  const user = isLive
    ? (live?.user ?? pendingLiveUser)
    : (pool.find((u) => u.id === userId) ?? activeUsers[0] ?? primaryUsers[0]);

  const setUserById = (id: string) => {
    if (isLive) return; // identity comes from the backend in a live session
    setUserId(id);
    try {
      localStorage.setItem(USER_KEY, id);
    } catch {}
  };

  const setWorkspaceById = (id: string) => {
    if (isLive) return; // a live session has exactly one workspace
    setWorkspaceId(id);
    try {
      localStorage.setItem(WS_KEY, id);
    } catch {}
    // If the current user isn't in the target tenant, land as that tenant's owner.
    if (userId !== hostUser.id) {
      const ds = datasetFor(id);
      if (!ds.users.some((u) => u.id === userId)) {
        const owner = ds.users.find((u) => u.role === "owner") ?? ds.users[0];
        if (owner) {
          setUserId(owner.id);
          try {
            localStorage.setItem(USER_KEY, owner.id);
          } catch {}
        }
      }
    }
  };

  const persistCreated = (next: Workspace[]) => {
    setCreated(next);
    try {
      localStorage.setItem(WS_CREATED_KEY, JSON.stringify(next));
    } catch {}
  };
  const persistPatches = (next: Record<string, Partial<Workspace>>) => {
    setPatches(next);
    try {
      localStorage.setItem(WS_PATCH_KEY, JSON.stringify(next));
    } catch {}
  };
  const persistDeleted = (next: string[]) => {
    setDeleted(next);
    try {
      localStorage.setItem(WS_DELETED_KEY, JSON.stringify(next));
    } catch {}
  };
  const persistImpersonating = (v: boolean) => {
    setImpersonating(v);
    try {
      localStorage.setItem(IMPERSONATE_KEY, v ? "1" : "0");
    } catch {}
  };

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
    persistCreated([...created, ws]);
    setWorkspaceById(ws.id);
    return ws;
  };

  const updateWorkspace = (id: string, patch: Partial<Workspace>) => {
    persistPatches({ ...patches, [id]: { ...patches[id], ...patch } });
  };

  const deleteWorkspace = (id: string) => {
    persistDeleted([...deleted, id]);
    persistCreated(created.filter((w) => w.id !== id));
    if (workspaceId === id) setWorkspaceById(primaryWorkspace.id);
  };

  const loginAsHost = () => {
    persistImpersonating(false);
    setUserById(hostUser.id);
  };

  const impersonate = (id: string) => {
    // Demo-only: a live host cannot become a tenant's owner from the browser.
    if (isLive) return;
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
    logoutApi(); // Clear the real backend bearer token, not just the mock session.
    try {
      localStorage.removeItem(USER_KEY);
      localStorage.setItem(IMPERSONATE_KEY, "0");
    } catch {}
    setImpersonating(false);
    setUserId(primaryUsers[0].id);
    loadSeq.current++; // abandon any in-flight live load
    dropLiveDatasets(); // a signed-out browser keeps none of the tenant's data
    setLive(null);
    setLiveError(null);
    setLiveWarning(null);
    setStatus("demo");
  };

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
        isImpersonating: !isLive && impersonating,
        impersonate,
        stopImpersonating,
        loginAsHost,
        isLive,
        status,
        liveError,
        liveWarning,
        refreshSession,
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
