// The AI assist contract mirror: the routes the web calls, the feature ids and
// the error-status map must match the backend contract (AI assist spec §5)
// exactly, and the core contract's own error map must stay untouched.
import { describe, expect, it } from "vitest";
import {
  AI_ERROR_STATUS,
  AI_FEATURE_IDS,
  AI_ROUTES,
  REGENERATABLE_FEATURES,
  aiFeatureRoute,
  isAiErrorCode,
  isAiFeatureId,
} from "./ai-contract";
import { TRACKER_ERROR_STATUS } from "./api-contract";
import contract from "@/test/api-contract.json";

describe("AI routes", () => {
  it("pins every fixed route of §5.1", () => {
    expect(AI_ROUTES).toEqual({
      status: "/api/app/ai-assist/status",
      settings: "/api/app/ai-assist/settings",
      usage: "/api/app/ai-assist/usage",
      ask: "/api/app/ai-assist/ask",
      feedback: "/api/app/ai-assist/feedback",
      platformOverview: "/api/app/platform-ai/overview",
      platformStatus: "/api/app/platform-ai/status",
    });
  });

  it("builds each feature's POST route from its id", () => {
    expect(aiFeatureRoute("period-summary")).toBe("/api/app/ai-assist/period-summary");
    expect(aiFeatureRoute("member-day-summary")).toBe("/api/app/ai-assist/member-day-summary");
    expect(aiFeatureRoute("report-explanation")).toBe("/api/app/ai-assist/report-explanation");
    expect(aiFeatureRoute("data-quality-review")).toBe("/api/app/ai-assist/data-quality-review");
    expect(aiFeatureRoute("note-suggestions")).toBe("/api/app/ai-assist/note-suggestions");
    expect(aiFeatureRoute("task-drafts")).toBe("/api/app/ai-assist/task-drafts");
    expect(aiFeatureRoute("ask")).toBe(AI_ROUTES.ask);
  });

  it("declares both route roots in the backend contract file", () => {
    expect(contract.endpoints).toContain("/api/app/ai-assist");
    expect(contract.endpoints).toContain("/api/app/platform-ai");
  });
});

describe("feature ids", () => {
  it("lists the seven features of v1", () => {
    expect([...AI_FEATURE_IDS]).toEqual([
      "period-summary",
      "member-day-summary",
      "report-explanation",
      "data-quality-review",
      "note-suggestions",
      "task-drafts",
      "ask",
    ]);
  });

  it("recognises only known ids", () => {
    for (const id of AI_FEATURE_IDS) expect(isAiFeatureId(id)).toBe(true);
    for (const other of ["", "summary", "ASK", null, undefined, 3]) expect(isAiFeatureId(other)).toBe(false);
  });

  it("lets only the four range features regenerate (their inputs carry regenerate)", () => {
    expect([...REGENERATABLE_FEATURES].sort()).toEqual(["data-quality-review", "member-day-summary", "period-summary", "report-explanation"]);
  });
});

describe("AI_ERROR_STATUS", () => {
  it("maps the 11 AI codes of §5.8 to their statuses", () => {
    expect(AI_ERROR_STATUS).toEqual({
      "Tracker:AiNotConfigured": 503,
      "Tracker:AiPlatformDisabled": 503,
      "Tracker:AiDisabled": 409,
      "Tracker:AiFeatureDisabled": 403,
      "Tracker:AiQuotaExceeded": 429,
      "Tracker:AiRateLimited": 429,
      "Tracker:AiProviderUnavailable": 503,
      "Tracker:AiOutputInvalid": 502,
      "Tracker:AiRangeTooLarge": 400,
      "Tracker:AiDisclosureNotAccepted": 400,
      "Tracker:AiConversationExpired": 409,
    });
    expect(Object.keys(AI_ERROR_STATUS)).toHaveLength(11);
  });

  it("keeps the AI codes out of the core contract's map", () => {
    expect(Object.keys(TRACKER_ERROR_STATUS)).toHaveLength(29);
    for (const code of Object.keys(AI_ERROR_STATUS)) expect(code in TRACKER_ERROR_STATUS).toBe(false);
  });

  it("recognises only AI codes", () => {
    expect(isAiErrorCode("Tracker:AiRateLimited")).toBe(true);
    expect(isAiErrorCode("Tracker:SeatLimitReached")).toBe(false);
    expect(isAiErrorCode("toString")).toBe(false);
    expect(isAiErrorCode(null)).toBe(false);
  });
});
