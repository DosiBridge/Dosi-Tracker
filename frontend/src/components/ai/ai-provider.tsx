"use client";

import { createContext, useCallback, useContext, useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { useSession } from "@/components/session-provider";
import { useAiStatus } from "@/hooks/use-ai-status";
import type { AiResultCache } from "@/hooks/use-ai-request";
import type { AiPageContextDto, AiStatusDto } from "@/lib/ai-contract";
import { stableKey, type AiPromptChip } from "@/lib/ai-assist";
import type { Role } from "@/lib/types";

/* ============================================================================
 * AI ASSIST CONTEXT — mounted once by the dashboard layout, scoped to the
 * session's workspace: a workspace switch reads the new workspace's status
 * and drops every result of the previous one.
 *
 * It holds:
 *  - the caller's AI status (useAiStatus: read once, shared by every card);
 *  - the page view's result cache, so a card shows its last result again
 *    after navigating away and back (React state only, never web storage);
 *  - whether the Ask AI drawer is open, and what it was opened for (the
 *    setup assistant's questions, a suggested question to ask at once);
 *  - the page context cards register (useAiPageContext), which Ask AI sends
 *    with a question so it knows what the person is looking at.
 *
 * Without a provider (a component rendered on its own, e.g. in a test) every
 * hook answers with safe defaults: no status, so AI stays hidden.
 * ========================================================================== */

/** The page context Ask AI sends with a question, registered by whatever the page shows. */
export interface AiPageContextStore {
  get(): AiPageContextDto | null;
  /** Register a context; returns the token to clear it with. */
  set(context: AiPageContextDto): number;
  /** Clear the context registered with `token` (a newer registration is kept). */
  clear(token: number): void;
  subscribe(onChange: () => void): () => void;
}

/** What the Ask AI drawer was last opened for. */
export interface AiAssistantRequest {
  /** Bumped by every openAssistant(): the drawer acts on each request once. */
  seq: number;
  /** "setup": the setup assistant's help questions instead of the page's. */
  chips: "page" | "setup";
  /** A suggested question to ask as soon as the drawer opens. */
  prompt: AiPromptChip | null;
}

export interface AiAssistantOpenOptions {
  chips?: "page" | "setup";
  prompt?: AiPromptChip | null;
}

export interface AiAssistantValue {
  status: AiStatusDto | null;
  statusLoading: boolean;
  refreshStatus: () => void;
  /** A real backend session: requests go to the server. Otherwise the demo's sample output. */
  live: boolean;
  role: Role;
  userId: string;
  workspaceId: string;
  /** This page view's results (never web storage). */
  cache: AiResultCache;
  drawerOpen: boolean;
  setDrawerOpen: (open: boolean) => void;
  /** Open the Ask AI drawer, optionally with the setup questions or a question to ask at once. */
  openAssistant: (options?: AiAssistantOpenOptions) => void;
  drawerRequest: AiAssistantRequest;
  pageContext: AiPageContextStore | null;
}

const noCache: AiResultCache = { get: () => undefined, set: () => {} };

const DEFAULT_VALUE: AiAssistantValue = {
  status: null,
  statusLoading: false,
  refreshStatus: () => {},
  live: false,
  role: "worker",
  userId: "",
  workspaceId: "",
  cache: noCache,
  drawerOpen: false,
  setDrawerOpen: () => {},
  openAssistant: () => {},
  drawerRequest: { seq: 0, chips: "page", prompt: null },
  pageContext: null,
};

const AiAssistantContext = createContext<AiAssistantValue>(DEFAULT_VALUE);

function createPageContextStore(): AiPageContextStore {
  let current: { token: number; context: AiPageContextDto } | null = null;
  let next = 0;
  const listeners = new Set<() => void>();
  const notify = () => listeners.forEach((l) => l());
  return {
    get: () => current?.context ?? null,
    set(context) {
      current = { token: ++next, context };
      notify();
      return current.token;
    },
    clear(token) {
      if (current?.token !== token) return;
      current = null;
      notify();
    },
    subscribe(onChange) {
      listeners.add(onChange);
      return () => listeners.delete(onChange);
    },
  };
}

function createResultCache(): AiResultCache {
  const results = new Map<string, unknown>();
  return { get: (key) => results.get(key), set: (key, value) => void results.set(key, value) };
}

export function AiAssistantProvider({ children }: { children: React.ReactNode }) {
  const { isLive, user, workspace } = useSession();
  const { status, loading, refresh } = useAiStatus({ live: isLive, role: user.role, workspaceId: workspace.id });
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [drawerRequest, setDrawerRequest] = useState<AiAssistantRequest>(DEFAULT_VALUE.drawerRequest);
  const openAssistant = useCallback((options: AiAssistantOpenOptions = {}) => {
    setDrawerRequest((r) => ({ seq: r.seq + 1, chips: options.chips ?? "page", prompt: options.prompt ?? null }));
    setDrawerOpen(true);
  }, []);
  const [scope, setScope] = useState(() => ({ workspaceId: workspace.id, cache: createResultCache() }));
  // A workspace switch drops the previous workspace's results and closes the drawer, without
  // remounting the shell around it (its toasts, menus and sidebar stay as they are).
  if (scope.workspaceId !== workspace.id) {
    setScope({ workspaceId: workspace.id, cache: createResultCache() });
    setDrawerOpen(false);
  }
  const cache = scope.cache;
  const [pageContext] = useState(createPageContextStore);

  const value = useMemo<AiAssistantValue>(
    () => ({
      status,
      statusLoading: loading,
      refreshStatus: refresh,
      live: isLive,
      role: user.role,
      userId: user.id,
      workspaceId: workspace.id,
      cache,
      drawerOpen,
      setDrawerOpen,
      openAssistant,
      drawerRequest,
      pageContext,
    }),
    [status, loading, refresh, isLive, user.role, user.id, workspace.id, cache, drawerOpen, openAssistant, drawerRequest, pageContext],
  );
  return <AiAssistantContext.Provider value={value}>{children}</AiAssistantContext.Provider>;
}

/** The AI assist context; safe defaults (AI hidden) when no provider is mounted. */
export function useAiAssistant(): AiAssistantValue {
  return useContext(AiAssistantContext);
}

/**
 * Tell Ask AI what this page shows (the report, period, member or project in
 * view). Registered while the component is mounted; pass null to register
 * nothing. Re-registers only when the context's content changes.
 */
export function useAiPageContext(context: AiPageContextDto | null): void {
  const store = useContext(AiAssistantContext).pageContext;
  // The key is the context's canonical JSON: registering from it keeps the effect off object identity.
  const key = context ? stableKey(context) : "";
  useEffect(() => {
    if (!store || !key) return;
    const token = store.set(JSON.parse(key) as AiPageContextDto);
    return () => store.clear(token);
  }, [store, key]);
}

const noSubscription = () => () => {};
const nothing = () => null;

/** The page context registered right now (what Ask AI will send); null when none. */
export function useAiPageContextValue(): AiPageContextDto | null {
  const store = useContext(AiAssistantContext).pageContext;
  const subscribe = useCallback((onChange: () => void) => (store ? store.subscribe(onChange) : noSubscription()), [store]);
  return useSyncExternalStore(subscribe, store ? store.get : nothing, nothing);
}
