"use client";

import { useCallback, useSyncExternalStore } from "react";

/* ============================================================================
 * A small per-viewer preference (e.g. how a report formats hours), remembered
 * in localStorage. Read through useSyncExternalStore, so server rendering and
 * the first client render agree (both use the fallback) and every component
 * using the same key stays in step. Storage that is unavailable — private
 * mode, blocked site data — only means the choice lasts for this page view.
 * ========================================================================== */

const EVENT = "dosi-stored-choice";

/** This page view's choices: they apply even when storage refuses the write. */
const memory = new Map<string, string>();

function read(key: string): string | null {
  const chosen = memory.get(key);
  if (chosen !== undefined) return chosen;
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function subscribe(onChange: () => void): () => void {
  window.addEventListener("storage", onChange);
  window.addEventListener(EVENT, onChange);
  return () => {
    window.removeEventListener("storage", onChange);
    window.removeEventListener(EVENT, onChange);
  };
}

export function useStoredChoice<T extends string>(key: string, allowed: readonly T[], fallback: T): [T, (value: T) => void] {
  const stored = useSyncExternalStore(
    subscribe,
    () => read(key),
    () => null,
  );
  const value = stored !== null && (allowed as readonly string[]).includes(stored) ? (stored as T) : fallback;
  const set = useCallback(
    (next: T) => {
      memory.set(key, next);
      try {
        window.localStorage.setItem(key, next);
      } catch {
        // Not remembered for next time, but applied to this page view.
      }
      window.dispatchEvent(new Event(EVENT));
    },
    [key],
  );
  return [value, set];
}

/** Forget this page view's choices (tests start each case from a clean slate). */
export function clearStoredChoices(): void {
  memory.clear();
}
