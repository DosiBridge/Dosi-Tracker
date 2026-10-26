// The AI assist wrappers: each calls its contract route on the ABP backend
// with the session's token and workspace (never an AI provider), passes the
// caller's abort signal through, and an older server without AI assist reads
// as "no AI" rather than an error.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  askAi,
  getAiSettings,
  getAiStatus,
  getAiUsage,
  getPlatformAiOverview,
  requestAiFeature,
  sendAiFeedback,
  updateAiSettings,
  updatePlatformAiStatus,
} from "./ai-api";
import { ApiError } from "./useApi";
import { demoAiStatus } from "@/lib/ai-demo-status";

const WS = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";

function res(body: unknown, status = 200): Response {
  const text = body === undefined ? "" : JSON.stringify(body);
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: status === 200 ? "OK" : "Error",
    headers: new Headers(),
    json: async () => JSON.parse(text),
    text: async () => text,
  } as Response;
}

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

const call = (n = 0) => {
  const [url, init] = fetchMock.mock.calls[n] ?? [];
  const headers = (init?.headers ?? {}) as Record<string, string>;
  return { path: new URL(String(url)).pathname, method: init?.method ?? "GET", body: init?.body, signal: init?.signal, headers };
};

beforeEach(() => {
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

describe("getAiStatus", () => {
  it("reads the caller's status in the current workspace", async () => {
    const status = demoAiStatus("owner");
    fetchMock.mockResolvedValue(res(status));
    expect(await getAiStatus()).toEqual(status);
    expect(call()).toMatchObject({ path: "/api/app/ai-assist/status", method: "GET" });
    expect(call().headers).toMatchObject({ Authorization: "Bearer tok", "X-Workspace-Id": WS });
  });

  it("reads an older server without AI assist as null", async () => {
    for (const status of [404, 405]) {
      fetchMock.mockResolvedValueOnce(res({}, status));
      expect(await getAiStatus()).toBeNull();
    }
  });

  it("still fails on any other error", async () => {
    fetchMock.mockResolvedValue(res({ error: { code: "Tracker:SubscriptionInactive" } }, 402));
    await expect(getAiStatus()).rejects.toBeInstanceOf(ApiError);
    fetchMock.mockRejectedValue(new TypeError("Failed to fetch"));
    await expect(getAiStatus()).rejects.toBeInstanceOf(TypeError);
  });
});

describe("settings and usage", () => {
  it("reads and writes the workspace settings, and reads usage", async () => {
    fetchMock.mockResolvedValue(res({ isEnabled: true }));
    await getAiSettings();
    expect(call(0)).toMatchObject({ path: "/api/app/ai-assist/settings", method: "GET" });
    const input = { isEnabled: true, allowManagers: true, allowMemberSelfService: true, allowWindowTitles: false, allowNotes: false, acknowledgedDisclosureVersion: "1.0" };
    await updateAiSettings(input);
    expect(call(1)).toMatchObject({ path: "/api/app/ai-assist/settings", method: "PUT", body: JSON.stringify(input) });
    await getAiUsage();
    expect(call(2)).toMatchObject({ path: "/api/app/ai-assist/usage", method: "GET" });
  });
});

describe("requestAiFeature", () => {
  it("POSTs the input to the feature's route once, with the caller's signal", async () => {
    fetchMock.mockResolvedValue(res({ meta: { status: "generated" }, headline: "h", insights: [], nextSteps: [], caveats: [] }));
    const controller = new AbortController();
    const input = { from: "2026-07-07T00:00:00Z", to: "2026-07-14T00:00:00Z", scope: "team" as const, regenerate: false };
    const result = await requestAiFeature("period-summary", input, { signal: controller.signal });
    expect(result.headline).toBe("h");
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(call()).toMatchObject({ path: "/api/app/ai-assist/period-summary", method: "POST", body: JSON.stringify(input), signal: controller.signal });
  });

  it("keeps a data-quality result's own items (POST bodies are never unwrapped)", async () => {
    fetchMock.mockResolvedValue(res({ meta: {}, items: [{ candidateId: "c1" }], candidateCount: 1 }));
    const result = await requestAiFeature("data-quality-review", { from: "a", to: "b", scope: "self" });
    expect(result.candidateCount).toBe(1);
    expect(result.items).toEqual([{ candidateId: "c1" }]);
  });

  it("throws the server's AI error with its data", async () => {
    fetchMock.mockResolvedValue(res({ error: { code: "Tracker:AiQuotaExceeded", message: "Used.", data: { resetsAt: "2026-08-01T00:00:00Z" } } }, 429));
    await expect(requestAiFeature("task-drafts", { projectId: "p", goal: "Launch" })).rejects.toMatchObject({
      status: 429,
      code: "Tracker:AiQuotaExceeded",
      data: { resetsAt: "2026-08-01T00:00:00Z" },
    });
  });
});

describe("Ask AI, feedback and the host console", () => {
  it("sends a turn with its signal", async () => {
    fetchMock.mockResolvedValue(res({ conversationId: "c", answer: "a" }));
    const controller = new AbortController();
    const input = { message: "How did this week go?", context: { page: "dashboard" as const } };
    expect((await askAi(input, { signal: controller.signal })).answer).toBe("a");
    expect(call()).toMatchObject({ path: "/api/app/ai-assist/ask", method: "POST", body: JSON.stringify(input), signal: controller.signal });
    await askAi({ message: "x" });
    expect(call(1).signal).toBeUndefined();
  });

  it("posts feedback (204)", async () => {
    fetchMock.mockResolvedValue(res(undefined, 204));
    await expect(sendAiFeedback({ resultId: "r1", rating: "down", reason: "tone" })).resolves.toBeUndefined();
    expect(call()).toMatchObject({ path: "/api/app/ai-assist/feedback", method: "POST", body: '{"resultId":"r1","rating":"down","reason":"tone"}' });
  });

  it("reads the platform overview and flips the runtime switch", async () => {
    fetchMock.mockResolvedValue(res({ enabled: false }));
    await getPlatformAiOverview();
    expect(call(0)).toMatchObject({ path: "/api/app/platform-ai/overview", method: "GET" });
    expect(await updatePlatformAiStatus({ enabled: false })).toEqual({ enabled: false });
    expect(call(1)).toMatchObject({ path: "/api/app/platform-ai/status", method: "PUT", body: '{"enabled":false}' });
  });
});
