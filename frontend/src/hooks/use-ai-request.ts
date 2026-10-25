"use client";

import { useEffect, useRef, useState } from "react";
import { requestAiFeature } from "./ai-api";
import {
  REGENERATABLE_FEATURES,
  type AiFeatureInputs,
  type AiFeatureResults,
  type AiInsightFeatureId,
} from "@/lib/ai-contract";
import { aiErrorState, aiResultCacheKey, type AiErrorState } from "@/lib/ai-assist";
import type { DemoAiContext } from "@/lib/ai-demo";

/* ============================================================================
 * One AI insight request, run only when the person asks (an explicit click:
 * nothing generates on page load).
 *
 *   idle → loading → ready | error
 *
 * `run(regenerate)` sends the request (a regenerate asks the server for a
 * fresh answer instead of its cached one); `cancel()` abandons it. The request
 * is aborted when the card unmounts or its input changes, and a response that
 * arrives after that — or after a newer run — is ignored, so a late answer can
 * never replace a newer one or land on the wrong period.
 *
 * A demo session never calls the API: run() resolves the demo's sample result
 * (lib/ai-demo, loaded on demand so a live deployment never downloads it). A
 * ready result is kept in the page
 * view's result cache (the AI provider's), so coming back to a page shows it
 * again without another request; it is never written to web storage.
 * ========================================================================== */

/** Where results are kept for this page view (components/ai/ai-provider holds one per workspace). */
export interface AiResultCache {
  get(key: string): unknown;
  set(key: string, value: unknown): void;
}

export type AiRequestState<T> =
  | { phase: "idle" }
  | { phase: "loading" }
  /** `reused`: shown again from this page view's cache, not just generated. */
  | { phase: "ready"; result: T; reused: boolean }
  | { phase: "error"; error: AiErrorState };

/** A feature's input without `regenerate`: run() adds it. */
export type AiRequestInput<F extends AiInsightFeatureId> = Omit<AiFeatureInputs[F], "regenerate">;

export interface UseAiRequestOptions {
  /** False keeps run() inert (the card is hidden for this caller). Default true. */
  enabled?: boolean;
  /** The session is live: ask the server. Otherwise the demo's sample output, with no network. */
  live: boolean;
  /** The workspace the request belongs to (the cache never shares results across workspaces). */
  workspaceId: string;
  cache?: AiResultCache | null;
  /** Demo only: the viewer and their zone, for self-scoped sample output. */
  demoContext?: DemoAiContext;
  /** The zone an error's dates (an allowance's renewal) are read in. */
  timeZone?: string;
}

export interface AiRequest<T> {
  state: AiRequestState<T>;
  run: (regenerate?: boolean) => void;
  cancel: () => void;
}

export function useAiRequest<F extends AiInsightFeatureId>(
  featureId: F,
  input: AiRequestInput<F>,
  options: UseAiRequestOptions,
): AiRequest<AiFeatureResults[F]> {
  const { enabled = true, live, workspaceId, cache, demoContext, timeZone } = options;
  const key = aiResultCacheKey(workspaceId, featureId, input);
  const [entry, setEntry] = useState<{ key: string; state: AiRequestState<AiFeatureResults[F]> } | null>(null);
  const controllerRef = useRef<AbortController | null>(null);
  /** Bumped by every run, cancel, input change and unmount: only the newest run may settle. */
  const runRef = useRef(0);

  const cached = cache?.get(key) as AiFeatureResults[F] | undefined;
  const resting: AiRequestState<AiFeatureResults[F]> = cached ? { phase: "ready", result: cached, reused: true } : { phase: "idle" };
  const state = entry?.key === key ? entry.state : resting;

  // A new input (or leaving the page) abandons whatever is in flight.
  useEffect(
    () => () => {
      runRef.current++;
      controllerRef.current?.abort();
      controllerRef.current = null;
    },
    [key],
  );

  // Plain functions, rebuilt every render: run() always sends the current input.
  const run = (regenerate = false) => {
    if (!enabled) return;
    controllerRef.current?.abort();
    controllerRef.current = null;
    const ticket = ++runRef.current;
    const body = (REGENERATABLE_FEATURES.includes(featureId) ? { ...input, regenerate } : { ...input }) as AiFeatureInputs[F];
    const settle = (next: AiRequestState<AiFeatureResults[F]>) => {
      if (runRef.current === ticket) setEntry({ key, state: next });
    };
    const succeed = (result: AiFeatureResults[F]) => {
      cache?.set(key, result);
      settle({ phase: "ready", result, reused: false });
    };
    setEntry({ key, state: { phase: "loading" } });

    if (!live) {
      import("@/lib/ai-demo")
        .then(({ demoAiResult }) => {
          if (runRef.current === ticket) succeed(demoAiResult(featureId, body, workspaceId, demoContext));
        })
        .catch(() => {
          // Never an API failure (nothing was sent): the generic "couldn't generate", with Retry.
          settle({ phase: "error", error: aiErrorState(null) });
        });
      return;
    }

    const controller = new AbortController();
    controllerRef.current = controller;
    requestAiFeature(featureId, body, { signal: controller.signal }).then(
      (result) => {
        if (!controller.signal.aborted) succeed(result);
      },
      (err: unknown) => {
        if (!controller.signal.aborted) settle({ phase: "error", error: aiErrorState(err, { timeZone }) });
      },
    );
  };

  const cancel = () => {
    runRef.current++;
    controllerRef.current?.abort();
    controllerRef.current = null;
    setEntry(null);
  };

  return { state, run, cancel };
}
