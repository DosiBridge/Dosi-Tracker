"use client";

import { createContext, useContext, useEffect, useState } from "react";
import { users as primaryUsers } from "@/lib/mock-data";
import { datasetFor, setActiveWorkspace, hydrateLiveBackendData } from "@/lib/tenant-data";
import { primaryWorkspace, workspaces as seedWorkspaces, type PlanId, type Workspace } from "@/lib/saas-data";
import type { User, Activity } from "@/lib/types";
import { getApi, logoutApi } from "@/hooks/useApi";

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
}

const SessionContext = createContext<SessionContextValue | null>(null);
const USER_KEY = "dosi-user";
const WS_KEY = "dosi-workspace";
const WS_CREATED_KEY = "dosi-workspaces-created";
const WS_PATCH_KEY = "dosi-workspaces-patches";
const WS_DELETED_KEY = "dosi-workspaces-deleted";
const IMPERSONATE_KEY = "dosi-impersonating";

export function SessionProvider({ children }: { children: React.ReactNode }) {
  // Default to owner + primary workspace so SSR and first client render match.
  const [userId, setUserId] = useState<string>(primaryUsers[0].id);
  const [created, setCreated] = useState<Workspace[]>([]);
  const [patches, setPatches] = useState<Record<string, Partial<Workspace>>>({});
  const [deleted, setDeleted] = useState<string[]>([]);
  const [workspaceId, setWorkspaceId] = useState<string>(primaryWorkspace.id);
  const [impersonating, setImpersonating] = useState(false);

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
  }, []);

  // Hydrate global mock state with real backend API data
  useEffect(() => {
    async function hydrate() {
      try {
        // Null = "this fetch failed". A merely-unreachable backend must never
        // replace the active tenant's demo data with empty arrays, so failures
        // resolve to null sentinels instead of [] and hydration below only runs
        // when at least one payload is a real array.
        const [apiProjects, apiActivities, appConfig] = await Promise.all([
          getApi('/api/app/project').catch(() => null),
          getApi('/api/app/activity').catch(() => null),
          getApi('/api/abp/application-configuration?IncludeLocalizationResources=false').catch(() => null)
        ]);

        const currentUser = appConfig?.currentUser;
        if (currentUser?.isAuthenticated) {
          // Map the real ABP identity onto the app's role model:
          // host-side users run the platform; a tenant's "admin" role is the
          // workspace owner; everyone else tracks time as a worker.
          const roles: string[] = currentUser.roles ?? [];
          const role = currentUser.tenantId == null
            ? "host"
            : roles.includes("admin")
              ? "owner"
              : "worker";

          setUserId(currentUser.id);
          const mappedUser: User = {
            id: currentUser.id,
            name: currentUser.name || currentUser.userName,
            email: currentUser.email,
            role,
            designation: role === "host" ? "Host · Super Admin" : role === "owner" ? "Workspace Owner" : "Member",
            status: "active",
            timezone: "UTC",
            trackedToday: 0,
            productivity: 0,
            joinedAt: new Date().toISOString().slice(0,10)
          };
          // Add them to mock dataset if they aren't there
          if (!datasetFor(workspaceId).users.find(u => u.id === mappedUser.id)) {
             datasetFor(workspaceId).users.push(mappedUser);
          }
        }

        const hasProjects = Array.isArray(apiProjects);
        const hasActivities = Array.isArray(apiActivities);
        if (hasProjects || hasActivities) {
          const mappedActivities: Activity[] = (hasActivities ? apiActivities : []).map((a: any) => ({
            id: a.id,
            userId: a.userId,
            projectId: a.projectId,
            startedAt: a.startedAt,
            endedAt: a.endedAt,
            description: a.description || "Activity block",
            productivity: a.productivity || 0,
            mouseClicks: a.mouseClicks || 0,
            keyboardHits: a.keyboardHits || 0,
            activeWindows: a.activeWindowsJson ? JSON.parse(a.activeWindowsJson) : [],
            runningPrograms: a.runningProgramsJson ? JSON.parse(a.runningProgramsJson) : [],
            screen: { app: "System", kind: "desktop" as any, accent: "#1e293b" },
            hasWebcam: false,
            online: false,
          }));

          hydrateLiveBackendData(hasProjects ? apiProjects : [], mappedActivities);
          console.log("Global tenant data hydrated from ABP backend.");
        }
      } catch (err) {
        console.warn("Failed to hydrate tenant data from API:", err);
      }
    }
    
    // Only try to hydrate if we have a token
    if (localStorage.getItem('dosi-token')) {
      hydrate();
    }
  }, []);

  const allWorkspaces = [...seedWorkspaces, ...created]
    .filter((w) => !deleted.includes(w.id))
    .map((w) => (patches[w.id] ? { ...w, ...patches[w.id] } : w));

  const workspace = allWorkspaces.find((w) => w.id === workspaceId) ?? allWorkspaces[0] ?? primaryWorkspace;

  // Point the tenant data layer at the active workspace. Combined with the
  // remount key in the dashboard layout, every page re-reads this tenant's data.
  setActiveWorkspace(workspace.id);

  // Resolve the current user from the active tenant roster (+ host).
  // Do not keep primary-tenant users in the pool when viewing another workspace.
  const activeUsers = datasetFor(workspace.id).users;
  const pool: User[] = [hostUser, ...activeUsers];
  const user = pool.find((u) => u.id === userId) ?? activeUsers[0] ?? primaryUsers[0];

  const setUserById = (id: string) => {
    setUserId(id);
    try {
      localStorage.setItem(USER_KEY, id);
    } catch {}
  };

  const setWorkspaceById = (id: string) => {
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
        isImpersonating: impersonating,
        impersonate,
        stopImpersonating,
        loginAsHost,
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
