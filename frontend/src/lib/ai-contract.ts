/* ============================================================================
 * AI ASSIST CONTRACT — the wire shapes and routes of the backend's AI assist
 * (AiAssistAppService) and its host console (PlatformAiAppService), mirrored
 * from the frozen contract in the AI assist specification (§5). Every route
 * comes from ABP's auto-API and carries camelCase JSON; Guids and DateTimes
 * are strings, decimals numbers.
 *
 * The browser never talks to an AI provider: every call goes to ABP with the
 * session's bearer token and X-Workspace-Id (hooks/ai-api.ts). The server
 * decides who may use what (`features[].allowed` and `scopes` in the status);
 * the web only reads those answers, it never re-derives permissions.
 *
 * Pure: types, route builders and the error-status map. No React, no network.
 * Both route roots are listed in src/test/api-contract.json.
 * ========================================================================== */

const AI_ASSIST = "/api/app/ai-assist";

/** The fixed AI assist routes (the insight features use aiFeatureRoute). */
export const AI_ROUTES = {
  status: "/api/app/ai-assist/status",
  settings: "/api/app/ai-assist/settings",
  usage: "/api/app/ai-assist/usage",
  ask: "/api/app/ai-assist/ask",
  feedback: "/api/app/ai-assist/feedback",
  platformOverview: "/api/app/platform-ai/overview",
  platformStatus: "/api/app/platform-ai/status",
} as const;

/** Every AI feature, in the order the server lists them in the status. */
export const AI_FEATURE_IDS = [
  "period-summary",
  "member-day-summary",
  "report-explanation",
  "data-quality-review",
  "note-suggestions",
  "task-drafts",
  "ask",
] as const;

export type AiFeatureId = (typeof AI_FEATURE_IDS)[number];

/** The one-shot features that answer with a result card (Ask AI is a conversation). */
export type AiInsightFeatureId = Exclude<AiFeatureId, "ask">;

export function isAiFeatureId(value: unknown): value is AiFeatureId {
  return typeof value === "string" && (AI_FEATURE_IDS as readonly string[]).includes(value);
}

/** The POST route of a feature: `/api/app/ai-assist/{id}`. */
export function aiFeatureRoute<F extends AiFeatureId>(id: F): `/api/app/ai-assist/${F}` {
  return `${AI_ASSIST}/${id}`;
}

/** The features whose input carries `regenerate` (a fresh answer instead of the cached one). */
export const REGENERATABLE_FEATURES: readonly AiInsightFeatureId[] = [
  "period-summary",
  "member-day-summary",
  "report-explanation",
  "data-quality-review",
];

/* ----------------------------- Errors (§5.8) ----------------------------- */

/**
 * AI assist's business error codes and the HTTP status each is returned with.
 * Kept apart from TRACKER_ERROR_STATUS (the core contract's codes).
 */
export const AI_ERROR_STATUS = {
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
} as const;

export type AiErrorCode = keyof typeof AI_ERROR_STATUS;

export function isAiErrorCode(value: unknown): value is AiErrorCode {
  return typeof value === "string" && Object.prototype.hasOwnProperty.call(AI_ERROR_STATUS, value);
}

/* ----------------------------- Route ids (§5.9) ----------------------------- */

/**
 * The places a result may link to. The server emits only these ids; the web
 * maps each to an href (lib/ai-assist routeIdToHref) and drops anything else.
 */
export type AiRouteId =
  | "dashboard"
  | "monitor"
  | "timesheet"
  | "team"
  | "projects"
  | "projects.detail"
  | "activities"
  | "download"
  | "install.windows"
  | "install.macos"
  | "install.linux"
  | "settings"
  | "settings.workspace"
  | "settings.privacy"
  | "reports"
  | `reports.${string}`
  | "billing";

/* ----------------------------- Common DTOs (§5.2) ----------------------------- */

/** "demo" is the web's own: sample output computed from the demo data (lib/ai-demo), never sent by a server. */
export type AiResultStatus = "generated" | "cached" | "fallback" | "no-data" | "demo";

export interface AiResultMetaDto {
  /** The usage record of this result; feedback names it. */
  resultId: string;
  featureId: string;
  status: AiResultStatus;
  /** UTC. */
  generatedAt: string;
  model: string | null;
  promptVersion: string;
  /** What kinds of data went into the request (lib/ai-assist dataCategoryLabel). */
  dataCategories: string[];
  disclaimer: string;
}

/** One figure a result used, already rendered with real names ("Show the figures used"). */
export interface AiFactRefDto {
  id: string;
  label: string;
  display: string;
}

export interface AiInsightItemDto {
  kind: string;
  text: string;
  facts: AiFactRefDto[];
  routeId: string | null;
}

export interface AiNextStepDto {
  label: string;
  routeId: string;
}

/** Why a feature is refused to this caller. */
export type AiFeatureDisabledReason = "operator" | "role" | "self-service-off" | "permission";

/** Whose data a feature may cover for this caller. */
export type AiScope = "team" | "self" | "member";

export interface AiFeatureStatusDto {
  featureId: string;
  allowed: boolean;
  reason: AiFeatureDisabledReason | null;
  /** period-summary: "team"/"self"; member-day-summary: "member"/"self"; data-quality-review: "team"/"self"; others none. */
  scopes: AiScope[];
}

export interface AiQuotaDto {
  requestsUsed: number;
  requestsLimit: number | null;
  tokensUsed: number;
  tokensLimit: number | null;
  resetsAt: string;
}

/* ----------------------------- Status, settings, usage (§5.3) ----------------------------- */

/** The first check that failed, in the order the server runs them; null when AI assist is available. */
export type AiStatusReason =
  | "not-configured"
  | "platform-disabled"
  | "subscription-inactive"
  | "workspace-disabled"
  | "reconsent-required"
  | "role-not-allowed"
  | "self-service-off"
  | "quota-exhausted";

export interface AiStatusDto {
  available: boolean;
  reason: AiStatusReason | null;
  workspaceEnabled: boolean;
  /** The caller is an active owner (may change the AI settings). */
  canManage: boolean;
  /** The processor's display name, only when configured; never a model, endpoint or credential. */
  providerDisplayName: string | null;
  /** "{code}.{operator}", e.g. "1.0". */
  disclosureVersion: string;
  allowWindowTitles: boolean;
  allowNotes: boolean;
  features: AiFeatureStatusDto[];
  /** Owner and manager: the workspace's month; member: null. */
  quota: AiQuotaDto | null;
}

/** Server-localized disclosure, shown before the owner turns AI assist on. */
export interface AiDisclosureDto {
  version: string;
  providerDisplayName: string;
  processingRegion: string | null;
  retentionNote: string | null;
  alwaysSent: string[];
  sentWhenAllowed: string[];
  neverSent: string[];
  whoCanUse: string[];
}

export interface AiWorkspaceSettingsDto {
  isEnabled: boolean;
  allowManagers: boolean;
  allowMemberSelfService: boolean;
  allowWindowTitles: boolean;
  allowNotes: boolean;
  consentVersion: string | null;
  enabledAt: string | null;
  enabledByName: string | null;
  disclosure: AiDisclosureDto;
}

/**
 * `acknowledgedDisclosureVersion` is required, and must equal the current
 * version, when enabling, re-consenting, or turning window titles or notes on
 * (otherwise 400 Tracker:AiDisclosureNotAccepted with data.version).
 */
export interface UpdateAiWorkspaceSettingsDto {
  isEnabled: boolean;
  allowManagers: boolean;
  allowMemberSelfService: boolean;
  allowWindowTitles: boolean;
  allowNotes: boolean;
  acknowledgedDisclosureVersion: string | null;
}

export interface AiUsageByFeatureDto {
  featureId: string;
  requests: number;
  tokens: number;
}

export interface AiUsageSummaryDto {
  monthStart: string;
  resetsAt: string;
  requestsUsed: number;
  requestsLimit: number | null;
  tokensUsed: number;
  tokensLimit: number | null;
  /** Owner only. */
  estimatedCostUsd?: number | null;
  /** Owner only. */
  byFeature?: AiUsageByFeatureDto[] | null;
  myRequestsToday: number;
  myDailyLimit: number | null;
}

/* ----------------------------- Insight features (§5.4) ----------------------------- */

export interface AiPeriodSummaryInput {
  /** UTC, [from, to), at most 31 days. */
  from: string;
  to: string;
  projectId?: string | null;
  scope: "team" | "self";
  timeZone?: string | null;
  regenerate?: boolean;
}

export interface AiPeriodSummaryDto {
  meta: AiResultMetaDto;
  headline: string;
  /** At most 5; kind trend | distribution | coverage | rhythm | data-quality. */
  insights: AiInsightItemDto[];
  nextSteps: AiNextStepDto[];
  caveats: string[];
}

export interface AiMemberDaySummaryInput {
  /** null = the caller. */
  userId?: string | null;
  /** The local day, yyyy-MM-dd. */
  date: string;
  timeZone?: string | null;
  regenerate?: boolean;
}

export interface AiMemberDaySummaryDto {
  meta: AiResultMetaDto;
  headline: string;
  summary: string;
  /** At most 5; kind focus | apps | projects | breaks | rhythm | data-quality. */
  highlights: AiInsightItemDto[];
  caveats: string[];
}

export interface AiReportExplanationInput {
  reportSlug: string;
  /** UTC, at most 366 days. */
  from: string;
  to: string;
  projectId?: string | null;
  userId?: string | null;
  timeZone?: string | null;
  dayBasis?: string | null;
  regenerate?: boolean;
}

export interface AiReportExplanationDto {
  meta: AiResultMetaDto;
  summary: string;
  keyFindings: AiInsightItemDto[];
  caveats: string[];
  questionsToExplore: string[];
}

/** data-gap, clock-skew, unmeasured-activity, software-input (team and self); never-tracked, outdated-agent (team). */
export type AiDataQualityKind =
  | "data-gap"
  | "clock-skew"
  | "unmeasured-activity"
  | "software-input"
  | "never-tracked"
  | "outdated-agent";

export interface AiDataQualityReviewInput {
  /** UTC, at most 31 days. */
  from: string;
  to: string;
  scope: "team" | "self";
  projectId?: string | null;
  timeZone?: string | null;
  kinds?: AiDataQualityKind[] | null;
  regenerate?: boolean;
}

export interface AiDataQualityItemDto {
  candidateId: string;
  kind: string;
  severity: "info" | "notice";
  title: string;
  explanation: string;
  possibleReasons: string[];
  actionId: string | null;
  actionRouteId: string | null;
  /** Rendered: the member's name, or "you". */
  subjectName: string | null;
  subjectUserId: string | null;
  facts: AiFactRefDto[];
}

export interface AiDataQualityReviewDto {
  meta: AiResultMetaDto;
  items: AiDataQualityItemDto[];
  candidateCount: number;
}

export interface AiNoteSuggestionsInput {
  activityId: string;
}

export interface AiNoteSuggestionDto {
  text: string;
  basis: "apps" | "project" | "window-titles" | "previous-note";
}

export interface AiNoteSuggestionsDto {
  meta: AiResultMetaDto;
  suggestions: AiNoteSuggestionDto[];
}

export interface AiTaskDraftsInput {
  projectId: string;
  /** 3-500 characters. */
  goal: string;
  /** 1-8, default 5. */
  maxTasks?: number;
}

export interface AiTaskDraftDto {
  title: string;
  description: string | null;
  rationale: string;
  possibleDuplicateOfTaskId: string | null;
}

export interface AiTaskDraftsDto {
  meta: AiResultMetaDto;
  drafts: AiTaskDraftDto[];
}

export type AiFeedbackRating = "up" | "down";
export type AiFeedbackReason = "inaccurate" | "not-useful" | "tone" | "other";

export interface AiFeedbackInput {
  resultId: string;
  rating: AiFeedbackRating;
  reason: AiFeedbackReason | null;
}

/* ----------------------------- Ask AI (§5.5) ----------------------------- */

export type AiPage =
  | "dashboard"
  | "monitor"
  | "timesheet"
  | "reports"
  | "projects"
  | "activities"
  | "team"
  | "settings"
  | "download"
  | "other";

/** What the page the question was asked on shows; the server checks every id before using it. */
export interface AiPageContextDto {
  page: AiPage;
  reportSlug?: string | null;
  from?: string | null;
  to?: string | null;
  projectId?: string | null;
  memberId?: string | null;
  /** yyyy-MM-dd */
  day?: string | null;
  timeZone?: string | null;
}

export interface AiAskInput {
  conversationId?: string | null;
  /** 1-1000 characters. */
  message: string;
  context?: AiPageContextDto | null;
}

export interface AiAnswerSourceDto {
  label: string;
  routeId: string | null;
  routeArg: string | null;
}

export interface AiAnswerDto {
  meta: AiResultMetaDto;
  conversationId: string;
  /** Plain text; lines starting "- " are list items. */
  answer: string;
  sources: AiAnswerSourceDto[];
  facts: AiFactRefDto[];
  isClarification: boolean;
}

/* ----------------------------- Platform (§5.6) ----------------------------- */

export type PlatformAiProviderState =
  | "configured"
  | "disabled"
  | "missing-api-key"
  | "missing-model"
  | "invalid-endpoint"
  | "unsupported-provider";

export interface PlatformAiTopWorkspaceDto {
  workspaceId: string;
  workspaceName: string;
  requests: number;
  tokens: number;
  estimatedCostUsd: number | null;
}

export interface PlatformAiOverviewDto {
  providerState: PlatformAiProviderState;
  providerDisplayName: string | null;
  defaultModel: string | null;
  configEnabled: boolean;
  runtimeEnabled: boolean;
  breakerState: "closed" | "open" | "half-open";
  monthStart: string;
  requests: number;
  inputTokens: number;
  outputTokens: number;
  estimatedCostUsd: number | null;
  budgetTokens: number | null;
  budgetUsd: number | null;
  fallbackRate: number;
  errorRate: number;
  topWorkspaces: PlatformAiTopWorkspaceDto[];
}

export interface PlatformAiStatusDto {
  enabled: boolean;
}

/* ----------------------------- Feature maps ----------------------------- */

/** What each insight feature takes. */
export interface AiFeatureInputs {
  "period-summary": AiPeriodSummaryInput;
  "member-day-summary": AiMemberDaySummaryInput;
  "report-explanation": AiReportExplanationInput;
  "data-quality-review": AiDataQualityReviewInput;
  "note-suggestions": AiNoteSuggestionsInput;
  "task-drafts": AiTaskDraftsInput;
}

/** What each insight feature answers. */
export interface AiFeatureResults {
  "period-summary": AiPeriodSummaryDto;
  "member-day-summary": AiMemberDaySummaryDto;
  "report-explanation": AiReportExplanationDto;
  "data-quality-review": AiDataQualityReviewDto;
  "note-suggestions": AiNoteSuggestionsDto;
  "task-drafts": AiTaskDraftsDto;
}

/** Any insight result: every one carries `meta`. */
export type AiFeatureResult = AiFeatureResults[AiInsightFeatureId];
