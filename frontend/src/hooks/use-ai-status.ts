"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { getAiStatus } from "./ai-api";
import { ApiError } from "./useApi";
import type { AiStatusDto } from "@/lib/ai-contract";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { currentWorkspaceId } from "@/lib/live-session";
import type { Role } from "@/lib/types";

/* ============================================================================
 * The caller's AI status: whether AI assist is on, and which features and
 * scopes the server allows them (lib/ai-assist aiAvailability reads it).
 *
 * A live session asks the server once per workspace: the answer is shared
 * through one module-level request keyed by the X-Workspace-Id it was sent
 * with, a workspace switch asks for the new one's, and `refresh()` asks again
 * (after Settings changed it). A
 * demo session never touches the network: it reads the demo's status for the
 * viewer's role. A status that can't be read (an older server, a failure)
 * is null, so every AI surface stays hidden instead of offering a button
 * that would only fail.
 * ========================================================================== */

export interface AiStatusState {
  /** null while loading, on an older server, or after a failed read: AI stays hidden. */
  status: AiStatusDto | null;
  /** A live session's status is being read (again). */
  loading: boolean;
  /** Read the status again (e.g. after the owner changed the AI settings). */
  refresh: () => void;
}

const requests = new Map<string, Promise<AiStatusDto | null>>();

const workspaceKey = () => currentWorkspaceId() ?? "";

function loadAiStatus(key: string): Promise<AiStatusDto | null> {
  let request = requests.get(key);
  if (!request) {
    request = getAiStatus().catch((err: unknown) => {
      // A failed read is retried by the next card that mounts; nothing personal is logged.
      requests.delete(key);
      console.warn("AI assist status couldn't be read; AI features stay hidden.", err instanceof ApiError ? `HTTP ${err.status}` : "unreachable");
      return null;
    });
    requests.set(key, request);
  }
  return request;
}

/** Forget every workspace's status (tests start each case from a clean slate). */
export function resetAiStatusCacheForTests(): void {
  requests.clear();
}

interface Loaded {
  generation: number;
  /** The session workspace the answer is for. */
  workspace: string;
  status: AiStatusDto | null;
}

/**
 * The AI status for the session: pass its `isLive`, the viewer's role (the
 * demo's status depends on it) and the session's workspace id (a switch reads
 * the new workspace's status; the previous one's never shows for it).
 */
export function useAiStatus({ live, role, workspaceId = "" }: { live: boolean; role: Role; workspaceId?: string }): AiStatusState {
  const [generation, setGeneration] = useState(0);
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const demo = useMemo(() => (live ? null : demoAiStatus(role)), [live, role]);

  useEffect(() => {
    if (!live) return;
    let current = true;
    void loadAiStatus(workspaceKey()).then((status) => {
      if (current) setLoaded({ generation, workspace: workspaceId, status });
    });
    return () => {
      current = false;
    };
  }, [live, generation, workspaceId]);

  const refresh = useCallback(() => {
    requests.delete(workspaceKey());
    setGeneration((g) => g + 1);
  }, []);

  if (!live) return { status: demo, loading: false, refresh };
  // While a refresh is under way the previous answer stays up (no flicker of every card), but only for the same workspace.
  const mine = loaded?.workspace === workspaceId ? loaded : null;
  return { status: mine?.status ?? null, loading: mine?.generation !== generation, refresh };
}
