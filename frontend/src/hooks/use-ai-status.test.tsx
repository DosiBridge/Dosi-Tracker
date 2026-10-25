// useAiStatus: a demo session reads the demo's status with no network; a live
// session asks once per workspace (shared by every consumer) and again on
// refresh(); a status that can't be read hides AI (null) with a warning.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import { resetAiStatusCacheForTests, useAiStatus } from "./use-ai-status";
import { demoAiStatus } from "@/lib/ai-demo-status";
import type { Role } from "@/lib/types";

const WS1 = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
const WS2 = "9b2504e0-4f89-11d3-9a0c-0305e82c3302";

function res(body: unknown, status = 200): Response {
  const text = JSON.stringify(body);
  return { ok: status < 300, status, statusText: "", headers: new Headers(), json: async () => JSON.parse(text), text: async () => text } as Response;
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

beforeEach(() => {
  resetAiStatusCacheForTests();
  localStorage.clear();
  localStorage.setItem("dosi-token", "tok");
  localStorage.setItem("dosi-workspace", WS1);
  fetchMock = vi.fn<typeof fetch>();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  localStorage.clear();
});

describe("useAiStatus in demo mode", () => {
  it("reads the demo's status for the role and never fetches", () => {
    const { result, rerender } = renderHook(({ role }) => useAiStatus({ live: false, role }), { initialProps: { role: "worker" as Role } });
    expect(result.current).toMatchObject({ status: demoAiStatus("worker"), loading: false });
    rerender({ role: "owner" });
    expect(result.current.status).toEqual(demoAiStatus("owner"));
    act(() => result.current.refresh());
    expect(result.current.loading).toBe(false);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

describe("useAiStatus in a live session", () => {
  it("asks the server once per workspace, shared by every consumer", async () => {
    const status = demoAiStatus("admin");
    fetchMock.mockResolvedValue(res(status));
    const first = renderHook(() => useAiStatus({ live: true, role: "admin" }));
    const second = renderHook(() => useAiStatus({ live: true, role: "admin" }));
    expect(first.result.current).toMatchObject({ status: null, loading: true });
    await waitFor(() => expect(first.result.current.status).toEqual(status));
    await waitFor(() => expect(second.result.current.status).toEqual(status));
    expect(first.result.current.loading).toBe(false);
    first.unmount();
    const third = renderHook(() => useAiStatus({ live: true, role: "admin" }));
    await waitFor(() => expect(third.result.current.status).toEqual(status));
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(new URL(String(fetchMock.mock.calls[0][0])).pathname).toBe("/api/app/ai-assist/status");

    // Another workspace is another question.
    localStorage.setItem("dosi-workspace", WS2);
    renderHook(() => useAiStatus({ live: true, role: "admin" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    expect((fetchMock.mock.calls[1][1]?.headers as Record<string, string>)["X-Workspace-Id"]).toBe(WS2);
  });

  it("never shows one workspace's status for another while the new one loads", async () => {
    const first = demoAiStatus("owner");
    fetchMock.mockResolvedValueOnce(res(first));
    const { result, rerender } = renderHook(({ workspaceId }) => useAiStatus({ live: true, role: "owner", workspaceId }), {
      initialProps: { workspaceId: "live-a" },
    });
    await waitFor(() => expect(result.current.status).toEqual(first));

    let release: (r: Response) => void = () => {};
    fetchMock.mockReturnValueOnce(new Promise<Response>((resolve) => (release = resolve)));
    localStorage.setItem("dosi-workspace", WS2);
    rerender({ workspaceId: "live-b" });
    expect(result.current).toMatchObject({ status: null, loading: true });
    const second = { ...first, canManage: false };
    await act(async () => release(res(second)));
    await waitFor(() => expect(result.current).toMatchObject({ status: second, loading: false }));
  });

  it("asks again on refresh, keeping the previous answer up meanwhile", async () => {
    const before = demoAiStatus("owner");
    const after = { ...before, workspaceEnabled: false, available: false, reason: "workspace-disabled" as const };
    fetchMock.mockResolvedValueOnce(res(before));
    const { result } = renderHook(() => useAiStatus({ live: true, role: "owner" }));
    await waitFor(() => expect(result.current.status).toEqual(before));

    let release: (r: Response) => void = () => {};
    fetchMock.mockReturnValueOnce(new Promise<Response>((resolve) => (release = resolve)));
    act(() => result.current.refresh());
    expect(result.current).toMatchObject({ status: before, loading: true });
    await act(async () => release(res(after)));
    await waitFor(() => expect(result.current).toMatchObject({ status: after, loading: false }));
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("hides AI on an older server (404) without a warning", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    fetchMock.mockResolvedValue(res({}, 404));
    const { result } = renderHook(() => useAiStatus({ live: true, role: "owner" }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.status).toBeNull();
    expect(warn).not.toHaveBeenCalled();
  });

  it("hides AI when the status can't be read, warns without personal data, and retries on the next mount", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    fetchMock.mockResolvedValueOnce(res({ error: { message: "Internal" } }, 500));
    const first = renderHook(() => useAiStatus({ live: true, role: "owner" }));
    await waitFor(() => expect(first.result.current.loading).toBe(false));
    expect(first.result.current.status).toBeNull();
    expect(warn).toHaveBeenCalledWith(expect.stringMatching(/AI assist status/), "HTTP 500");

    fetchMock.mockRejectedValueOnce(new TypeError("Failed to fetch"));
    const second = renderHook(() => useAiStatus({ live: true, role: "owner" }));
    await waitFor(() => expect(warn).toHaveBeenCalledTimes(2));
    expect(warn).toHaveBeenLastCalledWith(expect.any(String), "unreachable");
    expect(second.result.current.status).toBeNull();

    const status = demoAiStatus("owner");
    fetchMock.mockResolvedValueOnce(res(status));
    const third = renderHook(() => useAiStatus({ live: true, role: "owner" }));
    await waitFor(() => expect(third.result.current.status).toEqual(status));
  });

  it("ignores an answer that lands after unmount", async () => {
    let release: (r: Response) => void = () => {};
    fetchMock.mockReturnValueOnce(new Promise<Response>((resolve) => (release = resolve)));
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    const { unmount } = renderHook(() => useAiStatus({ live: true, role: "owner" }));
    unmount();
    await act(async () => release(res(demoAiStatus("owner"))));
    expect(error).not.toHaveBeenCalled();
  });
});
