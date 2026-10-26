// Shared fixtures for the AI assist component and page tests: the provider's
// value (useAiAssistant is mocked per file), server results in each status,
// and a promise the test settles itself. No test reaches a real provider:
// live requests go to vi-mocked hooks/ai-api functions.
import type { AiAssistantValue } from "@/components/ai/ai-provider";
import type {
  AiMemberDaySummaryDto,
  AiPeriodSummaryDto,
  AiReportExplanationDto,
  AiResultMetaDto,
  AiResultStatus,
  AiStatusDto,
} from "@/lib/ai-contract";
import { DISCLAIMER } from "@/lib/ai-assist";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { ApiError } from "@/hooks/useApi";

/** The AI assist context of a live owner in workspace w1, unless patched. */
export function aiContext(patch: Partial<AiAssistantValue> = {}): AiAssistantValue {
  return {
    status: demoAiStatus("owner"),
    statusLoading: false,
    refreshStatus: () => {},
    live: true,
    role: "owner",
    userId: "u1",
    workspaceId: "w1",
    cache: { get: () => undefined, set: () => {} },
    drawerOpen: false,
    setDrawerOpen: () => {},
    pageContext: null,
    ...patch,
  };
}

/** A status with one check failing (or the quota used up). */
export function aiStatusWith(reason: AiStatusDto["reason"], patch: Partial<AiStatusDto> = {}): AiStatusDto {
  return { ...demoAiStatus("owner"), available: reason === null, reason, ...patch };
}

export function aiMeta(featureId: string, status: AiResultStatus = "generated"): AiResultMetaDto {
  return {
    resultId: "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
    featureId,
    status,
    generatedAt: "2026-07-14T09:05:00Z",
    model: "m",
    promptVersion: `${featureId}@1`,
    dataCategories: ["durations", "app-names"],
    disclaimer: DISCLAIMER,
  };
}

export function periodResult(status: AiResultStatus = "generated"): AiPeriodSummaryDto {
  if (status === "no-data") return { meta: aiMeta("period-summary", status), headline: "Nothing was tracked for this period.", insights: [], nextSteps: [], caveats: [] };
  return {
    meta: aiMeta("period-summary", status),
    headline: "The team tracked 38 h 12 min this week.",
    insights: [{ kind: "distribution", text: "Apollo took the most time.", facts: [{ id: "F1", label: "Time on Apollo", display: "12 h 5 min" }], routeId: "projects" }],
    nextSteps: [{ label: "Open the weekly summary", routeId: "reports.weekly" }],
    caveats: ["This period includes today."],
  };
}

export function dayResult(status: AiResultStatus = "generated"): AiMemberDaySummaryDto {
  if (status === "no-data") return { meta: aiMeta("member-day-summary", status), headline: "Nothing was tracked on this day.", summary: "", highlights: [], caveats: [] };
  return {
    meta: aiMeta("member-day-summary", status),
    headline: "Wendy tracked 6 h 52 min on Tue 14 Jul.",
    summary: "Work ran from 09:02 to 17:41. Most time went to Apollo in VS Code.",
    highlights: [{ kind: "focus", text: "The longest continuous span was 2 h 10 min.", facts: [{ id: "F1", label: "Longest continuous span", display: "2 h 10 min" }], routeId: null }],
    caveats: ["Some blocks had no keyboard or mouse measurement."],
  };
}

export function reportResult(status: AiResultStatus = "generated"): AiReportExplanationDto {
  if (status === "no-data") return { meta: aiMeta("report-explanation", status), summary: "", keyFindings: [], caveats: [], questionsToExplore: [] };
  return {
    meta: aiMeta("report-explanation", status),
    summary: "This report covers 38 h 12 min of tracked time from 4 members over 5 days.",
    keyFindings: [{ kind: "distribution", text: "Apollo took 32% of the time.", facts: [{ id: "F1", label: "Share on Apollo", display: "32%" }], routeId: "reports.projects" }],
    caveats: ["App time counts the window in focus."],
    questionsToExplore: ["How does the split between projects compare with the period before?"],
  };
}

/** A promise the test resolves or rejects when it chooses. */
export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** An ABP business error as the hooks throw it. */
export function aiApiError(status: number, code: string | null, data: Record<string, unknown> | null = null): ApiError {
  return new ApiError(`HTTP ${status}`, status, { code, message: null, details: null, validationErrors: [], ...(data ? { data } : {}) });
}
