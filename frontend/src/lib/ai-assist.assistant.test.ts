// Ask AI and the FE-04 surfaces, the web's side (AI assist spec §5.5, §18.3,
// §18.4): which page a path asks about, what the launcher offers each caller,
// the drawer's scope chip, an answer's plain text as paragraphs and lists
// (nothing else interpreted), the data-quality words (severity never by
// colour alone), and the task-draft checks — which match the task form's.
import { describe, expect, it } from "vitest";
import {
  AGENT_HEALTH_KINDS,
  AI_GOAL_MAX_LENGTH,
  AI_GOAL_MIN_LENGTH,
  AI_MAX_TASK_DRAFTS,
  AI_PROMPT_CHIPS,
  AI_QUESTION_MAX_LENGTH,
  AI_SETUP_HELP_CHIPS,
  TIMESHEET_DATA_QUALITY_KINDS,
  aiAnswerBlocks,
  aiAskScopeLabel,
  aiAssistantMode,
  aiPageForPath,
  dataQualityActionLabel,
  dataQualitySeverityLabel,
  selectedTasksText,
  taskDraftProblem,
} from "./ai-assist";
import type { AiStatusDto } from "./ai-contract";
import { PROJECT_TASK_LIMITS } from "./api-contract";
import { demoAiStatus } from "./ai-demo-status";
import type { Role } from "./types";

function withReason(reason: AiStatusDto["reason"], patch: Partial<AiStatusDto> = {}): AiStatusDto {
  return { ...demoAiStatus("owner"), available: reason === null, reason, ...patch };
}

function withAsk(allowed: boolean, base: AiStatusDto = demoAiStatus("owner")): AiStatusDto {
  return { ...base, features: base.features.map((f) => (f.featureId === "ask" ? { ...f, allowed, reason: allowed ? null : "role" } : f)) };
}

describe("aiPageForPath", () => {
  it.each([
    ["/dashboard", "dashboard"],
    ["/insights", "dashboard"],
    ["/monitor", "monitor"],
    ["/timesheet", "timesheet"],
    ["/reports/weekly", "reports"],
    ["/projects/p-1", "projects"],
    ["/activities?view=screens", "activities"],
    ["/screenshots", "activities"],
    ["/team", "team"],
    ["/settings", "settings"],
    ["/download", "download"],
    ["/billing", "other"],
    ["/install/macos", "other"],
    ["/", "dashboard"],
  ])("%s asks about %s", (path, page) => {
    expect(aiPageForPath(path)).toBe(page);
  });

  it("reads no page from nothing, and never from a prototype key", () => {
    expect(aiPageForPath(null)).toBe("dashboard");
    expect(aiPageForPath(undefined)).toBe("dashboard");
    expect(aiPageForPath("/constructor")).toBe("other");
    expect(aiPageForPath("/settings#ai")).toBe("settings");
  });
});

describe("aiAssistantMode: what the Ask AI launcher offers (§18.3)", () => {
  it.each<[Role, "ready" | null]>([
    ["owner", "ready"],
    ["admin", "ready"],
    ["worker", "ready"],
    ["client", null],
  ])("an opted-in workspace's %s: %s", (role, mode) => {
    expect(aiAssistantMode(demoAiStatus(role), role)).toBe(mode);
  });

  it("keeps the assistant with the allowance note when the quota is used", () => {
    expect(aiAssistantMode(withReason("quota-exhausted"), "owner")).toBe("quota");
  });

  it("offers the owner of a workspace that hasn't opted in an explainer, and nobody else anything", () => {
    for (const reason of ["workspace-disabled", "reconsent-required"] as const) {
      expect(aiAssistantMode(withReason(reason), "owner")).toBe("turn-on");
      expect(aiAssistantMode(withReason(reason, { canManage: false }), "owner")).toBeNull();
      expect(aiAssistantMode(withReason(reason), "admin")).toBeNull();
      expect(aiAssistantMode(withReason(reason), "worker")).toBeNull();
    }
  });

  it("is hidden without AI on the server, when Ask AI isn't allowed, or for a reason the web doesn't know", () => {
    expect(aiAssistantMode(null, "owner")).toBeNull();
    expect(aiAssistantMode(withReason("not-configured"), "owner")).toBeNull();
    expect(aiAssistantMode(withReason("platform-disabled"), "owner")).toBeNull();
    expect(aiAssistantMode(withReason("subscription-inactive"), "owner")).toBeNull();
    expect(aiAssistantMode(withReason("self-service-off"), "worker")).toBeNull();
    expect(aiAssistantMode(withAsk(false), "owner")).toBeNull();
    expect(aiAssistantMode({ ...demoAiStatus("owner"), available: false, reason: "something-new" as AiStatusDto["reason"] }, "owner")).toBeNull();
  });
});

describe("aiAskScopeLabel: the drawer's scope chip", () => {
  it("says Team for an owner or manager the server lets see the team, else only their own activity", () => {
    expect(aiAskScopeLabel(demoAiStatus("owner"), "owner")).toBe("Team");
    expect(aiAskScopeLabel(demoAiStatus("admin"), "admin")).toBe("Team");
    expect(aiAskScopeLabel(demoAiStatus("worker"), "worker")).toBe("Only your activity");
    expect(aiAskScopeLabel(demoAiStatus("client"), "client")).toBe("Only your activity");
    const selfOnly = demoAiStatus("admin");
    selfOnly.features = selfOnly.features.map((f) => (f.featureId === "period-summary" ? { ...f, scopes: ["self"] } : f));
    expect(aiAskScopeLabel(selfOnly, "admin")).toBe("Only your activity");
  });

  it("falls back to the role when the status says nothing about the team", () => {
    expect(aiAskScopeLabel(null, "owner")).toBe("Team");
    const noSummaries = demoAiStatus("owner");
    noSummaries.features = noSummaries.features.map((f) => (f.featureId === "period-summary" ? { ...f, allowed: false, scopes: [] } : f));
    expect(aiAskScopeLabel(noSummaries, "owner")).toBe("Team");
  });
});

describe("aiAnswerBlocks: an answer's plain text", () => {
  it("makes consecutive '- ' lines one list and every other line a paragraph", () => {
    expect(aiAnswerBlocks("The team tracked 38 h.\nMost time went to:\n- Apollo: 12 h\n- Zephyr: 9 h\n\nCheck the weekly report.\n- One more")).toEqual([
      { kind: "paragraph", text: "The team tracked 38 h." },
      { kind: "paragraph", text: "Most time went to:" },
      { kind: "list", items: ["Apollo: 12 h", "Zephyr: 9 h"] },
      { kind: "paragraph", text: "Check the weekly report." },
      { kind: "list", items: ["One more"] },
    ]);
  });

  it("interprets nothing else: markup and links stay text, CRLF and blank items are handled", () => {
    expect(aiAnswerBlocks("<b>bold</b> [link](https://evil.example)\r\n-   \r\n-not a list\r\n  - indented item  ")).toEqual([
      { kind: "paragraph", text: "<b>bold</b> [link](https://evil.example)" },
      { kind: "paragraph", text: "-not a list" },
      { kind: "list", items: ["indented item"] },
    ]);
    expect(aiAnswerBlocks("")).toEqual([]);
    expect(aiAnswerBlocks(null)).toEqual([]);
    expect(aiAnswerBlocks(undefined)).toEqual([]);
  });
});

describe("data quality words", () => {
  it("names each severity in words", () => {
    expect(dataQualitySeverityLabel("notice")).toBe("Worth a look");
    expect(dataQualitySeverityLabel("info")).toBe("For information");
    expect(dataQualitySeverityLabel("anything")).toBe("For information");
  });

  it("names each action the server sends, and 'Open' for one it doesn't know yet", () => {
    for (const id of ["check-agent-running", "check-os-permissions", "send-install-link", "update-agent", "sync-device-clock"]) {
      expect(dataQualityActionLabel(id)).not.toBe("Open");
    }
    expect(dataQualityActionLabel("update-agent")).toBe("Get the latest agent");
    expect(dataQualityActionLabel("brand-new")).toBe("Open");
    expect(dataQualityActionLabel("toString")).toBe("Open");
    expect(dataQualityActionLabel(null)).toBe("Open");
    expect(dataQualityActionLabel(undefined)).toBe("Open");
  });

  it("asks the timesheet for the checks that also apply to one member, and agent health for the agents' side", () => {
    expect(TIMESHEET_DATA_QUALITY_KINDS).toEqual(["data-gap", "clock-skew", "unmeasured-activity", "software-input"]);
    expect(AGENT_HEALTH_KINDS).toEqual(["data-gap", "clock-skew", "never-tracked", "outdated-agent"]);
  });
});

describe("task drafts", () => {
  it("checks an edited draft with the task form's own rules", () => {
    expect(taskDraftProblem("Write the release notes", "")).toBeNull();
    expect(taskDraftProblem("   ", "")).toBe("The task needs a title.");
    expect(taskDraftProblem("x".repeat(PROJECT_TASK_LIMITS.titleMaxLength), "")).toBeNull();
    expect(taskDraftProblem(`  ${"x".repeat(PROJECT_TASK_LIMITS.titleMaxLength)}  `, "")).toBeNull();
    expect(taskDraftProblem("x".repeat(PROJECT_TASK_LIMITS.titleMaxLength + 1), "")).toBe(
      `The title cannot exceed ${PROJECT_TASK_LIMITS.titleMaxLength} characters.`,
    );
    expect(taskDraftProblem("Title", "d".repeat(PROJECT_TASK_LIMITS.descriptionMaxLength + 1))).toBe(
      `The description cannot exceed ${PROJECT_TASK_LIMITS.descriptionMaxLength} characters.`,
    );
  });

  it("counts the selection in words", () => {
    expect(selectedTasksText(0)).toBe("0 tasks selected");
    expect(selectedTasksText(1)).toBe("1 task selected");
    expect(selectedTasksText(3)).toBe("3 tasks selected");
  });

  it("uses the server's limits", () => {
    expect([AI_GOAL_MIN_LENGTH, AI_GOAL_MAX_LENGTH, AI_MAX_TASK_DRAFTS, AI_QUESTION_MAX_LENGTH]).toEqual([3, 500, 8, 1000]);
  });
});

describe("setup help questions", () => {
  it("lead with installing on macOS and what AI assist sends, and are the help pages' questions", () => {
    expect(AI_SETUP_HELP_CHIPS.slice(0, 2).map((c) => c.label)).toEqual([
      "How do I install the agent on macOS?",
      "What does AI assist send to the provider?",
    ]);
    expect(AI_PROMPT_CHIPS.download).toEqual(AI_SETUP_HELP_CHIPS);
    expect(AI_PROMPT_CHIPS.settings).toEqual(AI_SETUP_HELP_CHIPS);
    expect(AI_PROMPT_CHIPS.team.map((c) => c.id)).toContain("help-install");
  });
});
