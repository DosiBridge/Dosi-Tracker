"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useSession } from "@/components/session-provider";
import { getApi } from "@/hooks/useApi";
import { LIVE_ENDPOINTS } from "@/lib/live-loader";
import { grantedPoliciesOf, reportAccessFor, type ReportAccess } from "@/lib/report-math";

/*
 * The light half of the report kit: one live query's state, and what the
 * viewer may see. The reports hub reads access without drawing a chart, so it
 * imports from here and stays free of the charting library; report-shell
 * re-exports both for the reports themselves.
 */

/**
 * One live (backend) query behind a report. Demo sessions (`enabled` false)
 * never touch the network. Changing the query (`load` identity) clears the
 * previous result so a figure is never shown under the wrong range/filter, and
 * only the latest request may commit (a slow earlier one can't overwrite it).
 * There is no fallback: a failed load leaves `data` null and sets `error`.
 */
export function useLiveReport<T>(enabled: boolean, load: () => Promise<T>) {
  const [state, setState] = useState<{ data: T | null; loading: boolean; error: string | null }>(() => ({
    data: null,
    loading: enabled,
    error: null,
  }));
  const seq = useRef(0);

  const run = useCallback(async () => {
    if (!enabled) return;
    const id = ++seq.current;
    setState({ data: null, loading: true, error: null });
    try {
      const data = await load();
      if (id === seq.current) setState({ data, loading: false, error: null });
    } catch (err) {
      if (id === seq.current) {
        setState({ data: null, loading: false, error: err instanceof Error ? err.message : String(err) });
      }
    }
  }, [enabled, load]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void run();
  }, [run]);

  return { ...state, retry: run };
}

/**
 * What the viewer may see (contract §6): live, from their granted permissions
 * — those the session read with the workspace, or (when it has none) the ABP
 * application configuration asked for here — falling back to their role while
 * that loads or if it can't be read; in the demo, from their role.
 */
export function useReportAccess(): ReportAccess & { ready: boolean } {
  const { isLive, user, livePolicies } = useSession();
  const load = useCallback(async () => grantedPoliciesOf(await getApi(LIVE_ENDPOINTS.configuration)), []);
  const fetched = useLiveReport(isLive && !livePolicies, load);
  const policies = isLive ? (livePolicies ?? fetched.data) : null;
  return { ...reportAccessFor(user.role, policies), ready: !isLive || !!livePolicies || !fetched.loading };
}
