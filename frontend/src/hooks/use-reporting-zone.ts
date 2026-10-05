"use client";

import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { getApi } from "./useApi";
import { API_ROUTES } from "@/lib/api-contract";
import { browserTimeZone, effectiveTimeZone, type TimeZoneSource } from "@/lib/time-zone";

/* ============================================================================
 * The reporting time zone: every "day" a report shows is a calendar day here,
 * and the same zone is sent to the backend (contract §6) so both bucket alike.
 *
 * The viewer's own preference wins, then the workspace default, then the
 * browser's zone (contract §7). A demo session never touches the network and
 * reports in the browser's zone. An older server without these settings, or a
 * member who may not read the workspace's, simply falls through to the next
 * source.
 *
 * While a live session's settings load, the browser's zone stands in and
 * `ready` is false: a report that buckets by day should wait for `ready`, so
 * it isn't fetched once in the wrong zone and again in the right one.
 * ========================================================================== */

export interface ReportingZone {
  /** IANA zone id, e.g. "Asia/Dhaka". */
  timeZone: string;
  /** Where it came from: the viewer's preference, the workspace default, or the browser. */
  source: TimeZoneSource;
  /** False while a live session's preference and workspace default are still loading. */
  ready: boolean;
  /** Read the preference and workspace default again (e.g. after Settings saved a new zone). */
  refresh: () => void;
}

const noSubscription = () => () => {};
const serverZone = () => "UTC";

/** The browser's zone: "UTC" while rendering on the server, the real one once hydrated. */
export function useBrowserTimeZone(): string {
  return useSyncExternalStore(noSubscription, browserTimeZone, serverZone);
}

interface Loaded {
  /** Which request generation these answers belong to. */
  generation: number;
  user: string | null;
  workspace: string | null;
}

function zoneField(settled: PromiseSettledResult<unknown>, field: string): string | null {
  if (settled.status !== "fulfilled" || !settled.value || typeof settled.value !== "object") return null;
  const value = (settled.value as Record<string, unknown>)[field];
  return typeof value === "string" ? value : null;
}

/**
 * The zone reports should use. Pass the session's `isLive`: a demo session
 * reports in the browser's zone and makes no request.
 */
export function useReportingZone(live: boolean): ReportingZone {
  const browser = useBrowserTimeZone();
  const [generation, setGeneration] = useState(0);
  const [loaded, setLoaded] = useState<Loaded | null>(null);

  // A session that goes live again (signed out, then in) must not reuse the previous one's answers.
  const [wasLive, setWasLive] = useState(live);
  if (live !== wasLive) {
    setWasLive(live);
    if (live) setGeneration((g) => g + 1);
  }

  useEffect(() => {
    if (!live) return;
    let current = true;
    void Promise.allSettled([getApi(API_ROUTES.myPreferences), getApi(API_ROUTES.workspaceSettings)]).then(
      ([preferences, settings]) => {
        if (!current) return;
        setLoaded({
          generation,
          user: zoneField(preferences, "timeZone"),
          workspace: zoneField(settings, "defaultTimeZone"),
        });
      },
    );
    return () => {
      current = false;
    };
  }, [live, generation]);

  const refresh = useCallback(() => setGeneration((g) => g + 1), []);
  const answers = live && loaded?.generation === generation ? loaded : null;
  const { timeZone, source } = effectiveTimeZone({
    user: answers?.user,
    workspace: answers?.workspace,
    browser,
  });
  return { timeZone, source, ready: !live || answers !== null, refresh };
}
