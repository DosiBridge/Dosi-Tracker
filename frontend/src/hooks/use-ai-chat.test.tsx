// useAiChat: idle → sending → done | error. A question goes out once, with
// the page context and the conversation the server started; follow-ups carry
// that id. stop() and newChat() abandon what's in flight and a late answer
// never lands. An expired conversation starts a new one with the same
// question, once, and says so. Nothing touches web storage, and a demo answers
// the suggested questions with no network at all.
vi.mock("./ai-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./ai-api")>()),
  askAi: vi.fn(),
}));

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import { askAi } from "./ai-api";
import { useAiChat, type UseAiChatOptions } from "./use-ai-chat";
import type { AiAnswerDto, AiPageContextDto } from "@/lib/ai-contract";
import { AI_PROMPT_CHIPS } from "@/lib/ai-assist";
import { demoAnswer } from "@/lib/ai-demo";
import { aiApiError, aiMeta, deferred } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";

const CONTEXT: AiPageContextDto = { page: "dashboard", from: "2026-07-13T00:00:00.000Z", to: "2026-07-15T00:00:00.000Z", timeZone: "Asia/Dhaka" };

function answer(text: string, conversationId = "c-1", patch: Partial<AiAnswerDto> = {}): AiAnswerDto {
  return {
    meta: aiMeta("ask"),
    conversationId,
    answer: text,
    sources: [{ label: "Weekly summary", routeId: "reports.weekly", routeArg: null }],
    facts: [{ id: "F1", label: "Tracked this week", display: "38 h 12 min" }],
    isClarification: false,
    ...patch,
  };
}

const live = (extra: Partial<UseAiChatOptions> = {}): UseAiChatOptions => ({ live: true, workspaceId: "ws-1", timeZone: "UTC", ...extra });

beforeEach(() => {
  resetPrototypeState();
  vi.mocked(askAi).mockReset();
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("a live conversation", () => {
  it("asks once with the page context, then threads the server's conversation id through follow-ups", async () => {
    vi.mocked(askAi).mockResolvedValueOnce(answer("The team tracked 38 h.")).mockResolvedValueOnce(answer("Apollo took the most.", "c-1"));
    const { result } = renderHook(() => useAiChat(live()));
    expect(result.current.phase).toBe("idle");

    act(() => result.current.send("  How did this week go?  ", CONTEXT));
    expect(result.current.phase).toBe("sending");
    expect(result.current.turns).toMatchObject([{ role: "user", text: "How did this week go?" }]);
    await waitFor(() => expect(result.current.phase).toBe("done"));
    const [input, init] = vi.mocked(askAi).mock.calls[0];
    expect(input).toEqual({ conversationId: null, message: "How did this week go?", context: CONTEXT });
    expect(init?.signal).toBeInstanceOf(AbortSignal);
    expect(result.current.conversationId).toBe("c-1");
    expect(result.current.turns[1]).toMatchObject({
      role: "assistant",
      text: "The team tracked 38 h.",
      isClarification: false,
      sources: [{ routeId: "reports.weekly" }],
      facts: [{ id: "F1" }],
      meta: { featureId: "ask" },
    });

    act(() => result.current.send("And by project?", CONTEXT));
    await waitFor(() => expect(result.current.turns).toHaveLength(4));
    expect(vi.mocked(askAi).mock.calls[1][0]).toMatchObject({ conversationId: "c-1", message: "And by project?" });
  });

  it("ignores an empty question, one over 1000 characters, and a second one while the first is in flight", async () => {
    const pending = deferred<AiAnswerDto>();
    vi.mocked(askAi).mockReturnValue(pending.promise);
    const { result } = renderHook(() => useAiChat(live()));
    act(() => result.current.send("   "));
    act(() => result.current.send("x".repeat(1001)));
    expect(askAi).not.toHaveBeenCalled();
    expect(result.current.turns).toEqual([]);
    act(() => result.current.send("x".repeat(1000)));
    act(() => result.current.send("Another"));
    expect(askAi).toHaveBeenCalledTimes(1);
    expect(result.current.turns).toHaveLength(1);
    await act(async () => pending.resolve(answer("ok")));
  });

  it("stop() abandons the question and a late answer never lands", async () => {
    const pending = deferred<AiAnswerDto>();
    vi.mocked(askAi).mockReturnValue(pending.promise);
    const { result } = renderHook(() => useAiChat(live()));
    act(() => result.current.send("How did this week go?"));
    act(() => result.current.stop());
    expect(vi.mocked(askAi).mock.calls[0][1]?.signal?.aborted).toBe(true);
    expect(result.current.phase).toBe("idle");
    await act(async () => pending.resolve(answer("Too late")));
    expect(result.current.turns.map((t) => t.text)).toEqual(["How did this week go?"]);
    // Stopping when nothing is in flight changes nothing.
    act(() => result.current.stop());
    expect(result.current.phase).toBe("idle");
  });

  it("newChat() forgets the conversation: the next question starts a new one", async () => {
    vi.mocked(askAi).mockResolvedValueOnce(answer("First")).mockResolvedValueOnce(answer("Fresh", "c-2"));
    const { result } = renderHook(() => useAiChat(live()));
    act(() => result.current.send("One"));
    await waitFor(() => expect(result.current.phase).toBe("done"));
    act(() => result.current.newChat());
    expect(result.current).toMatchObject({ turns: [], phase: "idle", error: null, conversationId: null });
    act(() => result.current.send("Two"));
    await waitFor(() => expect(result.current.phase).toBe("done"));
    expect(vi.mocked(askAi).mock.calls[1][0].conversationId).toBeNull();
  });

  it("an expired conversation starts a new one with the same question, once, and says so", async () => {
    const onRestarted = vi.fn();
    vi.mocked(askAi)
      .mockResolvedValueOnce(answer("First", "c-1"))
      .mockRejectedValueOnce(aiApiError(409, "Tracker:AiConversationExpired"))
      .mockResolvedValueOnce(answer("Answered anew", "c-2"));
    const { result } = renderHook(() => useAiChat(live({ onRestarted })));
    act(() => result.current.send("One"));
    await waitFor(() => expect(result.current.phase).toBe("done"));
    act(() => result.current.send("Two"));
    await waitFor(() => expect(result.current.conversationId).toBe("c-2"));
    expect(onRestarted).toHaveBeenCalledTimes(1);
    expect(vi.mocked(askAi).mock.calls.map(([input]) => [input.conversationId, input.message])).toEqual([
      [null, "One"],
      ["c-1", "Two"],
      [null, "Two"],
    ]);
    // The new conversation starts with the question asked again.
    expect(result.current.turns.map((t) => t.text)).toEqual(["Two", "Answered anew"]);
  });

  it("gives up after one restart: a second expiry is an error", async () => {
    vi.mocked(askAi).mockRejectedValue(aiApiError(409, "Tracker:AiConversationExpired"));
    const onRestarted = vi.fn();
    const { result } = renderHook(() => useAiChat(live({ onRestarted })));
    act(() => result.current.send("One"));
    await waitFor(() => expect(result.current.phase).toBe("error"));
    expect(askAi).toHaveBeenCalledTimes(2);
    expect(onRestarted).toHaveBeenCalledTimes(1);
    expect(result.current.error).toMatchObject({ kind: "conversation-expired", message: "This conversation ended. Start a new one." });
  });

  it("maps a refusal through aiErrorState, and retry() asks the same question again without repeating it", async () => {
    vi.mocked(askAi).mockRejectedValueOnce(aiApiError(429, "Tracker:AiRateLimited", { reason: "minute", retryAfterSeconds: 12 })).mockResolvedValueOnce(answer("Now it works"));
    const { result } = renderHook(() => useAiChat(live()));
    act(() => result.current.send("One", CONTEXT));
    await waitFor(() => expect(result.current.phase).toBe("error"));
    expect(result.current.error).toMatchObject({ kind: "rate-limited", retryable: true, retryAfterSeconds: 12, message: "Too many AI requests. Try again in 12 seconds." });
    act(() => result.current.retry());
    await waitFor(() => expect(result.current.phase).toBe("done"));
    expect(vi.mocked(askAi).mock.calls[1][0]).toEqual({ conversationId: null, message: "One", context: CONTEXT });
    expect(result.current.turns.map((t) => t.text)).toEqual(["One", "Now it works"]);
    // Nothing to retry once answered.
    act(() => result.current.retry());
    expect(askAi).toHaveBeenCalledTimes(2);
  });

  it("names a failure that isn't the AI's own in the answer's words", async () => {
    vi.mocked(askAi).mockRejectedValue(aiApiError(500, null));
    const { result } = renderHook(() => useAiChat(live()));
    act(() => result.current.send("One"));
    await waitFor(() => expect(result.current.phase).toBe("error"));
    expect(result.current.error?.message).toBe("Couldn't get an answer. (HTTP 500)");
  });

  it("abandons the question when the component leaves", () => {
    vi.mocked(askAi).mockReturnValue(new Promise(() => {}));
    const { result, unmount } = renderHook(() => useAiChat(live()));
    act(() => result.current.send("One"));
    unmount();
    expect(vi.mocked(askAi).mock.calls[0][1]?.signal?.aborted).toBe(true);
  });

  it("never touches web storage", async () => {
    const spies = (["getItem", "setItem", "removeItem", "clear", "key"] as const).map((m) => vi.spyOn(Storage.prototype, m));
    vi.mocked(askAi).mockRejectedValueOnce(aiApiError(409, "Tracker:AiConversationExpired")).mockResolvedValue(answer("ok"));
    const { result } = renderHook(() => useAiChat(live({ onRestarted: () => {} })));
    act(() => result.current.send("One", CONTEXT));
    await waitFor(() => expect(result.current.phase).toBe("done"));
    act(() => result.current.send("Two"));
    await waitFor(() => expect(result.current.turns).toHaveLength(4));
    act(() => result.current.newChat());
    for (const spy of spies) expect(spy).not.toHaveBeenCalled();
  });
});

describe("a demo conversation", () => {
  it("answers a suggested question from the demo data with no network", async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    const chip = AI_PROMPT_CHIPS.dashboard[0];
    const { result } = renderHook(() => useAiChat({ live: false, workspaceId: "w1", demoContext: { userId: "u1" } }));
    act(() => result.current.send(chip.label, { page: "dashboard" }, chip.id));
    await waitFor(() => expect(result.current.phase).toBe("done"));
    expect(result.current.turns[1]).toMatchObject({ role: "assistant", text: demoAnswer(chip.id, "dashboard", "w1").answer, meta: { status: "demo" } });
    expect(result.current.conversationId).toBe("demo-conversation");
    expect(askAi).not.toHaveBeenCalled();
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("answers anything else with a clarification listing the page's questions", async () => {
    const { result } = renderHook(() => useAiChat({ live: false, workspaceId: "w1" }));
    act(() => result.current.send("What is the meaning of life?"));
    await waitFor(() => expect(result.current.phase).toBe("done"));
    expect(result.current.turns[1].isClarification).toBe(true);
    expect(result.current.turns[1].text).toContain(AI_PROMPT_CHIPS.other[0].label);
    expect(askAi).not.toHaveBeenCalled();
  });
});
