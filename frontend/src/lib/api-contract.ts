/**
 * The "no filter" value of the project and member pickers: a filter left on it
 * is never sent. Defined here, in a module that imports nothing, so any layer
 * can build queries without an import cycle (report-math re-exports it).
 */
export const ALL = "all";

/* ============================================================================
 * API CONTRACT v2 — the wire shapes, routes and rules the web shares with the
 * backend and both desktop agents. One file, typed exactly as the backend
 * serializes (ABP: camelCase JSON, Guids and DateTimes as strings, decimals as
 * numbers).
 *
 * Compatibility rule (contract preamble): everything this round added is
 * optional/nullable, because the web may meet an older server. New response
 * fields are therefore typed optional here — read them with a fallback
 * (`row.idleMinutes ?? 0`), never assume them. Request builders only add new
 * query parameters, which an older server ignores.
 *
 * Every backend route literal in this file is listed in
 * src/test/api-contract.json (the architecture spec enforces it). The pure
 * helpers below (query builders, paging, error parsing, note rules) carry no
 * clock, React or network of their own, so they are unit-tested directly.
 * ========================================================================== */

/* ----------------------------- ABP envelopes ----------------------------- */

export interface ListResultDto<T> {
  items: T[];
}

export interface PagedResultDto<T> {
  items: T[];
  totalCount: number;
}

/** The body of an ABP business or validation error: `{"error": {...}}`. */
export interface AbpErrorDto {
  code?: string | null;
  message?: string | null;
  details?: string | null;
  validationErrors?: { message?: string | null; members?: string[] | null }[] | null;
}

/** Business error codes and the HTTP status each is returned with (contract §1). */
export const TRACKER_ERROR_STATUS = {
  "Tracker:ActivityTimeRangeInvalid": 422,
  "Tracker:ActivityClientIdRequired": 400,
  "Tracker:ActivityClientIdConflict": 409,
  "Tracker:ActivityInFuture": 425,
  "Tracker:ActivityTooLong": 422,
  "Tracker:ActivityTooOld": 422,
  "Tracker:ActivityOverlaps": 409,
  "Tracker:ProjectArchived": 409,
  "Tracker:NotProjectMember": 403,
  "Tracker:InvalidCaptureData": 400,
  "Tracker:CaptureTooLarge": 413,
  "Tracker:SubscriptionInactive": 402,
  "Tracker:SeatLimitReached": 409,
  "Tracker:DuplicateProjectMember": 409,
  "Tracker:PlanNotFound": 404,
  "Tracker:PlanInUse": 409,
  "Tracker:ReportRangeInvalid": 400,
  "Tracker:InvalidUser": 404,
  "Tracker:CannotModifySelf": 403,
  "Tracker:RoleNotAllowed": 403,
  "Tracker:NotWorkspaceMember": 403,
  "Tracker:WorkspaceRequired": 400,
  "Tracker:EmailAlreadyRegistered": 409,
  "Tracker:DuplicateWorkspaceName": 409,
  "Tracker:LastOwnerCannotLeave": 409,
  "Tracker:AlreadyWorkspaceMember": 409,
  "Tracker:InvitationNotFound": 404,
  "Tracker:InvitationExpired": 410,
  "Tracker:InvitationEmailMismatch": 403,
} as const;

export type TrackerErrorCode = keyof typeof TRACKER_ERROR_STATUS;

/** What a failed response said, from either an ABP error body or an OAuth token error. */
export interface ApiErrorInfo {
  /** ABP `error.code` ("Tracker:SeatLimitReached") or OAuth `error` ("invalid_grant"). */
  code: string | null;
  /** ABP `error.message` or OAuth `error_description`: already written for people. */
  message: string | null;
  details: string | null;
  /** DataAnnotations failures (400): one entry per rule, with the offending members. */
  validationErrors: { message: string; members: string[] }[];
}

const EMPTY_ERROR: ApiErrorInfo = { code: null, message: null, details: null, validationErrors: [] };

function text(value: unknown): string | null {
  return typeof value === "string" && value.trim() !== "" ? value : null;
}

/** Read an error response body; anything that isn't one of the two known shapes yields empty fields. */
export function parseApiErrorBody(body: string | null | undefined): ApiErrorInfo {
  if (!body) return EMPTY_ERROR;
  let parsed: unknown;
  try {
    parsed = JSON.parse(body);
  } catch {
    return EMPTY_ERROR;
  }
  if (!parsed || typeof parsed !== "object") return EMPTY_ERROR;
  const root = parsed as { error?: unknown; error_description?: unknown };
  if (root.error && typeof root.error === "object") {
    const e = root.error as AbpErrorDto;
    const validationErrors = (Array.isArray(e.validationErrors) ? e.validationErrors : [])
      .filter((v) => v && typeof v === "object" && text(v.message))
      .map((v) => ({
        message: v.message as string,
        members: Array.isArray(v.members) ? v.members.filter((m): m is string => typeof m === "string") : [],
      }));
    return { code: text(e.code), message: text(e.message), details: text(e.details), validationErrors };
  }
  return { ...EMPTY_ERROR, code: text(root.error), message: text(root.error_description) };
}

/**
 * Seconds to wait from a Retry-After header: either delay-seconds ("120") or
 * an HTTP date. Null when absent or unreadable; never negative.
 */
export function parseRetryAfter(value: string | null | undefined, now: number = Date.now()): number | null {
  if (!value) return null;
  const raw = value.trim();
  if (/^\d+$/.test(raw)) return Number(raw);
  const at = Date.parse(raw);
  return Number.isFinite(at) ? Math.max(0, Math.ceil((at - now) / 1000)) : null;
}

/* ----------------------------- Paging ----------------------------- */

/** ABP's largest accepted page (LimitedResultRequestDto.MaxMaxResultCount). */
export const MAX_PAGE_SIZE = 1000;

/**
 * `endpoint` with its SkipCount/MaxResultCount replaced (any existing ones are
 * dropped, in any letter case); the other parameters are kept byte for byte.
 */
export function withPaging(endpoint: string, skipCount: number, maxResultCount: number): string {
  const at = endpoint.indexOf("?");
  const path = at === -1 ? endpoint : endpoint.slice(0, at);
  const params = (at === -1 ? "" : endpoint.slice(at + 1))
    .split("&")
    .filter((p) => p !== "" && !/^(skipcount|maxresultcount)=/i.test(p));
  params.push(`SkipCount=${Math.max(0, Math.trunc(skipCount))}`, `MaxResultCount=${Math.max(1, Math.trunc(maxResultCount))}`);
  return `${path}?${params.join("&")}`;
}

/** A list payload as `{ items, totalCount }`: ABP's paged envelope, or a bare array (its length is the total). */
export function toPagedResult<T>(payload: unknown): PagedResultDto<T> {
  if (Array.isArray(payload)) return { items: payload as T[], totalCount: payload.length };
  if (payload && typeof payload === "object") {
    const p = payload as { items?: unknown; totalCount?: unknown };
    const items = Array.isArray(p.items) ? (p.items as T[]) : [];
    const total = Number(p.totalCount);
    return { items, totalCount: Number.isFinite(total) && total >= items.length ? total : items.length };
  }
  return { items: [], totalCount: 0 };
}

export interface CollectPagesOptions<T> {
  /** Rows per request, 1…1000 (default 1000). */
  pageSize?: number;
  /** Stop once this many rows are collected; `truncated` then says the list is partial. */
  maxItems?: number;
  /**
   * A row's identity for de-duplication (default: its string `id`). Offset
   * paging over a list that grows while it is read can return a row twice.
   */
  idOf?: (item: T) => unknown;
}

export interface CollectedPages<T> {
  items: T[];
  /** The server's count of matching rows. */
  totalCount: number;
  /** Fewer rows were collected than the server holds: show "showing N of M", never a silent total. */
  truncated: boolean;
}

function defaultId(item: unknown): unknown {
  return item && typeof item === "object" ? (item as { id?: unknown }).id : undefined;
}

/**
 * Read every page of a paged list (SkipCount += page size) until the server's
 * total is reached, a short page arrives, or `maxItems` is hit. Pin the
 * range's end (`To=`) in `endpoint` to the moment the read started so uploads
 * landing mid-read can't shift the pages.
 */
export async function collectPages<T>(
  getPage: (endpoint: string) => Promise<PagedResultDto<T>>,
  endpoint: string,
  options: CollectPagesOptions<T> = {},
): Promise<CollectedPages<T>> {
  const pageSize = Math.min(MAX_PAGE_SIZE, Math.max(1, Math.trunc(options.pageSize ?? MAX_PAGE_SIZE)));
  const maxItems = options.maxItems ?? Number.POSITIVE_INFINITY;
  const idOf = options.idOf ?? defaultId;
  const seen = new Set<unknown>();
  const items: T[] = [];
  let skip = 0;
  let totalCount = 0;
  while (items.length < maxItems) {
    const page = await getPage(withPaging(endpoint, skip, pageSize));
    totalCount = Math.max(totalCount, page.totalCount);
    for (const item of page.items) {
      const id = idOf(item);
      if (id !== undefined && id !== null) {
        if (seen.has(id)) continue;
        seen.add(id);
      }
      items.push(item);
      if (items.length >= maxItems) break;
    }
    skip += page.items.length;
    if (page.items.length < pageSize || skip >= page.totalCount) break;
  }
  totalCount = Math.max(totalCount, items.length);
  return { items, totalCount, truncated: items.length < totalCount };
}

/* ----------------------------- §1 Upload a block (agents) ----------------------------- */

export type AgentPlatform = "windows" | "macos" | "linux";

export const PLATFORM_LABELS: Record<AgentPlatform, string> = { windows: "Windows", macos: "macOS", linux: "Linux" };

/** "Windows" / "macOS" / "Linux"; null for a missing or unknown platform. */
export function platformLabel(platform: string | null | undefined): string | null {
  return platform === "windows" || platform === "macos" || platform === "linux" ? PLATFORM_LABELS[platform] : null;
}

/** Why a block ended. */
export type ActivityEndReason =
  | "interval"
  | "pause"
  | "break"
  | "idle"
  | "lock"
  | "sleep"
  | "project"
  | "task"
  | "signout"
  | "quit"
  | "shutdown";

export const END_REASON_LABELS: Record<ActivityEndReason, string> = {
  interval: "Interval complete",
  pause: "Paused",
  break: "Break",
  idle: "Idle",
  lock: "Screen locked",
  sleep: "Computer slept",
  project: "Project switched",
  task: "Task changed",
  signout: "Signed out",
  quit: "App closed",
  shutdown: "Shut down",
};

/** A readable end reason; null for a missing or unknown one. */
export function endReasonLabel(reason: string | null | undefined): string | null {
  return reason && Object.prototype.hasOwnProperty.call(END_REASON_LABELS, reason)
    ? END_REASON_LABELS[reason as ActivityEndReason]
    : null;
}

export interface WindowInfoDto {
  appName?: string | null;
  /** Agents send "" for running programs (privacy); the server blanks them too. */
  windowTitle?: string | null;
  seconds?: number | null;
  keyboardHits?: number | null;
  mouseClicks?: number | null;
}

/** One minute of a block's timeline (also the stored TimelineJson shape). */
export interface ActivityMinuteDto {
  minute: number;
  keyboardHits: number;
  mouseClicks: number;
  active: boolean;
  appName?: string | null;
  /** Seconds of the minute with any input, 0–90 (newer agents). */
  activeSeconds?: number | null;
}

/** POST /api/app/activity — sent by the desktop agents; the web only reads blocks. */
export interface CreateActivityDto {
  projectId: string;
  clientActivityId: string;
  startedAt: string;
  endedAt: string;
  /** 0–100; null = not measured (input capture unavailable or not allowed). */
  productivity: number | null;
  mouseClicks: number;
  keyboardHits: number;
  description?: string | null;
  activeWindows: WindowInfoDto[];
  runningPrograms: WindowInfoDto[];
  timeline: ActivityMinuteDto[];
  /** JPEG (preferred) or PNG, detected server-side by magic bytes. */
  screenshotPngBase64?: string | null;
  screenshotThumbJpgBase64?: string | null;
  webcamJpgBase64?: string | null;
  platform?: AgentPlatform | null;
  osVersion?: string | null;
  agentVersion?: string | null;
  deviceId?: string | null;
  deviceName?: string | null;
  timeZone?: string | null;
  utcOffsetMinutes?: number | null;
  idleSeconds?: number | null;
  endReason?: ActivityEndReason | null;
  sentAt?: string | null;
  screenshotCapturedAt?: string | null;
  screenshotBlurred?: boolean | null;
  injectedInputEvents?: number | null;
}

/* ----------------------------- §2 Projects ----------------------------- */

/** GET /api/app/project/my-projects item (what the agents track). */
export interface MyProjectDto {
  id: string;
  title: string;
  intervalMinutes: number;
  allowScreenshot: boolean;
  allowWebcam: boolean;
  allowKeyboard: boolean;
  allowMouse: boolean;
  allowActiveWindow: boolean;
  allowRunningPrograms: boolean;
  /** 0 = idle detection off; 1–60 minutes. Default 5 when absent. */
  idleThresholdMinutes?: number;
  /** Default false when absent. */
  blurScreenshots?: boolean;
}

/** GET /api/app/project item. */
export interface ProjectDto {
  id: string;
  title: string;
  description?: string | null;
  color: string;
  intervalMinutes: number;
  isArchived: boolean;
  allowScreenshot: boolean;
  allowWebcam: boolean;
  allowKeyboard: boolean;
  allowMouse: boolean;
  allowActiveWindow: boolean;
  allowRunningPrograms: boolean;
  idleThresholdMinutes?: number;
  blurScreenshots?: boolean;
  archivedAt?: string | null;
  creationTime?: string | null;
}

/** POST/PUT /api/app/project body. */
export interface CreateUpdateProjectDto {
  title: string;
  description?: string | null;
  color: string;
  intervalMinutes: number;
  allowScreenshot: boolean;
  allowWebcam: boolean;
  allowKeyboard: boolean;
  allowMouse: boolean;
  allowActiveWindow: boolean;
  allowRunningPrograms: boolean;
  idleThresholdMinutes: number;
  blurScreenshots: boolean;
}

/** The server's validation limits for a project. */
export const PROJECT_LIMITS = {
  titleMaxLength: 128,
  descriptionMaxLength: 2048,
  intervalMinutes: { min: 5, max: 60, default: 10 },
  idleThresholdMinutes: { min: 0, max: 60, default: 5 },
} as const;

/** PUT target for editing a project (body: CreateUpdateProjectDto). */
export function projectRoute(projectId: string): string {
  return `/api/app/project/${encodeURIComponent(projectId)}`;
}

/** "#1a2b3c": the only colour form the server accepts. */
export function isProjectColor(value: unknown): value is string {
  return typeof value === "string" && /^#[0-9a-fA-F]{6}$/.test(value);
}

/* ----------------------------- §3 Presence ----------------------------- */

export type PresenceStatus = "tracking" | "paused" | "break" | "idle" | "locked" | "choosing" | "signedout" | "stopped";

export const PRESENCE_STATUSES: readonly PresenceStatus[] = [
  "tracking",
  "paused",
  "break",
  "idle",
  "locked",
  "choosing",
  "signedout",
  "stopped",
];

export const PRESENCE_STATUS_LABELS: Record<PresenceStatus, string> = {
  tracking: "Tracking",
  paused: "Paused",
  break: "On a break",
  idle: "Idle",
  locked: "Screen locked",
  choosing: "Choosing a project",
  signedout: "Signed out",
  stopped: "Not running",
};

/** POST /api/app/presence/heartbeat body (sent by the agents every 60 s). */
export interface HeartbeatDto {
  deviceId: string;
  deviceName?: string | null;
  platform?: AgentPlatform | null;
  osVersion?: string | null;
  agentVersion?: string | null;
  status: PresenceStatus;
  projectId?: string | null;
  statusSince?: string | null;
  currentApp?: string | null;
  timeZone?: string | null;
  utcOffsetMinutes?: number | null;
  sentAt?: string | null;
}

export interface HeartbeatResultDto {
  serverTime: string;
}

/** GET /api/app/presence/team item: one member device. */
export interface PresenceDto {
  userId: string;
  userName: string;
  name?: string | null;
  deviceId: string;
  deviceName?: string | null;
  platform?: AgentPlatform | null;
  osVersion?: string | null;
  agentVersion?: string | null;
  status: PresenceStatus;
  projectId?: string | null;
  projectTitle?: string | null;
  statusSince?: string | null;
  lastSeenAt: string;
  /** Last heartbeat within PRESENCE_ONLINE_SECONDS; otherwise the UI shows "Offline · last seen …". */
  online: boolean;
  currentApp?: string | null;
  timeZone?: string | null;
  utcOffsetMinutes?: number | null;
}

/** A device counts as online while its last heartbeat is at most this old. */
export const PRESENCE_ONLINE_SECONDS = 150;

/** A readable presence status ("Screen locked"); "Unknown" for anything else. */
export function presenceStatusLabel(status: string | null | undefined): string {
  return status && Object.prototype.hasOwnProperty.call(PRESENCE_STATUS_LABELS, status)
    ? PRESENCE_STATUS_LABELS[status as PresenceStatus]
    : "Unknown";
}

/**
 * Whether a device is online at `now` — the server's `online` rule, so a page
 * kept open can age a status without refetching. Unreadable times are offline.
 */
export function isPresenceOnline(lastSeenAt: string | null | undefined, now: Date): boolean {
  if (!lastSeenAt) return false;
  const raw = lastSeenAt.trim();
  // ABP may send a UTC time without its zone suffix; read it as UTC, not local time.
  const seen = Date.parse(/^\d{4}-\d{2}-\d{2}T[\d:.]+$/.test(raw) ? `${raw}Z` : raw);
  if (!Number.isFinite(seen)) return false;
  const age = now.getTime() - seen;
  return age <= PRESENCE_ONLINE_SECONDS * 1000;
}

/* ----------------------------- §5 Activities ----------------------------- */

export type CaptureKind = "screen" | "thumb" | "webcam";

export interface ActivityCaptureRefDto {
  id: string;
  kind: CaptureKind;
  /** When the capture was taken (newer servers; older ones send none). */
  capturedAt?: string | null;
  blurred?: boolean | null;
}

/**
 * The file extension for a capture's content type. Screen captures are JPEG
 * (what agents prefer, and so the default) or PNG — the server stores each
 * with its own type (contract §1).
 */
export function captureExtension(contentType: string | null | undefined): "jpg" | "png" | "webp" {
  const type = (contentType ?? "").toLowerCase();
  if (type.includes("png")) return "png";
  if (type.includes("webp")) return "webp";
  return "jpg";
}

/** GET /api/app/activity item. */
export interface ActivityDto {
  id: string;
  userId: string;
  projectId: string;
  clientActivityId?: string;
  startedAt: string;
  endedAt: string;
  /** 0–100, or null when the block's input was not measured. */
  productivity: number | null;
  mouseClicks: number;
  keyboardHits: number;
  description?: string | null;
  activeWindowsJson?: string;
  runningProgramsJson?: string;
  timelineJson?: string;
  captures?: ActivityCaptureRefDto[];
  creationTime?: string | null;
  platform?: AgentPlatform | null;
  osVersion?: string | null;
  agentVersion?: string | null;
  deviceId?: string | null;
  deviceName?: string | null;
  timeZone?: string | null;
  utcOffsetMinutes?: number | null;
  idleSeconds?: number | null;
  endReason?: ActivityEndReason | null;
  /** Server time minus the device clock when the block was sent; positive = device behind. */
  clockSkewSeconds?: number | null;
  /** Software-injected input seen during the block (excluded from counts and activity %). */
  injectedInputEvents?: number | null;
}

/** GET /api/app/activity filters. Paging is stable: newest first, then by id. */
export interface ActivityListQuery {
  projectId?: string | null;
  userId?: string | null;
  from?: Date | null;
  to?: Date | null;
  skipCount?: number;
  maxResultCount?: number;
}

/** PUT /api/app/activity/{id}/note body. */
export interface UpdateActivityNoteDto {
  description: string | null;
}

/** Longest note the server accepts. */
export const ACTIVITY_NOTE_MAX_LENGTH = 2048;

/** How long a block's owner may edit its note (Team.Manage holders always may). */
export const ACTIVITY_NOTE_EDIT_DAYS = 30;

/**
 * The note as the server will store it: whitespace runs (line breaks
 * included) collapse to one space, other control characters go, the ends are
 * trimmed; nothing left means no note (null).
 */
export function normalizeActivityNote(note: string | null | undefined): string | null {
  if (typeof note !== "string") return null;
  let kept = "";
  for (const ch of note) {
    const code = ch.codePointAt(0) ?? 0;
    const control = code < 0x20 || (code >= 0x7f && code <= 0x9f);
    // A control character that is whitespace (tab, line break) still separates words.
    if (control) kept += /\s/.test(ch) ? " " : "";
    else kept += ch;
  }
  const cleaned = kept.replace(/\s+/g, " ").trim();
  return cleaned === "" ? null : cleaned;
}

/** Whether the viewer may edit a block's note (contract §5). */
export function canEditActivityNote(input: {
  /** The viewer owns the block. */
  isOwner: boolean;
  /** The viewer holds Tracker.Team.Manage. */
  canManageTeam: boolean;
  startedAt: string;
  now: Date;
}): boolean {
  if (input.canManageTeam) return true;
  if (!input.isOwner) return false;
  const started = Date.parse(input.startedAt);
  if (!Number.isFinite(started)) return false;
  return input.now.getTime() - started <= ACTIVITY_NOTE_EDIT_DAYS * 86_400_000;
}

/* ----------------------------- §6 Reporting ----------------------------- */

/** "zone": days of the requested zone; "member": each block on the local day of its own device. */
export type DayBasis = "zone" | "member";

/** The backend's longest accepted report range, in days. */
export const MAX_REPORT_RANGE_DAYS = 366;

/** GetReportSummaryInput: [from, to) in UTC plus the scope and the day rule. */
export interface ReportingQuery {
  from: Date;
  to: Date;
  projectId?: string | null;
  userId?: string | null;
  /** IANA zone every "day" is a calendar day of (server default "UTC"). */
  timeZone?: string | null;
  dayBasis?: DayBasis | null;
}

function scoped(id: string | null | undefined): id is string {
  return typeof id === "string" && id !== "" && id !== ALL;
}

/**
 * Query string for the /api/app/reporting/* endpoints. A filter left on "all"
 * (or empty) is not sent; TimeZone and DayBasis only when given, so the
 * string is identical to report-math's reportQuery when they are not.
 */
export function reportingQuery(query: ReportingQuery): string {
  const qs = new URLSearchParams({ From: query.from.toISOString(), To: query.to.toISOString() });
  if (scoped(query.projectId)) qs.set("ProjectId", query.projectId);
  if (scoped(query.userId)) qs.set("UserId", query.userId);
  if (query.timeZone) qs.set("TimeZone", query.timeZone);
  if (query.dayBasis) qs.set("DayBasis", query.dayBasis);
  return qs.toString();
}

/** Query string for GET /api/app/activity. */
export function activitiesQuery(query: ActivityListQuery): string {
  const qs = new URLSearchParams();
  if (scoped(query.projectId)) qs.set("ProjectId", query.projectId);
  if (scoped(query.userId)) qs.set("UserId", query.userId);
  if (query.from) qs.set("From", query.from.toISOString());
  if (query.to) qs.set("To", query.to.toISOString());
  if (query.skipCount !== undefined) qs.set("SkipCount", String(Math.max(0, Math.trunc(query.skipCount))));
  if (query.maxResultCount !== undefined) qs.set("MaxResultCount", String(Math.max(1, Math.trunc(query.maxResultCount))));
  return qs.toString();
}

export interface ReportSummaryDto {
  totalActivities: number;
  totalTrackedMinutes: number;
  /** Minute-weighted over measured blocks; null when nothing was measured. */
  averageProductivity: number | null;
  totalMouseClicks: number;
  totalKeyboardHits: number;
  totalIdleMinutes?: number;
  perUser: UserReportRowDto[];
  perProject: ProjectReportRowDto[];
  perUserProject?: UserProjectReportRowDto[];
}

export interface UserReportRowDto {
  userId: string;
  userName?: string;
  name?: string | null;
  activityCount: number;
  trackedMinutes: number;
  idleMinutes?: number;
  averageProductivity: number | null;
}

export interface ProjectReportRowDto {
  projectId: string;
  projectTitle?: string;
  isArchived?: boolean;
  activityCount: number;
  trackedMinutes: number;
  idleMinutes?: number;
  averageProductivity: number | null;
}

/** One calendar day of the requested zone; `date` is that local date at 00:00 without a zone ("2026-07-14T00:00:00"). */
export interface DailyReportRowDto {
  date: string;
  activityCount: number;
  trackedMinutes: number;
  idleMinutes?: number;
  averageProductivity: number | null;
  mouseClicks: number;
  keyboardHits: number;
}

export interface UserDailyReportRowDto {
  userId: string;
  userName: string;
  date: string;
  trackedMinutes: number;
  activityCount: number;
  idleMinutes?: number;
  /** UTC instant of the day's first block start. */
  firstStartedAt?: string;
  /** UTC instant of the day's last block end. */
  lastEndedAt?: string;
}

export interface UserProjectReportRowDto {
  userId: string;
  userName: string;
  projectId: string;
  projectTitle: string;
  activityCount: number;
  trackedMinutes: number;
  idleMinutes: number;
  averageProductivity: number | null;
  /** Σ(block minutes × the rate in effect at the block's start) / 60, rounded to cents. */
  amount: number;
  /** The effective rate, minute-weighted over the range. */
  hourlyRate: number;
}

/** GET /api/app/reporting/payroll */
export interface PayrollReportDto {
  rows: UserProjectReportRowDto[];
  totalMinutes: number;
  totalAmount: number;
}

/** GET /api/app/reporting/hourly item (blocks split across hour boundaries, in the requested zone). */
export interface HourlyReportRowDto {
  /** 0 = Sunday … 6 = Saturday */
  dayOfWeek: number;
  /** 0–23 */
  hour: number;
  trackedMinutes: number;
  activeMinutes: number;
}

/** GET /api/app/reporting/devices item (grouped by member and device). */
export interface DeviceReportRowDto {
  userId: string;
  userName: string;
  deviceId?: string | null;
  deviceName?: string | null;
  platform?: AgentPlatform | null;
  osVersion?: string | null;
  agentVersion?: string | null;
  lastActivityAt: string;
  trackedMinutes: number;
}

/** GET /api/app/reporting/app-usage item; rows are grouped by the canonical app name (see lib/apps). */
export interface AppUsageRowDto {
  appName: string;
  /** The canonical name (contract §8); absent from older servers — derive it with canonicalAppName. */
  displayName?: string;
  trackedMinutes: number;
  idleMinutes?: number;
  activityCount: number;
  userCount: number;
}

export interface AttendanceRowDto {
  userId: string;
  userName?: string;
  daysPresent: number;
  /** Weekdays fully inside the range, not in the future, with nothing tracked. */
  daysAbsent: number;
  weekendDaysWorked?: number;
  workedMinutes: number;
  idleMinutes?: number;
  /** Minutes after local midnight of the first start, averaged over present days. */
  averageStartMinutes?: number | null;
  /** Minutes after local midnight of the last end, averaged over present days. */
  averageEndMinutes?: number | null;
  presentDays: string[];
}

/** GET /api/app/reporting/attendance: every member of the scoped projects (clients excluded). */
export interface AttendanceReportDto {
  /** Weekdays of the range in the requested zone. */
  days: string[];
  rows: AttendanceRowDto[];
}

/* ----------------------------- §7 Team, workspace, host ----------------------------- */

/** A member's role on one project. Clients follow a project's progress and never track. */
export type ProjectRole = "Admin" | "Manager" | "Worker" | "Client";

export const PROJECT_ROLES: readonly ProjectRole[] = ["Admin", "Manager", "Worker", "Client"];

/** The roles only a workspace owner may grant (contract §7). */
export const OWNER_ONLY_ROLES: readonly ProjectRole[] = ["Admin", "Manager"];

/** What the signed-in user may change on someone's project membership (the server enforces the same). */
export interface MembershipEditRules {
  /** Roles the caller may set; empty when the role can't be changed. */
  roles: readonly ProjectRole[];
  canChangeRole: boolean;
  canChangeRate: boolean;
}

/**
 * Contract §7: only workspace owners grant Admin/Manager or change hourly
 * rates; nobody changes their own role or rate (403 Tracker:CannotModifySelf);
 * without Tracker.Team.Manage nothing changes at all.
 */
export function membershipEditRules(input: { canManageTeam: boolean; callerIsOwner: boolean; isSelf: boolean }): MembershipEditRules {
  if (!input.canManageTeam || input.isSelf) return { roles: [], canChangeRole: false, canChangeRate: false };
  const roles = input.callerIsOwner ? PROJECT_ROLES : PROJECT_ROLES.filter((r) => !OWNER_ONLY_ROLES.includes(r));
  return { roles, canChangeRole: true, canChangeRate: input.callerIsOwner };
}

/**
 * Actions on another member: changing their workspace role (owners only) and
 * removal from the workspace (never yourself or an owner).
 */
export function memberAccountActions(input: {
  canManageTeam: boolean;
  callerIsOwner: boolean;
  targetIsOwner: boolean;
  isSelf: boolean;
}): { canChangeWorkspaceRole: boolean; canRemoveFromWorkspace: boolean } {
  const allowed = input.canManageTeam && !input.isSelf;
  return {
    canChangeWorkspaceRole: allowed && input.callerIsOwner,
    canRemoveFromWorkspace: allowed && !input.targetIsOwner,
  };
}

/** POST /api/app/team/member body: add an existing workspace user to a project (404 Tracker:InvalidUser otherwise). */
export interface CreateProjectMemberDto {
  projectId: string;
  userId: string;
  role: ProjectRole;
  hourlyRate: number;
}

/** POST /api/app/team/invite-member body. */
export interface InviteMemberDto {
  projectId?: string | null;
  email: string;
  role: ProjectRole;
  /** owner | manager | member (default member). */
  workspaceRole?: string | null;
  hourlyRate: number;
}

/**
 * POST /api/app/team/invite-member result. An existing account is `added` at
 * once; anyone else gets an invitation link (`invitePath`, "/invite/{token}").
 */
export interface InviteMemberResultDto {
  added: boolean;
  workspaceRole: string;
  member?: ProjectMemberDto | null;
  invitationId?: string | null;
  invitePath?: string | null;
  expiresAt?: string | null;
}

/** PUT /api/app/team/member?projectId=&userId= body. A new rate applies from now on (rates are effective-dated). */
export interface UpdateProjectMemberDto {
  role: ProjectRole;
  hourlyRate: number;
}

/** GET /api/app/team/project-members/{projectId} item. */
export interface ProjectMemberDto {
  id: string;
  projectId: string;
  userId: string;
  /** "Admin" | "Manager" | "Worker" | "Client" (a free-form label on older servers). */
  role: string;
  hourlyRate: number;
  userName?: string;
  name?: string | null;
  surname?: string | null;
  email?: string | null;
  creationTime?: string | null;
}

/** GET /api/app/team/members item. */
export interface TeamMemberDto {
  userId: string;
  userName: string;
  name?: string | null;
  surname?: string | null;
  email?: string | null;
  isActive: boolean;
  /** Workspace role: owner | manager | member. */
  role?: string | null;
  isManager: boolean;
  isOwner: boolean;
  isClient?: boolean;
  joinedAt?: string | null;
  projectIds: string[];
}

/** POST /api/account/my-profile/change-password body. */
export interface ChangePasswordInput {
  currentPassword: string;
  newPassword: string;
}

/** What a new project starts with (workspace settings). */
export interface NewProjectDefaultsDto {
  intervalMinutes: number;
  allowScreenshot: boolean;
  allowWebcam: boolean;
  allowKeyboard: boolean;
  allowMouse: boolean;
  allowActiveWindow: boolean;
  allowRunningPrograms: boolean;
  blurScreenshots: boolean;
  idleThresholdMinutes: number;
}

/** GET/PUT /api/app/workspace/settings (workspace owners). */
export interface WorkspaceSettingsDto {
  defaultTimeZone: string | null;
  newProjectDefaults: NewProjectDefaultsDto;
}

/** GET/PUT /api/app/workspace/my-preferences (any member). */
export interface MyPreferencesDto {
  timeZone?: string | null;
}

export interface PlanDto {
  id: string;
  name: string;
  pricePerUser: number;
  maxSeats: number;
  trialDays: number;
}

/** POST /api/app/platform/plan and PUT /api/app/platform/{id}/plan body (host). */
export interface CreateUpdatePlanDto {
  name: string;
  pricePerUser: number;
  maxSeats: number;
  trialDays: number;
}

export interface SubscriptionDto {
  id: string;
  planId: string;
  status: string;
  trialEndsAt?: string | null;
}

export interface InvoiceDto {
  id: string;
  amount: number;
  status: string;
  dueDate: string;
  /** Start of the billed period (unique per tenant); absent from older servers. */
  billingPeriodStart?: string | null;
}

export interface PlatformInvoiceDto {
  id: string;
  workspaceId?: string | null;
  workspaceName: string;
  amount: number;
  status: string;
  dueDate: string;
  billingPeriodStart?: string | null;
}

/* ----------------------------- Routes ----------------------------- */

/** Fixed routes the web calls (each is listed in src/test/api-contract.json). */
export const API_ROUTES = {
  activities: "/api/app/activity",
  presenceTeam: "/api/app/presence/team",
  changePassword: "/api/account/my-profile/change-password",
  workspaceSettings: "/api/app/workspace/settings",
  myPreferences: "/api/app/workspace/my-preferences",
} as const;

export type ReportingEndpoint =
  | "summary"
  | "daily-series"
  | "user-daily-series"
  | "app-usage"
  | "attendance"
  | "payroll"
  | "hourly"
  | "devices";

export const REPORTING_ROUTES: Record<ReportingEndpoint, string> = {
  summary: "/api/app/reporting/summary",
  "daily-series": "/api/app/reporting/daily-series",
  "user-daily-series": "/api/app/reporting/user-daily-series",
  "app-usage": "/api/app/reporting/app-usage",
  attendance: "/api/app/reporting/attendance",
  payroll: "/api/app/reporting/payroll",
  hourly: "/api/app/reporting/hourly",
  devices: "/api/app/reporting/devices",
};

/** A reporting endpoint with its query: `reportingRoute("payroll", { from, to, timeZone })`. */
export function reportingRoute(endpoint: ReportingEndpoint, query: ReportingQuery): string {
  return `${REPORTING_ROUTES[endpoint]}?${reportingQuery(query)}`;
}

/** GET /api/app/activity with its filters. */
export function activitiesRoute(query: ActivityListQuery = {}): string {
  const qs = activitiesQuery(query);
  return qs ? `${API_ROUTES.activities}?${qs}` : API_ROUTES.activities;
}

/** PUT target for a block's note. */
export function activityNoteRoute(activityId: string): string {
  return `/api/app/activity/${encodeURIComponent(activityId)}/note`;
}

/** The team endpoints with fixed paths: the roster (GET), adding a member (POST) and inviting one (POST). */
export const TEAM_ROUTES = {
  members: "/api/app/team/members",
  member: "/api/app/team/member",
  inviteMember: "/api/app/team/invite-member",
} as const;

/** GET: a project's memberships (role, rate, and the member's names on newer servers). */
export function projectMembersRoute(projectId: string): string {
  return `/api/app/team/project-members/${encodeURIComponent(projectId)}`;
}

/** PUT: change a member's role and/or rate on one project (Team.Manage; see membershipEditRules). */
export function updateMemberRoute(projectId: string, userId: string): string {
  return `/api/app/team/member?projectId=${encodeURIComponent(projectId)}&userId=${encodeURIComponent(userId)}`;
}

/** DELETE: remove one membership, by its id from project-members (ABP's secondary-id route). */
export function removeMemberRoute(membershipId: string): string {
  return `/api/app/team/member/${encodeURIComponent(membershipId)}`;
}

/** DELETE: end someone's membership of the current workspace (their account stays). */
export function removeFromWorkspaceRoute(userId: string): string {
  return `/api/app/team/from-workspace?userId=${encodeURIComponent(userId)}`;
}

/** PUT (owner): change a member's workspace role (owner | manager | member). */
export function workspaceRoleRoute(userId: string, role: string): string {
  return `/api/app/team/workspace-role?userId=${encodeURIComponent(userId)}&role=${encodeURIComponent(role)}`;
}

/** GET pending invitations of the current workspace. */
export const TEAM_INVITATIONS_ROUTE = "/api/app/team/invitations";

/** DELETE: revoke a pending invitation. */
export function revokeInvitationRoute(invitationId: string): string {
  return `/api/app/team/${encodeURIComponent(invitationId)}/invitation`;
}

/** Workspace endpoints that are not scoped to the current workspace (no X-Workspace-Id). */
export const WORKSPACE_ROUTES = {
  mine: "/api/app/workspaces/mine",
  create: "/api/app/workspaces",
  register: "/api/app/workspace/register",
} as const;

export function leaveWorkspaceRoute(workspaceId: string): string {
  return `/api/app/workspaces/${encodeURIComponent(workspaceId)}/leave`;
}

export function invitationRoute(token: string): string {
  return `/api/app/invitations/${encodeURIComponent(token)}`;
}

export function acceptInvitationRoute(token: string): string {
  return `${invitationRoute(token)}/accept`;
}

/** GET /api/app/team/invitations item. */
export interface WorkspaceInvitationDto {
  id: string;
  email: string;
  role: string;
  projectId?: string | null;
  projectRole?: string | null;
  expiresAt: string;
  creationTime?: string;
}

/** GET /api/app/invitations/{token} (anonymous). */
export interface InvitationInfoDto {
  workspaceName: string;
  email: string;
  role: string;
  expired: boolean;
}

/** POST /api/app/invitations/{token}/accept. */
export interface AcceptInvitationResultDto {
  workspaceId: string;
  workspaceName: string;
  role: string;
}

/** POST /api/app/workspace/register body (anonymous). */
export interface RegisterWorkspaceInput {
  firstName?: string;
  surname?: string;
  email: string;
  password: string;
  workspaceName?: string;
  planId?: string;
  planName?: string;
  inviteToken?: string;
}

export interface WorkspaceRegistrationResultDto {
  workspaceId: string;
  workspaceName: string;
  userName: string;
}

/** POST (platform): move a workspace to another plan. */
export function changeWorkspacePlanRoute(workspaceId: string, planId: string): string {
  return `/api/app/platform/change-workspace-plan?workspaceId=${encodeURIComponent(workspaceId)}&planId=${encodeURIComponent(planId)}`;
}

/** POST (platform): mark a workspace's subscription active. */
export function activateSubscriptionRoute(workspaceId: string): string {
  return `/api/app/platform/activate-subscription?workspaceId=${encodeURIComponent(workspaceId)}`;
}

/** POST (platform): suspend a workspace — it can't record time until activated again. */
export function suspendSubscriptionRoute(workspaceId: string): string {
  return `/api/app/platform/suspend-subscription?workspaceId=${encodeURIComponent(workspaceId)}`;
}

/** GET (platform): every workspace with its plan, subscription status and occupied seats. */
export const WORKSPACE_SUBSCRIPTIONS_ROUTE = "/api/app/platform/workspace-subscriptions";

/** GET /api/app/platform/workspace-subscriptions item. */
export interface WorkspaceSubscriptionDto {
  workspaceId: string;
  workspaceName: string;
  /** Null (as are the plan and status fields) when the workspace has no subscription. */
  subscriptionId?: string | null;
  planId?: string | null;
  planName?: string | null;
  /** trialing | active | past_due | suspended | canceled | expired */
  status?: string | null;
  trialEndsAt?: string | null;
  /** Occupied seats (the count that is invoiced and limited by the plan). */
  seatsUsed: number;
  /** The plan's seat limit; null without a known plan. */
  maxSeats?: number | null;
}

/** POST (host): record an invoice as paid. */
export function markInvoicePaidRoute(invoiceId: string): string {
  return `/api/app/platform/mark-invoice-paid?invoiceId=${encodeURIComponent(invoiceId)}`;
}
