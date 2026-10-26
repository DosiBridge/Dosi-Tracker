// The web side of AI assist: when a card shows (the §18.3 matrix: status ×
// role × feature × scope), what each failed request says, where results may
// link (§5.9 allowlist), and the shared copy, which must never name a
// credential (architecture RULE 8).
import { describe, expect, it } from "vitest";
import {
  AI_COPY,
  AI_PROMPT_CHIPS,
  AI_UNREACHABLE_TEXT,
  DEMO_NOTE,
  DISCLAIMER,
  WHATS_SENT,
  aiAvailability,
  aiErrorState,
  aiResultCacheKey,
  aiSetupNotice,
  dataCategoryLabel,
  formatAiDate,
  isAiRouteId,
  quotaNote,
  resultBadge,
  routeIdToHref,
  stableKey,
} from "./ai-assist";
import { AI_FEATURE_IDS, type AiFeatureId, type AiScope, type AiStatusDto, type AiStatusReason } from "./ai-contract";
import { demoAiStatus } from "./ai-demo-status";
import { reportCatalog } from "./reports-data";
import type { Role } from "./types";

const ROLES: Role[] = ["owner", "admin", "worker", "client", "host"];

function status(patch: Partial<AiStatusDto> = {}): AiStatusDto {
  return { ...demoAiStatus("owner"), ...patch };
}

/** An ApiError's shape (hooks/useApi), without importing the hook layer into a lib test. */
function apiError(status: number, code: string | null, data: Record<string, unknown> | null = null, extra: Record<string, unknown> = {}) {
  return { name: "ApiError", status, code, data, serverMessage: null, retryAfterSeconds: null, validationErrors: [], ...extra };
}

describe("aiAvailability", () => {
  it("hides everything without a status (older server, failed read)", () => {
    for (const id of AI_FEATURE_IDS) {
      expect(aiAvailability(null, id)).toEqual({ visible: false, enabled: false, reason: "unavailable" });
      expect(aiAvailability(undefined, id)).toEqual({ visible: false, enabled: false, reason: "unavailable" });
    }
  });

  it.each<[AiStatusReason, string]>([
    ["not-configured", "unavailable"],
    ["platform-disabled", "unavailable"],
    ["subscription-inactive", "unavailable"],
    ["workspace-disabled", "workspace-off"],
    ["reconsent-required", "reconsent"],
    ["role-not-allowed", "not-allowed"],
    ["self-service-off", "not-allowed"],
  ])("hides every card when the status says %s", (reason, expected) => {
    for (const id of AI_FEATURE_IDS) {
      expect(aiAvailability(status({ available: false, reason }), id)).toEqual({ visible: false, enabled: false, reason: expected });
    }
  });

  it("keeps Generate on with the quota note when the allowance is used", () => {
    expect(aiAvailability(status({ available: false, reason: "quota-exhausted" }), "period-summary", "team")).toEqual({
      visible: true,
      enabled: true,
      reason: "quota",
    });
  });

  it("hides when unavailable for a reason this web doesn't know", () => {
    expect(aiAvailability(status({ available: false, reason: null }), "period-summary")).toEqual({ visible: false, enabled: false, reason: "unavailable" });
  });

  it("hides a feature the server refused or didn't list", () => {
    const refused = status({ features: status().features.map((f) => (f.featureId === "task-drafts" ? { ...f, allowed: false, reason: "permission" } : f)) });
    expect(aiAvailability(refused, "task-drafts").reason).toBe("not-allowed");
    expect(aiAvailability(status({ features: [] }), "ask").reason).toBe("not-allowed");
  });

  // The full matrix, against what the server says to each role (§4.2, mirrored by the demo status).
  const MATRIX: Record<Role, Partial<Record<AiFeatureId, (AiScope | undefined)[]>>> = {
    owner: {
      "period-summary": [undefined, "team", "self"],
      "member-day-summary": [undefined, "member", "self"],
      "report-explanation": [undefined],
      "data-quality-review": [undefined, "team", "self"],
      "note-suggestions": [undefined],
      "task-drafts": [undefined],
      ask: [undefined],
    },
    admin: {
      "period-summary": [undefined, "team", "self"],
      "member-day-summary": [undefined, "member", "self"],
      "report-explanation": [undefined],
      "data-quality-review": [undefined, "team", "self"],
      "note-suggestions": [undefined],
      "task-drafts": [undefined],
      ask: [undefined],
    },
    worker: {
      "period-summary": [undefined, "self"],
      "member-day-summary": [undefined, "self"],
      "data-quality-review": [undefined, "self"],
      "note-suggestions": [undefined],
      "task-drafts": [undefined],
      ask: [undefined],
    },
    client: {},
    host: {},
  };
  const SCOPES: (AiScope | undefined)[] = [undefined, "team", "self", "member"];

  it.each(ROLES)("follows the role matrix for %s", (role) => {
    const s = demoAiStatus(role);
    for (const id of AI_FEATURE_IDS) {
      for (const scope of SCOPES) {
        const expected = MATRIX[role][id]?.includes(scope) ?? false;
        const got = aiAvailability(s, id, scope);
        expect(got.visible, `${role} ${id} ${scope ?? "(any)"}`).toBe(expected);
        expect(got.enabled).toBe(expected);
      }
    }
  });
});

describe("aiSetupNotice", () => {
  it("tells only the owner the server has no AI assist", () => {
    for (const s of [null, status({ available: false, reason: "not-configured" }), status({ available: false, reason: "platform-disabled" })]) {
      expect(aiSetupNotice(s, "owner")).toBe("server-unavailable");
      expect(aiSetupNotice(s, "admin")).toBeNull();
      expect(aiSetupNotice(s, "worker")).toBeNull();
    }
  });

  it("asks the owner to turn it on, and tells a manager the owner can", () => {
    for (const reason of ["workspace-disabled", "reconsent-required"] as const) {
      const s = status({ available: false, reason, workspaceEnabled: false });
      expect(aiSetupNotice(s, "owner")).toBe("turn-on");
      expect(aiSetupNotice(s, "admin")).toBe("ask-owner");
      expect(aiSetupNotice(s, "worker")).toBeNull();
      expect(aiSetupNotice(s, "client")).toBeNull();
      // An owner the server doesn't count as an active owner can't change it either.
      expect(aiSetupNotice({ ...s, canManage: false }, "owner")).toBeNull();
    }
  });

  it("says nothing once AI assist is on", () => {
    for (const role of ROLES) expect(aiSetupNotice(status(), role)).toBeNull();
    expect(aiSetupNotice(status({ available: false, reason: "quota-exhausted" }), "owner")).toBeNull();
  });
});

describe("aiErrorState", () => {
  it("names the server's absence of AI and the platform switch, without Retry", () => {
    expect(aiErrorState(apiError(503, "Tracker:AiNotConfigured"))).toMatchObject({ kind: "not-configured", message: "AI assist isn't available on this server.", retryable: false });
    expect(aiErrorState(apiError(503, "Tracker:AiPlatformDisabled"))).toMatchObject({ kind: "platform-disabled", retryable: false });
  });

  it("tells an off workspace from one that needs the disclosure accepted again", () => {
    expect(aiErrorState(apiError(409, "Tracker:AiDisabled", { reason: "off" })).message).toBe("AI assist is off for this workspace.");
    expect(aiErrorState(apiError(409, "Tracker:AiDisabled", { reason: "reconsent" })).message).toMatch(/updated AI disclosure/);
    expect(aiErrorState(apiError(409, "Tracker:AiDisabled")).kind).toBe("disabled");
  });

  it("reads the renewal date of an exhausted allowance", () => {
    const state = aiErrorState(apiError(429, "Tracker:AiQuotaExceeded", { scope: "workspace-month", resetsAt: "2026-08-01T00:00:00Z" }));
    expect(state).toMatchObject({ kind: "quota", resetsAt: "2026-08-01T00:00:00Z", retryable: false });
    expect(state.message).toBe("The AI allowance for this period has been used. It renews on Sat 1 Aug.");
    expect(aiErrorState(apiError(429, "Tracker:AiQuotaExceeded")).message).toBe("The AI allowance for this period has been used.");
    // In the viewer's zone: midnight UTC is still 31 July in New York.
    expect(aiErrorState(apiError(429, "Tracker:AiQuotaExceeded", { resetsAt: "2026-08-01T00:00:00Z" }), { timeZone: "America/New_York" }).message).toMatch(/Fri 31 Jul/);
  });

  it("says how long to wait when rate-limited, from error.data or Retry-After", () => {
    expect(aiErrorState(apiError(429, "Tracker:AiRateLimited", { reason: "minute", retryAfterSeconds: 20 }))).toMatchObject({
      kind: "rate-limited",
      message: "Too many AI requests. Try again in 20 seconds.",
      retryAfterSeconds: 20,
      retryable: true,
    });
    expect(aiErrorState(apiError(429, "Tracker:AiRateLimited", { reason: "busy", retryAfterSeconds: 1 })).message).toBe(
      "Another AI request is still running. Try again in 1 second.",
    );
    expect(aiErrorState(apiError(429, "Tracker:AiRateLimited", null, { retryAfterSeconds: 4.2 }))).toMatchObject({ retryAfterSeconds: 5 });
    expect(aiErrorState(apiError(429, "Tracker:AiRateLimited", { retryAfterSeconds: -3 })).message).toBe("Too many AI requests. Try again in a moment.");
  });

  it("offers Retry for a slow provider and an unreliable answer", () => {
    expect(aiErrorState(apiError(503, "Tracker:AiProviderUnavailable", { kind: "timeout", retryAfterSeconds: 30 }))).toMatchObject({
      kind: "provider-unavailable",
      retryable: true,
      retryAfterSeconds: 30,
    });
    expect(aiErrorState(apiError(502, "Tracker:AiOutputInvalid"))).toMatchObject({ kind: "output-invalid", retryable: true });
  });

  it("names the range cap and the disclosure version", () => {
    expect(aiErrorState(apiError(400, "Tracker:AiRangeTooLarge", { maxDays: 31 })).message).toBe("Choose 31 days or fewer for AI summaries.");
    expect(aiErrorState(apiError(400, "Tracker:AiRangeTooLarge")).message).toBe("Choose fewer days for AI summaries.");
    expect(aiErrorState(apiError(400, "Tracker:AiDisclosureNotAccepted", { version: "1.2" })).message).toBe(
      "Review and accept the AI disclosure (version 1.2) to turn on AI assist.",
    );
    expect(aiErrorState(apiError(400, "Tracker:AiDisclosureNotAccepted")).message).toBe("Review and accept the AI disclosure to turn on AI assist.");
  });

  it("maps the remaining AI and subscription codes", () => {
    expect(aiErrorState(apiError(403, "Tracker:AiFeatureDisabled", { reason: "role" }))).toMatchObject({ kind: "feature-disabled", retryable: false });
    expect(aiErrorState(apiError(409, "Tracker:AiConversationExpired"))).toMatchObject({ kind: "conversation-expired", message: "This conversation ended. Start a new one." });
    expect(aiErrorState(apiError(402, "Tracker:SubscriptionInactive"))).toMatchObject({ kind: "subscription-inactive", retryable: false });
  });

  it("falls back like requestErrorText for everything else", () => {
    expect(aiErrorState(apiError(400, null, null, { validationErrors: [{ message: "Date is invalid." }, { message: " " }, null] })).message).toBe("Date is invalid.");
    expect(aiErrorState(apiError(404, "Tracker:InvalidUser", null, { serverMessage: "That member isn't in this workspace." }))).toMatchObject({
      kind: "unknown",
      message: "That member isn't in this workspace.",
      retryable: false,
    });
    expect(aiErrorState(apiError(403, null, null, { serverMessage: "Not yours." })).kind).toBe("forbidden");
    expect(aiErrorState(apiError(403, null))).toMatchObject({ kind: "forbidden", message: "You don't have permission to do that.", retryable: false });
    expect(aiErrorState(apiError(500, null))).toMatchObject({ message: "Couldn't generate the summary. (HTTP 500)", retryable: true });
    expect(aiErrorState(apiError(429, null), { fallback: "Couldn't suggest a note." })).toMatchObject({ message: "Couldn't suggest a note. (HTTP 429)", retryable: true });
    expect(aiErrorState(apiError(408, null)).retryable).toBe(true);
    expect(aiErrorState(apiError(422, null)).retryable).toBe(false);
  });

  it("tells an unreachable server apart, in the same words as the rest of the app", () => {
    // components/layout/request-error's UNREACHABLE_TEXT (pinned there by its own test).
    expect(AI_UNREACHABLE_TEXT).toBe("Couldn't reach the server. Check your connection and try again.");
    expect(aiErrorState({ name: "ApiUnreachableError" })).toMatchObject({ kind: "unreachable", message: AI_UNREACHABLE_TEXT, retryable: true });
    expect(aiErrorState(new TypeError("Failed to fetch")).kind).toBe("unreachable");
  });

  it("survives anything thrown", () => {
    for (const thrown of [new Error("boom"), "weird", null, undefined, 42]) {
      expect(aiErrorState(thrown)).toEqual({ kind: "unknown", message: "Couldn't generate the summary.", retryable: true, retryAfterSeconds: null, resetsAt: null });
    }
  });
});

describe("routeIdToHref", () => {
  it("maps every fixed route id of §5.9", () => {
    expect(routeIdToHref("dashboard")).toBe("/dashboard");
    expect(routeIdToHref("monitor")).toBe("/monitor");
    expect(routeIdToHref("timesheet")).toBe("/timesheet");
    expect(routeIdToHref("team")).toBe("/team");
    expect(routeIdToHref("projects")).toBe("/projects");
    expect(routeIdToHref("activities")).toBe("/activities");
    expect(routeIdToHref("download")).toBe("/download");
    expect(routeIdToHref("install.windows")).toBe("/install/windows");
    expect(routeIdToHref("install.macos")).toBe("/install/macos");
    expect(routeIdToHref("install.linux")).toBe("/install/linux");
    expect(routeIdToHref("settings")).toBe("/settings");
    expect(routeIdToHref("settings.workspace")).toBe("/settings?tab=workspace");
    expect(routeIdToHref("settings.privacy")).toBe("/settings?tab=privacy");
    expect(routeIdToHref("reports")).toBe("/reports");
    expect(routeIdToHref("billing")).toBe("/billing");
  });

  it("links a project only with a plain id", () => {
    expect(routeIdToHref("projects.detail", "3f2504e0-4f89-11d3-9a0c-0305e82c3301")).toBe("/projects/3f2504e0-4f89-11d3-9a0c-0305e82c3301");
    expect(routeIdToHref("projects.detail", "p1")).toBe("/projects/p1");
    for (const arg of [undefined, null, "", "../billing", "a/b", "x?y", "%2e%2e", "a".repeat(65)]) {
      expect(routeIdToHref("projects.detail", arg)).toBeNull();
    }
  });

  it("links a report only when the catalog has it", () => {
    for (const r of reportCatalog) expect(routeIdToHref(`reports.${r.slug}`)).toBe(`/reports/${r.slug}`);
    expect(routeIdToHref("reports.secret-report")).toBeNull();
    expect(routeIdToHref("reports.")).toBeNull();
  });

  it("drops unknown ids", () => {
    for (const id of [null, undefined, "", "admin", "https://evil.example", "/billing", "toString", "settings.security", "install.android"]) {
      expect(routeIdToHref(id)).toBeNull();
    }
    expect(isAiRouteId("projects.detail")).toBe(true);
    expect(isAiRouteId("reports.weekly")).toBe(true);
    expect(isAiRouteId("nowhere")).toBe(false);
  });
});

describe("copy", () => {
  const all = [
    DISCLAIMER,
    DEMO_NOTE,
    ...WHATS_SENT.always,
    ...WHATS_SENT.whenAllowed,
    ...WHATS_SENT.never,
    ...Object.values(AI_COPY),
    ...Object.values(AI_PROMPT_CHIPS).flatMap((chips) => chips.map((c) => c.label)),
    quotaNote("2026-08-01T00:00:00Z"),
  ];

  it("never names a credential or a configuration key (RULE 8)", () => {
    for (const text of all) expect(text).not.toMatch(/secret|password|api[_-]?key/i);
  });

  it("carries the spec's disclaimer and demo note", () => {
    expect(DISCLAIMER).toBe(
      "AI-generated from your tracker data. Figures come from the tracker; the wording can be wrong - check before acting. Not for decisions about people on its own.",
    );
    expect(DEMO_NOTE).toBe("Demo - sample AI output from the demo data; nothing was sent anywhere.");
    expect(WHATS_SENT.never).toContain("Screenshots and webcam images");
  });

  it("writes the quota note with its renewal date when known", () => {
    expect(quotaNote("2026-08-01T00:00:00Z")).toBe("This month's AI allowance is used; only summaries generated earlier can be shown. Renews Sat 1 Aug.");
    expect(quotaNote(null)).toBe("This month's AI allowance is used; only summaries generated earlier can be shown.");
    expect(formatAiDate("not a date")).toBeNull();
    expect(formatAiDate(undefined)).toBeNull();
  });

  it("offers prompt chips on every page, with unique ids per page", () => {
    for (const [page, chips] of Object.entries(AI_PROMPT_CHIPS)) {
      expect(chips.length, page).toBeGreaterThan(0);
      expect(new Set(chips.map((c) => c.id)).size).toBe(chips.length);
    }
  });
});

describe("dataCategoryLabel", () => {
  it("names every category the server sends", () => {
    for (const c of ["durations", "times", "activity-percent", "project-names", "task-titles", "task-notes", "app-names", "window-titles", "agent-info", "help-content"]) {
      expect(dataCategoryLabel(c)).not.toBe(c);
    }
    expect(dataCategoryLabel("window-titles")).toBe("Window titles");
  });

  it("spells out an unknown one", () => {
    expect(dataCategoryLabel("calendar-events")).toBe("Calendar events");
    expect(dataCategoryLabel("--")).toBe("Other data");
  });
});

describe("resultBadge", () => {
  it("is AI for generated output, muted for the fallback and the demo, absent for no data", () => {
    expect(resultBadge({ status: "generated" })).toEqual({ label: "AI", tone: "info" });
    expect(resultBadge({ status: "cached" })).toEqual({ label: "AI", tone: "info" });
    expect(resultBadge(null)).toEqual({ label: "AI", tone: "info" });
    expect(resultBadge({ status: "fallback" })).toEqual({ label: "Basic summary", tone: "muted" });
    expect(resultBadge({ status: "demo" })).toEqual({ label: "Sample", tone: "muted" });
    expect(resultBadge({ status: "no-data" })).toBeNull();
  });
});

describe("stableKey", () => {
  it("ignores key order and undefined fields", () => {
    expect(stableKey({ b: 2, a: 1, c: undefined })).toBe(stableKey({ a: 1, b: 2 }));
    expect(stableKey({ a: { y: [1, { q: 1, p: 2 }], x: null } })).toBe('{"a":{"x":null,"y":[1,{"p":2,"q":1}]}}');
  });

  it("tells different inputs apart", () => {
    expect(stableKey({ from: "2026-07-07" })).not.toBe(stableKey({ from: "2026-07-08" }));
    expect(stableKey(undefined)).toBe("");
    expect(stableKey("x")).toBe('"x"');
  });

  it("scopes a cache key to the workspace and feature", () => {
    const input = { scope: "team", from: "a" };
    expect(aiResultCacheKey("w1", "period-summary", input)).toBe('w1|period-summary|{"from":"a","scope":"team"}');
    expect(aiResultCacheKey("w2", "period-summary", input)).not.toBe(aiResultCacheKey("w1", "period-summary", input));
  });
});
