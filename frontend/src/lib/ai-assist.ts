import {
  type AiFeatureId,
  type AiPage,
  type AiResultMetaDto,
  type AiRouteId,
  type AiScope,
  type AiStatusDto,
} from "./ai-contract";
import { reportCatalog } from "./reports-data";
import { formatInZone } from "./time-zone";
import type { Role } from "./types";

/* ============================================================================
 * AI ASSIST, the web's side: when a card shows (the gating matrix of the AI
 * assist specification §18.3), what a failed request tells the person, where
 * a result may link (§5.9), and the copy every AI surface shares.
 *
 * Pure: no React, no network. The server computes who may use what
 * (`features[].allowed`, `scopes`); nothing here re-derives a permission, it
 * only reads the status the server sent.
 *
 * Copy rule: no AI text names a credential or a configuration key. Say "AI
 * provider connection".
 * ========================================================================== */

/* ----------------------------- Copy ----------------------------- */

/** Under every AI result. */
export const DISCLAIMER =
  "AI-generated from your tracker data. Figures come from the tracker; the wording can be wrong - check before acting. Not for decisions about people on its own.";

/** Under every demo result instead of the disclaimer. */
export const DEMO_NOTE = "Demo - sample AI output from the demo data; nothing was sent anywhere.";

/** What AI assist sends, and never sends (the server's disclosure in Settings is the authoritative, localized list). */
export const WHATS_SENT = {
  always: [
    "Tracked durations, idle time, activity % (the share of minutes with keyboard or mouse input) and first and last activity times",
    "Project names and task titles",
    "App names",
    "Agent platform and version (agent health only)",
    "Your Ask AI questions as you type them",
  ],
  whenAllowed: ["Window titles", "Task notes"],
  never: [
    "Screenshots and webcam images",
    "Keystrokes (only counts are recorded, and counts are not sent either)",
    "Member names, emails and user names (replaced with aliases)",
    "Device names, IP addresses and OS versions",
    "Pay rates and amounts",
    "The workspace name",
  ],
} as const;

/** The card copy of §18.3. */
export const AI_COPY = {
  generate: "Generate",
  regenerate: "Regenerate",
  cancel: "Cancel",
  retry: "Retry",
  loading: "Writing summary...",
  slow: "Still working - you can cancel.",
  ready: "Summary ready",
  noData: "Nothing was tracked for this period.",
  aiBadge: "AI",
  fallbackBadge: "Basic summary",
  sampleBadge: "Sample",
  fallbackNote: "AI is unavailable right now - this summary is built directly from your data.",
  cachedNote: "reused, no new data",
  whatsSent: "What's sent?",
  figuresUsed: "Show the figures used",
  serverUnavailable: "AI assist isn't available on this server right now.",
  turnOn: "Turn on AI assist",
  askOwner: "Your workspace owner can turn on AI assist",
} as const;

/** "Generate stays enabled" note while the month's allowance is used up. */
export function quotaNote(resetsAt: string | null | undefined, timeZone = "UTC"): string {
  const date = formatAiDate(resetsAt, timeZone);
  return `This month's AI allowance is used; only summaries generated earlier can be shown.${date ? ` Renews ${date}.` : ""}`;
}

/** "Fri 1 Aug" for an ISO instant read in a zone; null when unreadable. */
export function formatAiDate(iso: string | null | undefined, timeZone = "UTC"): string | null {
  const at = typeof iso === "string" ? Date.parse(iso) : NaN;
  if (!Number.isFinite(at)) return null;
  return formatInZone(at, timeZone, { weekday: "short", day: "numeric", month: "short" }).replace(",", "");
}

/* ----------------------------- Availability (§18.3) ----------------------------- */

/**
 * Why a card shows as it does:
 *  - "ready": Generate;
 *  - "quota": Generate stays enabled with the quota note (the server answers
 *    with earlier summaries, or AiQuotaExceeded);
 *  - "unavailable": no status (older server, failed read), not configured,
 *    switched off for the platform, or the subscription isn't usable;
 *  - "workspace-off" / "reconsent": the workspace hasn't opted in (the owner
 *    sees a separate "Turn on" prompt, see aiSetupNotice);
 *  - "not-allowed": this feature, or this scope, is not for this caller.
 */
export type AiAvailabilityReason = "ready" | "quota" | "unavailable" | "workspace-off" | "reconsent" | "not-allowed";

export interface AiAvailability {
  /** Render the card at all. Hidden is the default for anything not plainly allowed ("no broken buttons"). */
  visible: boolean;
  /** Generate can be pressed: every visible card's can (the server answers an exhausted quota itself). */
  enabled: boolean;
  reason: AiAvailabilityReason;
}

const HIDDEN_BY_STATUS: Partial<Record<NonNullable<AiStatusDto["reason"]>, AiAvailabilityReason>> = {
  "not-configured": "unavailable",
  "platform-disabled": "unavailable",
  "subscription-inactive": "unavailable",
  "workspace-disabled": "workspace-off",
  "reconsent-required": "reconsent",
  "role-not-allowed": "not-allowed",
  "self-service-off": "not-allowed",
};

const hidden = (reason: AiAvailabilityReason): AiAvailability => ({ visible: false, enabled: false, reason });

/** Whether a feature's card shows for this caller, from the server's status alone. */
export function aiAvailability(status: AiStatusDto | null | undefined, featureId: AiFeatureId, scope?: AiScope): AiAvailability {
  if (!status) return hidden("unavailable");
  const blocked = status.reason ? HIDDEN_BY_STATUS[status.reason] : undefined;
  if (blocked) return hidden(blocked);
  const quota = status.reason === "quota-exhausted";
  // Not available for a reason the web doesn't know (a newer server): hide rather than guess.
  if (!status.available && !quota) return hidden("unavailable");
  const feature = status.features.find((f) => f.featureId === featureId);
  if (!feature?.allowed) return hidden("not-allowed");
  if (scope && !feature.scopes.includes(scope)) return hidden("not-allowed");
  return { visible: true, enabled: true, reason: quota ? "quota" : "ready" };
}

/**
 * The prompt about AI assist itself (not a feature card):
 *  - "server-unavailable": the owner's Settings > Workspace note (nothing elsewhere);
 *  - "turn-on": the owner's dismissible "Turn on AI assist" (workspace off or re-consent needed);
 *  - "ask-owner": a manager's dismissible "Your workspace owner can turn on AI assist";
 *  - null: nothing to say (ready, or a member or client, who never see either).
 */
export type AiSetupNotice = "server-unavailable" | "turn-on" | "ask-owner" | null;

export function aiSetupNotice(status: AiStatusDto | null | undefined, role: Role): AiSetupNotice {
  const owner = role === "owner";
  if (!status || status.reason === "not-configured" || status.reason === "platform-disabled") {
    return owner ? "server-unavailable" : null;
  }
  if (status.reason !== "workspace-disabled" && status.reason !== "reconsent-required") return null;
  if (owner && status.canManage) return "turn-on";
  return role === "admin" ? "ask-owner" : null;
}

/* ----------------------------- Errors ----------------------------- */

/** Shown when the request never reached the server (the same words as components/layout/request-error). */
export const AI_UNREACHABLE_TEXT = "Couldn't reach the server. Check your connection and try again.";
const AI_FORBIDDEN_TEXT = "You don't have permission to do that.";

export type AiErrorKind =
  | "not-configured"
  | "platform-disabled"
  | "disabled"
  | "feature-disabled"
  | "quota"
  | "rate-limited"
  | "provider-unavailable"
  | "output-invalid"
  | "range-too-large"
  | "disclosure"
  | "conversation-expired"
  | "subscription-inactive"
  | "forbidden"
  | "unreachable"
  | "unknown";

export interface AiErrorState {
  kind: AiErrorKind;
  /** One sentence for the card's alert. */
  message: string;
  /** Wait this long before retrying, when the server said. */
  retryAfterSeconds: number | null;
  /** When an exhausted allowance renews (ISO), when the server said. */
  resetsAt: string | null;
  /** Show Retry. */
  retryable: boolean;
}

/** The fields of hooks/useApi's ApiError this module reads (lib can't import the hook layer). */
interface ErrorLike {
  name?: unknown;
  status?: unknown;
  code?: unknown;
  serverMessage?: unknown;
  retryAfterSeconds?: unknown;
  data?: unknown;
  validationErrors?: unknown;
}

const str = (value: unknown): string | null => (typeof value === "string" && value.trim() !== "" ? value : null);
const num = (value: unknown): number | null => (typeof value === "number" && Number.isFinite(value) && value >= 0 ? Math.ceil(value) : null);

function waitText(seconds: number | null): string {
  if (seconds === null) return "Try again in a moment.";
  return `Try again in ${seconds} ${seconds === 1 ? "second" : "seconds"}.`;
}

/** What a failed AI request tells the person. `timeZone` reads the renewal date of an exhausted allowance. */
export function aiErrorState(error: unknown, options: { timeZone?: string; fallback?: string } = {}): AiErrorState {
  const fallback = options.fallback ?? "Couldn't generate the summary.";
  const state = (kind: AiErrorKind, message: string, retryable: boolean, extra: Partial<AiErrorState> = {}): AiErrorState => ({
    kind,
    message,
    retryable,
    retryAfterSeconds: null,
    resetsAt: null,
    ...extra,
  });

  const e = (error && typeof error === "object" ? error : {}) as ErrorLike;
  if (e.name === "ApiUnreachableError" || error instanceof TypeError) return state("unreachable", AI_UNREACHABLE_TEXT, true);
  if (typeof e.status !== "number") return state("unknown", fallback, true);

  const status = e.status;
  const data = (e.data && typeof e.data === "object" ? e.data : {}) as Record<string, unknown>;
  const reason = str(data.reason);
  const retryAfter = num(data.retryAfterSeconds) ?? num(e.retryAfterSeconds);
  switch (str(e.code)) {
    case "Tracker:AiNotConfigured":
      return state("not-configured", "AI assist isn't available on this server.", false);
    case "Tracker:AiPlatformDisabled":
      return state("platform-disabled", "AI assist is turned off for the platform right now.", false);
    case "Tracker:AiDisabled":
      return state(
        "disabled",
        reason === "reconsent"
          ? "AI assist is paused until the workspace owner reviews the updated AI disclosure."
          : "AI assist is off for this workspace.",
        false,
      );
    case "Tracker:AiFeatureDisabled":
      return state("feature-disabled", "This AI feature isn't available to you.", false);
    case "Tracker:AiQuotaExceeded": {
      const resetsAt = str(data.resetsAt);
      const date = formatAiDate(resetsAt, options.timeZone);
      return state(
        "quota",
        `The AI allowance for this period has been used.${date ? ` It renews on ${date}.` : ""}`,
        false,
        { resetsAt },
      );
    }
    case "Tracker:AiRateLimited":
      return state(
        "rate-limited",
        `${reason === "busy" ? "Another AI request is still running." : "Too many AI requests."} ${waitText(retryAfter)}`,
        true,
        { retryAfterSeconds: retryAfter },
      );
    case "Tracker:AiProviderUnavailable":
      return state("provider-unavailable", "The AI service is slow or unavailable. Try again in a minute.", true, {
        retryAfterSeconds: retryAfter,
      });
    case "Tracker:AiOutputInvalid":
      return state("output-invalid", "The AI couldn't produce a reliable answer. Try again.", true);
    case "Tracker:AiRangeTooLarge": {
      const maxDays = num(data.maxDays);
      return state("range-too-large", `Choose ${maxDays ?? "fewer"} days${maxDays === null ? "" : " or fewer"} for AI summaries.`, false);
    }
    case "Tracker:AiDisclosureNotAccepted": {
      const version = str(data.version);
      return state("disclosure", `Review and accept the AI disclosure${version ? ` (version ${version})` : ""} to turn on AI assist.`, false);
    }
    case "Tracker:AiConversationExpired":
      return state("conversation-expired", "This conversation ended. Start a new one.", false);
    case "Tracker:SubscriptionInactive":
      return state("subscription-inactive", "This workspace's subscription isn't active, so AI assist is paused.", false);
  }

  // Not an AI code: the same order as components/layout/request-error's requestErrorText.
  const rules = Array.isArray(e.validationErrors)
    ? e.validationErrors.map((v) => str((v as { message?: unknown } | null)?.message)).filter((m): m is string => m !== null)
    : [];
  const retryable = status >= 500 || status === 429 || status === 408;
  if (rules.length > 0) return state("unknown", rules.join(" "), false);
  const serverMessage = str(e.serverMessage);
  if (serverMessage) return state(status === 403 ? "forbidden" : "unknown", serverMessage, retryable, { retryAfterSeconds: retryAfter });
  if (status === 403) return state("forbidden", AI_FORBIDDEN_TEXT, false);
  return state("unknown", `${fallback} (HTTP ${status})`, retryable, { retryAfterSeconds: retryAfter });
}

/* ----------------------------- Route ids (§5.9) ----------------------------- */

const FIXED_ROUTES: Record<string, string> = {
  dashboard: "/dashboard",
  monitor: "/monitor",
  timesheet: "/timesheet",
  team: "/team",
  projects: "/projects",
  activities: "/activities",
  download: "/download",
  "install.windows": "/install/windows",
  "install.macos": "/install/macos",
  "install.linux": "/install/linux",
  settings: "/settings",
  "settings.workspace": "/settings?tab=workspace",
  "settings.privacy": "/settings?tab=privacy",
  reports: "/reports",
  billing: "/billing",
};

/** A route argument the web accepts: an id (GUID or demo id), never a path. */
const ROUTE_ARG = /^[A-Za-z0-9-]{1,64}$/;

/**
 * The href of an allowlisted route id; null for anything else (an unknown id,
 * a report the catalog doesn't have, a project link without a usable id).
 * Callers still check `canAccess(role, href)` before rendering a link.
 */
export function routeIdToHref(routeId: string | null | undefined, arg?: string | null): string | null {
  if (!routeId) return null;
  if (Object.prototype.hasOwnProperty.call(FIXED_ROUTES, routeId)) return FIXED_ROUTES[routeId];
  if (routeId === "projects.detail") return arg && ROUTE_ARG.test(arg) ? `/projects/${arg}` : null;
  if (routeId.startsWith("reports.")) {
    const slug = routeId.slice("reports.".length);
    return reportCatalog.some((r) => r.slug === slug) ? `/reports/${slug}` : null;
  }
  return null;
}

/** Narrow a server string to a known route id. */
export function isAiRouteId(value: string | null | undefined): value is AiRouteId {
  return routeIdToHref(value, "x") !== null;
}

/* ----------------------------- Data categories ----------------------------- */

const CATEGORY_LABELS: Record<string, string> = {
  durations: "Tracked durations",
  times: "Start and end times",
  "activity-percent": "Activity % (share of minutes with input)",
  "project-names": "Project names",
  "task-titles": "Task titles",
  "task-notes": "Task notes",
  "app-names": "App names",
  "window-titles": "Window titles",
  "agent-info": "Agent platform and version",
  "help-content": "Help articles",
};

/** The words for a result's `meta.dataCategories` entry; an unknown one is spelled out from its id. */
export function dataCategoryLabel(category: string): string {
  if (Object.prototype.hasOwnProperty.call(CATEGORY_LABELS, category)) return CATEGORY_LABELS[category];
  const words = category.replace(/[-_]+/g, " ").trim();
  return words ? words[0].toUpperCase() + words.slice(1) : "Other data";
}

/* ----------------------------- Result helpers ----------------------------- */

/** The badge of a result: the AI badge, the muted "Basic summary" or "Sample", or none (nothing tracked). */
export function resultBadge(meta: Pick<AiResultMetaDto, "status"> | null | undefined): { label: string; tone: "info" | "muted" } | null {
  switch (meta?.status) {
    case "no-data":
      return null;
    case "fallback":
      return { label: AI_COPY.fallbackBadge, tone: "muted" };
    case "demo":
      return { label: AI_COPY.sampleBadge, tone: "muted" };
    default:
      return { label: AI_COPY.aiBadge, tone: "info" };
  }
}

/* ----------------------------- Prompt chips ----------------------------- */

export interface AiPromptChip {
  /** Stable id: demo answers are keyed by it (lib/ai-demo demoAnswer). */
  id: string;
  /** The question, as sent. */
  label: string;
}

const HELP_CHIPS: AiPromptChip[] = [
  { id: "help-install", label: "How do I install the desktop agent?" },
  { id: "help-not-measured", label: "Why does activity say \"not measured\"?" },
];

/** The suggested questions of each page's Ask AI drawer. */
export const AI_PROMPT_CHIPS: Record<AiPage, readonly AiPromptChip[]> = {
  dashboard: [
    { id: "week-overview", label: "How did this week go?" },
    { id: "top-projects", label: "Which projects took the most time this week?" },
  ],
  monitor: [
    { id: "day-overview", label: "Summarise today's tracked time" },
    { id: "top-apps", label: "Which apps were used most today?" },
  ],
  timesheet: [
    { id: "week-overview", label: "How did this week go?" },
    { id: "data-gaps", label: "Are there gaps in this week's tracking?" },
  ],
  reports: [
    { id: "week-overview", label: "How did this week go?" },
    { id: "top-projects", label: "Which projects took the most time this week?" },
  ],
  projects: [{ id: "top-projects", label: "Which projects took the most time this week?" }],
  activities: [{ id: "top-apps", label: "Which apps were used most today?" }],
  team: [{ id: "data-gaps", label: "Are there gaps in this week's tracking?" }, HELP_CHIPS[0]],
  settings: HELP_CHIPS,
  download: HELP_CHIPS,
  other: [{ id: "week-overview", label: "How did this week go?" }, HELP_CHIPS[0]],
};

/* ----------------------------- Cache keys ----------------------------- */

function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(value).sort()) {
      const v = (value as Record<string, unknown>)[key];
      if (v !== undefined) out[key] = canonical(v);
    }
    return out;
  }
  return value;
}

/**
 * A deterministic key for a request input: key order doesn't matter and
 * undefined fields are dropped, so `{a:1,b:undefined}` and `{a:1}` agree.
 */
export function stableKey(input: unknown): string {
  return JSON.stringify(canonical(input)) ?? "";
}

/** Where a result is kept for this page view: one entry per workspace, feature and input. */
export function aiResultCacheKey(workspaceId: string, featureId: AiFeatureId, input: unknown): string {
  return `${workspaceId}|${featureId}|${stableKey(input)}`;
}
