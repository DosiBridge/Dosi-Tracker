"use client";

import { useCallback, useEffect, useEffectEvent, useState } from "react";
import { ApiError, getTeamPresence } from "@/hooks/useApi";
import type { PresenceDto } from "@/lib/api-contract";

/* ============================================================================
 * Live refresh for the pages that show who is working NOW (Member Monitor,
 * Activity): a timer that only runs while the tab is visible, and the team's
 * presence (GET /api/app/presence/team) kept fresh on it.
 *
 * A hidden tab makes no requests. When it becomes visible again after a full
 * interval or more, it refreshes at once instead of showing stale states
 * until the next tick.
 * ========================================================================== */

/** Run `callback` every `intervalMs` while the page is visible (and `enabled`). */
export function useVisibleInterval(callback: () => void, intervalMs: number, enabled: boolean): void {
  const tick = useEffectEvent(callback);
  useEffect(() => {
    if (!enabled || !(intervalMs > 0)) return;
    let timer: number | null = null;
    let last = Date.now();
    const run = () => {
      last = Date.now();
      tick();
    };
    const start = () => {
      if (timer === null) timer = window.setInterval(run, intervalMs);
    };
    const stop = () => {
      if (timer !== null) window.clearInterval(timer);
      timer = null;
    };
    const onVisibility = () => {
      if (document.visibilityState === "hidden") {
        stop();
        return;
      }
      if (Date.now() - last >= intervalMs) run();
      start();
    };
    if (document.visibilityState !== "hidden") start();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [enabled, intervalMs]);
}

export interface TeamPresence {
  /** Every member device the server reported (null until the first answer arrives). */
  devices: PresenceDto[] | null;
  /** The server has no presence endpoint (an older server): the page derives presence from uploads instead. */
  unsupported: boolean;
  /** The latest refresh failed (the previous answer, if any, is kept). */
  failed: boolean;
  /** When the last answer arrived (epoch ms). */
  updatedAt: number | null;
  /** Ask again now. */
  refresh: () => void;
}

interface PresenceResult {
  devices: PresenceDto[] | null;
  unsupported: boolean;
  failed: boolean;
  updatedAt: number | null;
}

const INITIAL: PresenceResult = { devices: null, unsupported: false, failed: false, updatedAt: null };

/**
 * The team's presence, refreshed every `intervalMs` while the tab is visible.
 * Pass `enabled: isLive` — a demo session never touches the network.
 */
export function useTeamPresence(enabled: boolean, intervalMs: number): TeamPresence {
  const [result, setResult] = useState<PresenceResult>(INITIAL);
  const [generation, setGeneration] = useState(0);
  const refresh = useCallback(() => setGeneration((g) => g + 1), []);
  const unsupported = result.unsupported;

  useEffect(() => {
    if (!enabled || unsupported) return;
    let current = true;
    getTeamPresence()
      .then((devices) => {
        if (current) setResult({ devices, unsupported: false, failed: false, updatedAt: Date.now() });
      })
      .catch((err: unknown) => {
        if (!current) return;
        // 404/405: this server predates presence — stop asking and fall back to the uploads.
        const missing = err instanceof ApiError && (err.status === 404 || err.status === 405);
        setResult((prev) => ({ ...prev, unsupported: missing, failed: true }));
      });
    return () => {
      current = false;
    };
  }, [enabled, unsupported, generation]);

  useVisibleInterval(refresh, intervalMs, enabled && !unsupported);

  return { ...result, refresh };
}
