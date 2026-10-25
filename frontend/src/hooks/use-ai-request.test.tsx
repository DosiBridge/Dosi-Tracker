// useAiRequest: nothing runs until run(); a demo resolves the sample result on
// a microtask and never fetches; a live run calls the server once with the
// body and an abort signal; unmounting, a new input and cancel() abort it; a
// late response never lands; results are reused from the page view's cache.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import { useAiRequest, type AiResultCache, type UseAiRequestOptions } from "./use-ai-request";
import type { AiPeriodSummaryDto } from "@/lib/ai-contract";
import { demoAiResult } from "@/lib/ai-demo";
import { DAY_MS, NOW } from "@/lib/demo-clock";
import { resetTenantDataForTests } from "@/lib/demo-tenants";
import { datasetFor } from "@/lib/tenant-data";

const WS = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
const WEEK = { from: new Date(NOW.getTime() - 7 * DAY_MS).toISOString(), to: NOW.toISOString(), scope: "team" as const };
const OTHER_WEEK = { ...WEEK, from: new Date(NOW.getTime() - 14 * DAY_MS).toISOString() };

function summary(headline: string): AiPeriodSummaryDto {
  return {
    meta: { resultId: headline, featureId: "period-summary", status: "generated", generatedAt: "2026-07-14T10:00:00Z", model: "m", promptVersion: "period-summary@1", dataCategories: [], disclaimer: "d" },
    headline,
    insights: [],
    nextSteps: [],
    caveats: [],
  };
}

function res(body: unknown, status = 200): Response {
  const text = JSON.stringify(body);
  return { ok: status < 300, status, statusText: "", headers: new Headers(), json: async () => JSON.parse(text), text: async () => text } as Response;
}

function mapCache(): AiResultCache & { map: Map<string, unknown> } {
  const map = new Map<string, unknown>();
  return { map, get: (k) => map.get(k), set: (k, v) => void map.set(k, v) };
}

/** A fetch that never answers on its own: the test resolves each call, and aborting rejects it. */
function deferredFetch() {
  const calls: { init: RequestInit | undefined; resolve: (r: Response) => void }[] = [];
  fetchMock.mockImplementation(
    (_url, init) =>
      new Promise<Response>((resolve, reject) => {
        calls.push({ init, resolve });
        init?.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
      }),
  );
  return calls;
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

beforeEach(() => {
  resetTenantDataForTests();
  localStorage.clear();
  localStorage.setItem("dosi-token", "tok");
  localStorage.setItem("dosi-workspace", WS);
  fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

const live = (extra: Partial<UseAiRequestOptions> = {}): UseAiRequestOptions => ({ live: true, workspaceId: "live-ws", ...extra });

describe("in demo mode", () => {
  it("stays idle until run(), then resolves the demo's sample on a microtask without any request", async () => {
    const cache = mapCache();
    const { result } = renderHook(() => useAiRequest("period-summary", WEEK, { live: false, workspaceId: "w1", cache }));
    expect(result.current.state).toEqual({ phase: "idle" });
    act(() => result.current.run());
    expect(result.current.state).toEqual({ phase: "loading" });
    await waitFor(() => expect(result.current.state.phase).toBe("ready"));
    const expected = demoAiResult("period-summary", { ...WEEK, regenerate: false }, "w1");
    expect(result.current.state).toEqual({ phase: "ready", result: expected, reused: false });
    expect(expected.meta.status).toBe("demo");
    expect(cache.map.size).toBe(1);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("passes the viewer to self-scoped sample output", async () => {
    const { result } = renderHook(() =>
      useAiRequest("period-summary", { ...WEEK, scope: "self" }, { live: false, workspaceId: "w1", demoContext: { userId: "u1" } }),
    );
    act(() => result.current.run());
    await waitFor(() => expect(result.current.state.phase).toBe("ready"));
    const state = result.current.state as { result: AiPeriodSummaryDto };
    expect(state.result.headline.startsWith("You tracked")).toBe(true);
  });

  it("shows a generic, retryable error if the sample can't be built", async () => {
    datasetFor("ai-broken").activities = null as never;
    const { result } = renderHook(() => useAiRequest("period-summary", WEEK, { live: false, workspaceId: "ai-broken" }));
    act(() => result.current.run());
    await waitFor(() => expect(result.current.state.phase).toBe("error"));
    expect(result.current.state).toMatchObject({ phase: "error", error: { kind: "unknown", retryable: true } });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("drops a sample that resolves after unmount", async () => {
    const cache = mapCache();
    const { result, unmount } = renderHook(() => useAiRequest("period-summary", WEEK, { live: false, workspaceId: "w1", cache }));
    act(() => result.current.run());
    unmount();
    await Promise.resolve();
    expect(cache.map.size).toBe(0);
  });
});

describe("in a live session", () => {
  it("calls the server once with the body and a signal, and caches the result", async () => {
    fetchMock.mockResolvedValue(res(summary("live")));
    const cache = mapCache();
    const { result } = renderHook(() => useAiRequest("period-summary", WEEK, live({ cache })));
    act(() => result.current.run());
    await waitFor(() => expect(result.current.state.phase).toBe("ready"));
    expect(result.current.state).toEqual({ phase: "ready", result: summary("live"), reused: false });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0];
    expect(new URL(String(url)).pathname).toBe("/api/app/ai-assist/period-summary");
    expect(init).toMatchObject({ method: "POST", body: JSON.stringify({ ...WEEK, regenerate: false }) });
    expect(init?.signal).toBeInstanceOf(AbortSignal);

    // Another card with the same input shows the kept result without asking again.
    const again = renderHook(() => useAiRequest("period-summary", { scope: "team", to: WEEK.to, from: WEEK.from }, live({ cache })));
    expect(again.result.current.state).toEqual({ phase: "ready", result: summary("live"), reused: true });
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("asks for a fresh answer on regenerate, and sends no regenerate flag where the input has none", async () => {
    fetchMock.mockResolvedValue(res(summary("x")));
    const period = renderHook(() => useAiRequest("period-summary", WEEK, live()));
    act(() => period.result.current.run(true));
    await waitFor(() => expect(period.result.current.state.phase).toBe("ready"));
    expect(JSON.parse(String(fetchMock.mock.calls[0][1]?.body))).toMatchObject({ regenerate: true });

    fetchMock.mockResolvedValue(res({ meta: summary("n").meta, suggestions: [] }));
    const notes = renderHook(() => useAiRequest("note-suggestions", { activityId: "a1" }, live()));
    act(() => notes.result.current.run(true));
    await waitFor(() => expect(notes.result.current.state.phase).toBe("ready"));
    expect(fetchMock.mock.calls[1][1]?.body).toBe('{"activityId":"a1"}');
  });

  it("does nothing while disabled", () => {
    const { result } = renderHook(() => useAiRequest("period-summary", WEEK, live({ enabled: false })));
    act(() => result.current.run());
    expect(result.current.state).toEqual({ phase: "idle" });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("aborts the request on unmount", async () => {
    const calls = deferredFetch();
    const { result, unmount } = renderHook(() => useAiRequest("period-summary", WEEK, live()));
    act(() => result.current.run());
    await waitFor(() => expect(calls).toHaveLength(1));
    unmount();
    expect(calls[0].init?.signal?.aborted).toBe(true);
  });

  it("aborts the request when the input changes, and the new input starts idle", async () => {
    const calls = deferredFetch();
    const { result, rerender } = renderHook(({ input }) => useAiRequest("period-summary", input, live()), { initialProps: { input: WEEK } });
    act(() => result.current.run());
    await waitFor(() => expect(calls).toHaveLength(1));
    rerender({ input: OTHER_WEEK });
    expect(calls[0].init?.signal?.aborted).toBe(true);
    expect(result.current.state).toEqual({ phase: "idle" });
    await act(async () => calls[0].resolve(res(summary("late"))));
    expect(result.current.state).toEqual({ phase: "idle" });
  });

  it("ignores a late response once a newer run started", async () => {
    // A server that answers even though the request was abandoned.
    const resolvers: ((r: Response) => void)[] = [];
    fetchMock.mockImplementation(() => new Promise<Response>((resolve) => resolvers.push(resolve)));
    const { result } = renderHook(() => useAiRequest("period-summary", WEEK, live()));
    act(() => result.current.run());
    await waitFor(() => expect(resolvers).toHaveLength(1));
    act(() => result.current.run(true));
    await waitFor(() => expect(resolvers).toHaveLength(2));
    await act(async () => resolvers[1](res(summary("newer"))));
    await waitFor(() => expect(result.current.state.phase).toBe("ready"));
    await act(async () => resolvers[0](res(summary("older"))));
    expect(result.current.state).toMatchObject({ phase: "ready", result: { headline: "newer" } });
  });

  it("cancel() abandons the request and goes back to idle", async () => {
    const calls = deferredFetch();
    const { result } = renderHook(() => useAiRequest("period-summary", WEEK, live()));
    act(() => result.current.run());
    await waitFor(() => expect(calls).toHaveLength(1));
    act(() => result.current.cancel());
    expect(calls[0].init?.signal?.aborted).toBe(true);
    expect(result.current.state).toEqual({ phase: "idle" });
  });

  it("cancelling a regenerate goes back to the kept result", async () => {
    const cache = mapCache();
    fetchMock.mockResolvedValueOnce(res(summary("first")));
    const { result } = renderHook(() => useAiRequest("period-summary", WEEK, live({ cache })));
    act(() => result.current.run());
    await waitFor(() => expect(result.current.state.phase).toBe("ready"));
    deferredFetch();
    act(() => result.current.run(true));
    expect(result.current.state.phase).toBe("loading");
    act(() => result.current.cancel());
    expect(result.current.state).toEqual({ phase: "ready", result: summary("first"), reused: true });
  });

  it("turns a refusal into the card's error", async () => {
    fetchMock.mockResolvedValue(res({ error: { code: "Tracker:AiRateLimited", data: { reason: "minute", retryAfterSeconds: 20 } } }, 429));
    const { result } = renderHook(() => useAiRequest("period-summary", WEEK, live()));
    act(() => result.current.run());
    await waitFor(() => expect(result.current.state.phase).toBe("error"));
    expect(result.current.state).toEqual({
      phase: "error",
      error: { kind: "rate-limited", message: "Too many AI requests. Try again in 20 seconds.", retryAfterSeconds: 20, resetsAt: null, retryable: true },
    });
  });
});
