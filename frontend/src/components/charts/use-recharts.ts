"use client";

import { useSyncExternalStore } from "react";
import type { Recharts } from "./recharts-kit";

/*
 * recharts, loaded once per page view through one dynamic import (see
 * recharts-kit.ts for why). A chart calls useRecharts() and draws a
 * placeholder (ChartLoading) until the kit is there; the download starts as
 * soon as a module with charts is evaluated in the browser, alongside the
 * page's own data, so the placeholder is rarely seen for long.
 *
 * The server and the hydration pass always see "not loaded" (the server can't
 * draw a chart anyway: it has no layout to measure), so the markup they agree
 * on never depends on how fast the download was.
 */

let kit: Recharts | null = null;
let pending: Promise<Recharts> | null = null;
const listeners = new Set<() => void>();

/** Load the kit (once); resolves with it. A failed download is tried again by the next caller. */
export function loadRecharts(): Promise<Recharts> {
  if (!pending) {
    pending = import("./recharts-kit").then(
      (module) => {
        kit = module.recharts;
        listeners.forEach((notify) => notify());
        return kit;
      },
      (error: unknown) => {
        pending = null;
        throw error;
      },
    );
  }
  return pending;
}

/**
 * The kit for code that only runs inside a drawn chart — a custom bar shape,
 * a tick renderer — where useRecharts() has already returned it.
 */
export function loadedRecharts(): Recharts {
  if (!kit) throw new Error("recharts is used before it has loaded");
  return kit;
}

function subscribe(onChange: () => void): () => void {
  listeners.add(onChange);
  if (!kit) loadRecharts().catch(() => {
    // Offline or a failed deploy: the chart keeps its placeholder; the next mount retries.
  });
  return () => {
    listeners.delete(onChange);
  };
}

const currentKit = () => kit;
const notLoadedOnServer = () => null;

/** recharts' components, or null while they download (draw ChartLoading then). */
export function useRecharts(): Recharts | null {
  return useSyncExternalStore(subscribe, currentKit, notLoadedOnServer);
}

// Start the download as soon as a page with charts runs in the browser.
if (typeof window !== "undefined") {
  loadRecharts().catch(() => {
    // Retried when a chart mounts.
  });
}
