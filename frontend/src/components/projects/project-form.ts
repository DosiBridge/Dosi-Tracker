import {
  PROJECT_LIMITS,
  isProjectColor,
  type CreateUpdateProjectDto,
  type NewProjectDefaultsDto,
  type ProjectDto,
} from "@/lib/api-contract";
import type { Project, TrackingPermissions } from "@/lib/types";

/* ============================================================================
 * PROJECT FORM — the pure rules behind creating and editing a project: what a
 * new project starts with (the workspace's saved defaults, else the built-in
 * ones), how a stored project becomes an editable draft, what the server will
 * accept (contract §2: title ≤ 128, description ≤ 2048, colour "#rrggbb",
 * interval 5–60 min, idle threshold 0–60 min) and the exact body that is sent
 * (CreateUpdateProjectDto). No React here, so every rule is unit-tested.
 * ========================================================================== */

/** The colours offered as swatches, each with the name a screen reader announces. */
export const PROJECT_SWATCHES: readonly { value: string; name: string }[] = [
  { value: "#0d9488", name: "Teal" },
  { value: "#0284c7", name: "Blue" },
  { value: "#6366f1", name: "Indigo" },
  { value: "#db2777", name: "Pink" },
  { value: "#16a34a", name: "Green" },
  { value: "#d97706", name: "Amber" },
];

/** Screenshot / block intervals offered in the form (minutes, within the server's 5–60). */
export const INTERVAL_CHOICES: readonly number[] = [5, 10, 15, 20, 30, 45, 60];

/** Idle thresholds offered in the form (minutes; 0 turns idle detection off). */
export const IDLE_CHOICES: readonly number[] = [0, 1, 2, 3, 5, 10, 15, 20, 30, 45, 60];

/**
 * What a new project starts with when the workspace has saved no defaults:
 * the agents' own defaults (contract §2) — screenshots, keyboard, mouse,
 * active window and running programs on; webcam and blur off; a 10-minute
 * interval; idle detection after 5 minutes.
 */
export const BUILT_IN_PROJECT_DEFAULTS: NewProjectDefaultsDto = {
  intervalMinutes: PROJECT_LIMITS.intervalMinutes.default,
  allowScreenshot: true,
  allowWebcam: false,
  allowKeyboard: true,
  allowMouse: true,
  allowActiveWindow: true,
  allowRunningPrograms: true,
  blurScreenshots: false,
  idleThresholdMinutes: PROJECT_LIMITS.idleThresholdMinutes.default,
};

/** A project being created or edited: the request body, with the description kept as typed. */
export interface ProjectDraft extends Omit<CreateUpdateProjectDto, "description"> {
  description: string;
}

/** The fields of a draft that can be invalid, each with the sentence to show next to it. */
export type ProjectDraftErrors = Partial<Record<"title" | "description" | "color" | "intervalMinutes" | "idleThresholdMinutes", string>>;

/** The capture switches, in the order the form lists them. */
export const PERMISSION_FIELDS = [
  "allowScreenshot",
  "allowWebcam",
  "allowKeyboard",
  "allowMouse",
  "allowActiveWindow",
  "allowRunningPrograms",
] as const;

export type PermissionField = (typeof PERMISSION_FIELDS)[number];

function boolOr(value: unknown, fallback: boolean): boolean {
  return typeof value === "boolean" ? value : fallback;
}

function wholeIn(value: unknown, min: number, max: number, fallback: number): number {
  const n = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, Math.round(n)));
}

/**
 * The workspace's saved new-project defaults as the server sent them, read
 * defensively: a missing or malformed field keeps the built-in default and
 * numbers are pulled into the server's ranges.
 */
export function normalizeNewProjectDefaults(raw: unknown): NewProjectDefaultsDto {
  const d = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const base = BUILT_IN_PROJECT_DEFAULTS;
  const { intervalMinutes: interval, idleThresholdMinutes: idle } = PROJECT_LIMITS;
  return {
    intervalMinutes: wholeIn(d.intervalMinutes, interval.min, interval.max, base.intervalMinutes),
    allowScreenshot: boolOr(d.allowScreenshot, base.allowScreenshot),
    allowWebcam: boolOr(d.allowWebcam, base.allowWebcam),
    allowKeyboard: boolOr(d.allowKeyboard, base.allowKeyboard),
    allowMouse: boolOr(d.allowMouse, base.allowMouse),
    allowActiveWindow: boolOr(d.allowActiveWindow, base.allowActiveWindow),
    allowRunningPrograms: boolOr(d.allowRunningPrograms, base.allowRunningPrograms),
    blurScreenshots: boolOr(d.blurScreenshots, base.blurScreenshots),
    idleThresholdMinutes: wholeIn(d.idleThresholdMinutes, idle.min, idle.max, base.idleThresholdMinutes),
  };
}

/** A blank draft for a new project, starting from the workspace defaults (built-in ones when there are none). */
export function newProjectDraft(defaults?: NewProjectDefaultsDto | null): ProjectDraft {
  return {
    title: "",
    description: "",
    color: PROJECT_SWATCHES[0].value,
    ...normalizeNewProjectDefaults(defaults ?? BUILT_IN_PROJECT_DEFAULTS),
  };
}

/**
 * An editable draft of a stored project (GET /api/app/project/{id}). Fields an
 * older server doesn't send take the contract's defaults (idle 5 min, no blur),
 * which is also what its agents assume.
 */
export function draftFromProject(project: Partial<ProjectDto>): ProjectDraft {
  const settings = normalizeNewProjectDefaults(project);
  return {
    title: typeof project.title === "string" ? project.title : "",
    description: typeof project.description === "string" ? project.description : "",
    color: isProjectColor(project.color) ? project.color : PROJECT_SWATCHES[0].value,
    ...settings,
  };
}

/** A draft of a demo project (the demo model has no idle threshold or blur; it gets the defaults). */
export function draftFromDemoProject(project: Project): ProjectDraft {
  return draftFromProject({
    title: project.title,
    description: project.description,
    color: project.color,
    intervalMinutes: project.intervalMinutes,
    ...permissionsToDto(project.permissions),
  });
}

/** The demo model's permission flags as the API's allow* fields. */
export function permissionsToDto(p: TrackingPermissions): Pick<ProjectDraft, PermissionField> {
  return {
    allowScreenshot: p.screenshot,
    allowWebcam: p.webcam,
    allowKeyboard: p.keyboard,
    allowMouse: p.mouse,
    allowActiveWindow: p.activeWindow,
    allowRunningPrograms: p.runningPrograms,
  };
}

/** The API's allow* fields as the demo model's permission flags. */
export function permissionsFromDto(d: Pick<ProjectDraft, PermissionField>): TrackingPermissions {
  return {
    screenshot: d.allowScreenshot,
    webcam: d.allowWebcam,
    keyboard: d.allowKeyboard,
    mouse: d.allowMouse,
    activeWindow: d.allowActiveWindow,
    runningPrograms: d.allowRunningPrograms,
  };
}

/** What the server would reject in the tracking settings (block length and idle threshold). */
export function validateTrackingSettings(settings: Pick<NewProjectDefaultsDto, "intervalMinutes" | "idleThresholdMinutes">): ProjectDraftErrors {
  const errors: ProjectDraftErrors = {};
  const { intervalMinutes: interval, idleThresholdMinutes: idle } = PROJECT_LIMITS;
  if (!Number.isInteger(settings.intervalMinutes) || settings.intervalMinutes < interval.min || settings.intervalMinutes > interval.max) {
    errors.intervalMinutes = `Choose ${interval.min}–${interval.max} minutes.`;
  }
  if (
    !Number.isInteger(settings.idleThresholdMinutes) ||
    settings.idleThresholdMinutes < idle.min ||
    settings.idleThresholdMinutes > idle.max
  ) {
    errors.idleThresholdMinutes = `Choose ${idle.max} minutes or fewer (0 turns idle detection off).`;
  }
  return errors;
}

/** What the server would reject, field by field (empty when the draft can be sent). */
export function validateProjectDraft(draft: ProjectDraft): ProjectDraftErrors {
  const errors: ProjectDraftErrors = {};
  const title = draft.title.trim();
  if (!title) errors.title = "Give the project a name.";
  else if (title.length > PROJECT_LIMITS.titleMaxLength) {
    errors.title = `Use ${PROJECT_LIMITS.titleMaxLength} characters or fewer.`;
  }
  if (draft.description.trim().length > PROJECT_LIMITS.descriptionMaxLength) {
    errors.description = `Keep the description to ${PROJECT_LIMITS.descriptionMaxLength.toLocaleString("en")} characters.`;
  }
  if (!isProjectColor(draft.color)) errors.color = "Pick a colour.";
  return { ...errors, ...validateTrackingSettings(draft) };
}

/** Whether a draft has no errors. */
export function isDraftValid(draft: ProjectDraft): boolean {
  return Object.keys(validateProjectDraft(draft)).length === 0;
}

/**
 * The request body for POST/PUT /api/app/project: the name trimmed, an empty
 * description sent as null (never a placeholder such as "No description").
 */
export function toProjectDto(draft: ProjectDraft): CreateUpdateProjectDto {
  const description = draft.description.trim();
  return {
    title: draft.title.trim(),
    description: description === "" ? null : description,
    color: draft.color.toLowerCase(),
    intervalMinutes: draft.intervalMinutes,
    allowScreenshot: draft.allowScreenshot,
    allowWebcam: draft.allowWebcam,
    allowKeyboard: draft.allowKeyboard,
    allowMouse: draft.allowMouse,
    allowActiveWindow: draft.allowActiveWindow,
    allowRunningPrograms: draft.allowRunningPrograms,
    idleThresholdMinutes: draft.idleThresholdMinutes,
    blurScreenshots: draft.blurScreenshots,
  };
}

/** A demo project built from a submitted form (demo mode keeps projects in the browser). */
export function demoProjectFrom(
  dto: CreateUpdateProjectDto,
  init: { id: string; memberIds: string[]; createdAt: string },
): Project {
  return {
    id: init.id,
    title: dto.title,
    description: dto.description ?? "",
    color: dto.color,
    archived: false,
    intervalMinutes: dto.intervalMinutes,
    permissions: permissionsFromDto(dto),
    memberIds: [...init.memberIds],
    createdAt: init.createdAt,
    loggedThisWeek: 0,
    loggedThisMonth: 0,
    loggedTotal: 0,
  };
}

/** "Every 10 min" — how often the agent closes a block and may take a screenshot. */
export function intervalLabel(minutes: number): string {
  return `Every ${minutes} min`;
}

/** "Off" or "After 5 min without input". */
export function idleLabel(minutes: number): string {
  return minutes <= 0 ? "Off" : `After ${minutes} min without input`;
}

/** The short option label for an idle threshold in a select. */
export function idleChoiceLabel(minutes: number): string {
  return minutes <= 0 ? "Off — never stop for idle time" : `${minutes} min`;
}
