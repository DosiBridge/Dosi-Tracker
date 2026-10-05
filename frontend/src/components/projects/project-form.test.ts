import { describe, expect, it } from "vitest";
import {
  BUILT_IN_PROJECT_DEFAULTS,
  demoProjectFrom,
  draftFromDemoProject,
  draftFromProject,
  idleChoiceLabel,
  idleLabel,
  intervalLabel,
  isDraftValid,
  newProjectDraft,
  normalizeNewProjectDefaults,
  permissionsFromDto,
  permissionsToDto,
  toProjectDto,
  validateProjectDraft,
  validateTrackingSettings,
  type ProjectDraft,
} from "./project-form";
import { buildProject } from "@/test/factories";

const draft = (patch: Partial<ProjectDraft> = {}): ProjectDraft => ({ ...newProjectDraft(), title: "Apollo", ...patch });

describe("normalizeNewProjectDefaults", () => {
  it("keeps valid server values", () => {
    const saved = { ...BUILT_IN_PROJECT_DEFAULTS, intervalMinutes: 30, allowWebcam: true, blurScreenshots: true, idleThresholdMinutes: 0 };
    expect(normalizeNewProjectDefaults(saved)).toEqual(saved);
  });

  it("falls back to the built-in default for a missing or malformed field", () => {
    expect(normalizeNewProjectDefaults(null)).toEqual(BUILT_IN_PROJECT_DEFAULTS);
    expect(normalizeNewProjectDefaults({ allowScreenshot: "yes", intervalMinutes: "soon" })).toEqual(BUILT_IN_PROJECT_DEFAULTS);
  });

  it("pulls numbers into the server's ranges", () => {
    const d = normalizeNewProjectDefaults({ intervalMinutes: 2, idleThresholdMinutes: 600 });
    expect(d.intervalMinutes).toBe(5);
    expect(d.idleThresholdMinutes).toBe(60);
    expect(normalizeNewProjectDefaults({ intervalMinutes: 99.6 }).intervalMinutes).toBe(60);
    expect(normalizeNewProjectDefaults({ idleThresholdMinutes: -3 }).idleThresholdMinutes).toBe(0);
  });

  it("the built-in defaults are the agents' own (contract §2): 10-minute blocks, idle after 5, no webcam, no blur", () => {
    expect(BUILT_IN_PROJECT_DEFAULTS).toMatchObject({ intervalMinutes: 10, idleThresholdMinutes: 5, allowWebcam: false, blurScreenshots: false, allowScreenshot: true });
  });
});

describe("drafts", () => {
  it("a new draft starts empty, from the given defaults", () => {
    const d = newProjectDraft({ ...BUILT_IN_PROJECT_DEFAULTS, intervalMinutes: 15 });
    expect(d.title).toBe("");
    expect(d.description).toBe("");
    expect(d.intervalMinutes).toBe(15);
    expect(d.color).toMatch(/^#[0-9a-f]{6}$/);
  });

  it("an older server's project (no idle or blur fields) edits with the contract's defaults", () => {
    const d = draftFromProject({ title: "Old", color: "#123abc", intervalMinutes: 20, allowWebcam: true });
    expect(d).toMatchObject({ title: "Old", color: "#123abc", intervalMinutes: 20, allowWebcam: true, idleThresholdMinutes: 5, blurScreenshots: false });
  });

  it("an invalid stored colour is replaced rather than sent back", () => {
    expect(draftFromProject({ title: "x", color: "url(https://x)" }).color).toMatch(/^#[0-9a-f]{6}$/);
  });

  it("a demo project's permissions map onto the API's allow* fields and back", () => {
    const p = buildProject({ permissions: { screenshot: false, webcam: true, keyboard: true, mouse: false, activeWindow: true, runningPrograms: false } });
    const d = draftFromDemoProject(p);
    expect(permissionsToDto(p.permissions)).toEqual({
      allowScreenshot: false,
      allowWebcam: true,
      allowKeyboard: true,
      allowMouse: false,
      allowActiveWindow: true,
      allowRunningPrograms: false,
    });
    expect(permissionsFromDto(d)).toEqual(p.permissions);
  });
});

describe("validateProjectDraft", () => {
  it("accepts a complete draft", () => {
    expect(validateProjectDraft(draft())).toEqual({});
    expect(isDraftValid(draft())).toBe(true);
  });

  it("requires a name and caps it at the server's 128 characters", () => {
    expect(validateProjectDraft(draft({ title: "   " })).title).toBeDefined();
    expect(validateProjectDraft(draft({ title: "x".repeat(129) })).title).toMatch(/128/);
    expect(validateProjectDraft(draft({ title: "x".repeat(128) })).title).toBeUndefined();
  });

  it("caps the description at 2048 characters and the colour to #rrggbb", () => {
    expect(validateProjectDraft(draft({ description: "x".repeat(2049) })).description).toMatch(/2,048/);
    expect(validateProjectDraft(draft({ color: "#abc" })).color).toBeDefined();
    expect(validateProjectDraft(draft({ color: "red" })).color).toBeDefined();
  });

  it("keeps the block length to 5–60 whole minutes and idle detection to 0–60", () => {
    expect(validateTrackingSettings({ intervalMinutes: 4, idleThresholdMinutes: 5 }).intervalMinutes).toBeDefined();
    expect(validateTrackingSettings({ intervalMinutes: 61, idleThresholdMinutes: 5 }).intervalMinutes).toBeDefined();
    expect(validateTrackingSettings({ intervalMinutes: 7.5, idleThresholdMinutes: 5 }).intervalMinutes).toBeDefined();
    expect(validateTrackingSettings({ intervalMinutes: 10, idleThresholdMinutes: 61 }).idleThresholdMinutes).toBeDefined();
    expect(validateTrackingSettings({ intervalMinutes: 60, idleThresholdMinutes: 0 })).toEqual({});
  });
});

describe("toProjectDto", () => {
  it("trims the name and sends an empty description as null — never a placeholder", () => {
    const dto = toProjectDto(draft({ title: "  Apollo  ", description: "   " }));
    expect(dto.title).toBe("Apollo");
    expect(dto.description).toBeNull();
  });

  it("sends every tracking field the contract defines", () => {
    const dto = toProjectDto(draft({ color: "#ABCDEF", intervalMinutes: 30, idleThresholdMinutes: 0, blurScreenshots: true }));
    expect(dto).toEqual({
      title: "Apollo",
      description: null,
      color: "#abcdef",
      intervalMinutes: 30,
      allowScreenshot: true,
      allowWebcam: false,
      allowKeyboard: true,
      allowMouse: true,
      allowActiveWindow: true,
      allowRunningPrograms: true,
      idleThresholdMinutes: 0,
      blurScreenshots: true,
    });
  });

  it("builds a demo project from the submitted form", () => {
    const p = demoProjectFrom(toProjectDto(draft({ description: "Moon" })), { id: "p9", memberIds: ["u1"], createdAt: "2026-10-05" });
    expect(p).toMatchObject({ id: "p9", title: "Apollo", description: "Moon", archived: false, memberIds: ["u1"], loggedTotal: 0 });
  });
});

describe("labels", () => {
  it("describes the block length and idle detection in words", () => {
    expect(intervalLabel(10)).toBe("Every 10 min");
    expect(idleLabel(0)).toBe("Off");
    expect(idleLabel(5)).toBe("After 5 min without input");
    expect(idleChoiceLabel(0)).toMatch(/^Off/);
    expect(idleChoiceLabel(15)).toBe("15 min");
  });
});
