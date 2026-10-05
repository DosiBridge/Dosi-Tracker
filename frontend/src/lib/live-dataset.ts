import type {
  Activity,
  ActivityMinute,
  BlockEndReason,
  CaptureKind,
  CaptureRef,
  DevicePlatform,
  Project,
  Role,
  ScreenMock,
  User,
  UserStatus,
  WindowInfo,
} from "./types";
import type { AppNotification, AppUsage, NotificationType } from "./reports-data";
import { DEFAULT_SESSION_DESCRIPTION, mainAppOf } from "./activity-report";
import {
  activeMinutes,
  activitiesOnDay,
  activitiesOnZonedDay,
  averageProductivity,
  durationMinutes,
  inRange,
  minutesForProject,
  totalMinutes,
} from "./metrics";
import { END_REASON_LABELS, normalizeActivityNote } from "./api-contract";
import { appCategory, appColor, appKey, friendlyAppName } from "./apps";
import { addZonedDays, browserTimeZone, startOfZonedDay, zonedDayKey, zonedParts } from "./time-zone";
import { colorFromString } from "./utils";

/* ============================================================================
 * LIVE DATASET — pure mapping + derivation from real backend payloads.
 *
 * The demo dataset (mock-data / tenant-data generators) invents people, apps
 * and charts. A live tenant must never see any of that, so every chart the
 * dashboard draws in live mode is DERIVED here from the tenant's real rows, and
 * a series with no rows behind it is genuinely empty — never back-filled.
 *
 * Everything is a pure function of its inputs (the clock is a parameter), so
 * the derivations are unit-tested without a backend.
 * ========================================================================== */

/* ----------------------------- API shapes ----------------------------- */

/** GET /api/app/activity item (ActivityDto). */
export interface ApiActivityDto {
  id: string;
  userId: string;
  projectId: string;
  startedAt: string;
  endedAt: string;
  /** Activity % 0–100; null = the block's input was not measured (contract §1). */
  productivity?: number | null;
  mouseClicks?: number | null;
  keyboardHits?: number | null;
  /** The member's task note. */
  description?: string | null;
  /**
   * JSON string of per-(app, window title) focus over the whole block, sorted
   * by seconds desc: `{ AppName, WindowTitle, Seconds, KeyboardHits, MouseClicks }`.
   * Legacy rows: a single entry with no seconds or counts.
   */
  activeWindowsJson?: unknown;
  runningProgramsJson?: unknown;
  /**
   * JSON string of 1-minute buckets `{ Minute, KeyboardHits, MouseClicks, Active, AppName }`
   * (Minute is 0-based from startedAt). Missing or "[]" on legacy rows.
   */
  timelineJson?: unknown;
  /**
   * The block's stored captures `[{ id, kind: "screen" | "webcam" | "thumb", capturedAt, blurred }]`;
   * absent on older backends (and capturedAt/blurred on pre-v2 ones).
   */
  captures?: unknown;
  /* Agent data v2 (contract §5) — null or absent for blocks from older agents and on older servers. */
  platform?: string | null;
  osVersion?: string | null;
  agentVersion?: string | null;
  deviceId?: string | null;
  deviceName?: string | null;
  timeZone?: string | null;
  utcOffsetMinutes?: number | null;
  idleSeconds?: number | null;
  endReason?: string | null;
  clockSkewSeconds?: number | null;
  injectedInputEvents?: number | null;
}

/** GET /api/app/project item (ProjectDto). */
export interface ApiProjectDto {
  id: string;
  title?: string | null;
  description?: string | null;
  color?: string | null;
  intervalMinutes?: number | null;
  isArchived?: boolean | null;
  allowScreenshot?: boolean | null;
  allowWebcam?: boolean | null;
  allowKeyboard?: boolean | null;
  allowMouse?: boolean | null;
  allowActiveWindow?: boolean | null;
  allowRunningPrograms?: boolean | null;
  /** 0 = idle detection off; absent on older servers (agents then use 5). */
  idleThresholdMinutes?: number | null;
  blurScreenshots?: boolean | null;
  archivedAt?: string | null;
  creationTime?: string | null;
}

/** GET /api/app/team/members item. */
export interface ApiTeamMemberDto {
  userId: string;
  userName?: string | null;
  name?: string | null;
  surname?: string | null;
  email?: string | null;
  isActive?: boolean | null;
  isManager?: boolean | null;
  isOwner?: boolean | null;
  projectIds?: string[] | null;
}

/* ----------------------------- Small guards ----------------------------- */

const DAY_MS = 24 * 60 * 60 * 1000;
const DAY_NAMES = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/**
 * ABP may serialize a UTC DateTime without a zone suffix ("2026-09-23T08:00:00").
 * `Date.parse` would read that as LOCAL time and shift every row by the
 * viewer's offset, so a bare timestamp is pinned to UTC. Returns a canonical
 * `toISOString()` value, or null when the input is not a date at all.
 */
export function normalizeUtcIso(value: unknown): string | null {
  if (typeof value !== "string" || value.trim() === "") return null;
  const raw = value.trim();
  const hasZone = /([zZ]|[+-]\d{2}:?\d{2})$/.test(raw);
  const withZone = !hasZone && /^\d{4}-\d{2}-\d{2}T/.test(raw) ? `${raw}Z` : raw;
  const ms = Date.parse(withZone);
  return Number.isFinite(ms) ? new Date(ms).toISOString() : null;
}

function str(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function num(value: unknown, fallback = 0): number {
  const n = typeof value === "number" ? value : Number(value);
  return Number.isFinite(n) ? n : fallback;
}

/** Case- and separator-insensitive key: "appName" = "AppName" = "app_name". */
const normalizedKey = (key: string) => key.replace(/[_\-\s]/g, "").toLowerCase();

/**
 * Read one field of an agent/backend record whatever its casing: the agents
 * send camelCase, .NET re-serializes PascalCase, older builds used snake_case.
 * The first non-null value under a matching key wins.
 */
function field(rec: Record<string, unknown>, name: string): unknown {
  if (rec[name] !== undefined && rec[name] !== null) return rec[name];
  const want = normalizedKey(name);
  for (const key of Object.keys(rec)) {
    if (normalizedKey(key) === want && rec[key] !== undefined && rec[key] !== null) return rec[key];
  }
  return undefined;
}

/** A reported, non-negative whole count — or undefined when the field is absent or not a number. */
function optionalCount(value: unknown): number | undefined {
  if (value === undefined || value === null || value === "") return undefined;
  const n = typeof value === "number" ? value : Number(value);
  return Number.isFinite(n) ? Math.max(0, Math.round(n)) : undefined;
}

/**
 * The object entries of a JSON-string-or-array payload. Corrupt JSON, a
 * non-array, and non-object entries all drop out silently — agent payloads
 * must never crash a page.
 */
function recordsOf(raw: unknown): Record<string, unknown>[] {
  let value: unknown = raw;
  if (typeof raw === "string") {
    if (raw.trim() === "") return [];
    try {
      value = JSON.parse(raw);
    } catch {
      return [];
    }
  }
  if (!Array.isArray(value)) return [];
  return value.filter(
    (item): item is Record<string, unknown> => !!item && typeof item === "object" && !Array.isArray(item),
  );
}

/**
 * Parse an agent window list. The backend stores whatever the desktop agent
 * sent as a JSON string (camelCase from the agents, PascalCase when re-serialized
 * by .NET), so this accepts a JSON string OR an already-parsed array, any key
 * casing, and silently drops anything malformed. Corrupt JSON yields [] — it
 * must never crash a page. Per-window input counts are kept only when the
 * agent reported them (newer agents), so "not reported" never reads as zero.
 */
export function parseWindowList(raw: unknown): WindowInfo[] {
  const out: WindowInfo[] = [];
  for (const rec of recordsOf(raw)) {
    const appName = str(field(rec, "appName")).trim();
    if (!appName) continue;
    const win: WindowInfo = {
      appName,
      windowTitle: str(field(rec, "windowTitle")),
      seconds: Math.max(0, num(field(rec, "seconds"), 0)),
    };
    const keyboardHits = optionalCount(field(rec, "keyboardHits"));
    const mouseClicks = optionalCount(field(rec, "mouseClicks"));
    if (keyboardHits !== undefined) win.keyboardHits = keyboardHits;
    if (mouseClicks !== undefined) win.mouseClicks = mouseClicks;
    out.push(win);
  }
  return out;
}

/** A block lasts minutes; more than a day of buckets is not a real timeline. */
const MAX_TIMELINE_MINUTES = 24 * 60;

/** A bucket covers a minute; the last one absorbs the remainder (up to 89 s, contract §1). */
const MAX_BUCKET_ACTIVE_SECONDS = 90;

/**
 * Parse a block's per-minute timeline (`timelineJson`), any key casing, sorted
 * by minute. Buckets without a valid minute index are dropped, a duplicated
 * minute keeps its last bucket, and a missing `Active` flag is inferred from
 * the counts. Newer agents report each bucket's `ActiveSeconds` (kept when
 * present, clamped to 0–90). Missing, "[]" or corrupt input → [].
 */
export function parseTimeline(raw: unknown): ActivityMinute[] {
  const byMinute = new Map<number, ActivityMinute>();
  for (const rec of recordsOf(raw)) {
    const minute = num(field(rec, "minute"), Number.NaN);
    if (!Number.isInteger(minute) || minute < 0 || minute >= MAX_TIMELINE_MINUTES) continue;
    const keyboardHits = optionalCount(field(rec, "keyboardHits")) ?? 0;
    const mouseClicks = optionalCount(field(rec, "mouseClicks")) ?? 0;
    const flag = field(rec, "active");
    const bucket: ActivityMinute = {
      minute,
      keyboardHits,
      mouseClicks,
      active: typeof flag === "boolean" ? flag : keyboardHits + mouseClicks > 0,
    };
    const appName = str(field(rec, "appName")).trim();
    if (appName) bucket.appName = appName;
    const activeSeconds = optionalCount(field(rec, "activeSeconds"));
    if (activeSeconds !== undefined) bucket.activeSeconds = Math.min(MAX_BUCKET_ACTIVE_SECONDS, activeSeconds);
    byMinute.set(minute, bucket);
  }
  return [...byMinute.values()].sort((a, b) => a.minute - b.minute);
}

const CAPTURE_KINDS: readonly CaptureKind[] = ["screen", "webcam", "thumb"];

/**
 * Parse a block's capture refs. Entries need an id and a known kind; anything
 * else — including the property older backends never send — yields []. When
 * the server says when a capture was taken and whether the agent blurred it
 * (contract §5), those ride along.
 */
export function parseCaptures(raw: unknown): CaptureRef[] {
  const out: CaptureRef[] = [];
  const seen = new Set<string>();
  for (const rec of recordsOf(raw)) {
    const rawId = field(rec, "id");
    const id = typeof rawId === "number" && Number.isFinite(rawId) ? String(rawId) : str(rawId).trim();
    const kind = str(field(rec, "kind")).trim().toLowerCase() as CaptureKind;
    if (!id || seen.has(id) || !CAPTURE_KINDS.includes(kind)) continue;
    seen.add(id);
    const ref: CaptureRef = { id, kind };
    const capturedAt = normalizeUtcIso(field(rec, "capturedAt"));
    if (capturedAt) ref.capturedAt = capturedAt;
    const blurred = field(rec, "blurred");
    if (typeof blurred === "boolean") ref.blurred = blurred;
    out.push(ref);
  }
  return out;
}

/**
 * Capture ids to try for a list tile, best first: the small "thumb" rendition,
 * then the full screen capture. [] when the block has neither.
 */
export function thumbnailCaptureIds(activity: Pick<Activity, "captures">): string[] {
  const captures = Array.isArray(activity.captures) ? activity.captures : [];
  const thumb = captures.find((c) => c.kind === "thumb");
  const screen = captures.find((c) => c.kind === "screen");
  return [thumb?.id, screen?.id].filter((id): id is string => !!id);
}

/** GET /api/app/activity/screenshots/{activityId} item (capture metadata; no bytes). */
export interface ApiCaptureDto {
  id: string;
  activityId?: string | null;
  kind?: string | null;
  blurred?: boolean | null;
  capturedAt?: string | null;
  sizeBytes?: number | null;
  contentType?: string | null;
  creationTime?: string | null;
}

/** A full-size capture of a block, ready to download and show. */
export interface CaptureMeta {
  id: string;
  kind: "screen" | "webcam";
  capturedAt: string | null;
  blurred: boolean;
  sizeBytes: number | null;
  contentType: string | null;
}

/**
 * Map the screenshots endpoint's metadata list, oldest capture first. Entries
 * without an id are dropped, and so are "thumb" renditions — the detail view
 * shows the full capture. An unknown kind is treated as a screen capture.
 */
export function mapApiCaptureMetas(list: unknown): CaptureMeta[] {
  const out: CaptureMeta[] = [];
  for (const rec of asItems<unknown>(list).filter((x): x is Record<string, unknown> => !!x && typeof x === "object")) {
    const id = str(field(rec, "id")).trim();
    const kind = str(field(rec, "kind")).trim().toLowerCase();
    if (!id || kind === "thumb") continue;
    const size = optionalCount(field(rec, "sizeBytes"));
    out.push({
      id,
      kind: kind === "webcam" ? "webcam" : "screen",
      capturedAt: normalizeUtcIso(field(rec, "capturedAt") ?? field(rec, "creationTime")),
      blurred: field(rec, "blurred") === true,
      sizeBytes: size ?? null,
      contentType: str(field(rec, "contentType")).trim() || null,
    });
  }
  const at = (c: CaptureMeta) => (c.capturedAt ? Date.parse(c.capturedAt) : Number.POSITIVE_INFINITY);
  return out.map((c, i) => ({ c, i })).sort((a, b) => at(a.c) - at(b.c) || a.i - b.i).map(({ c }) => c);
}

/**
 * A neutral, honest stand-in for the screenshot tile of a real activity,
 * labelled with the block's main app (the one with the most focused time).
 */
function screenFor(active: WindowInfo[]): ScreenMock {
  const app = mainAppOf(active) ?? "Desktop";
  return { app, kind: "editor", accent: colorFromString(app, 55, 45) };
}

/** A trimmed, non-empty string, or undefined. */
function text(value: unknown, maxLength?: number): string | undefined {
  const s = str(value).trim();
  if (!s) return undefined;
  return maxLength && s.length > maxLength ? s.slice(0, maxLength) : s;
}

/** A finite whole number, or undefined. */
function wholeNumber(value: unknown): number | undefined {
  if (value === undefined || value === null || value === "") return undefined;
  const n = typeof value === "number" ? value : Number(value);
  return Number.isFinite(n) ? Math.round(n) : undefined;
}

const PLATFORMS: readonly DevicePlatform[] = ["windows", "macos"];

/** The contract's offset range: UTC−14:00 … UTC+14:00. */
const MAX_UTC_OFFSET_MINUTES = 840;

/**
 * The block's device and agent context (contract §5), with only the fields the
 * server sent — an older agent's block carries none of them. Values the server
 * would have stored as null (an unknown platform or end reason, an offset out
 * of range) are dropped here too.
 */
function agentContext(dto: ApiActivityDto, blockSeconds: number): Partial<Activity> {
  const extra: Partial<Activity> = {};
  const platform = str(dto.platform).trim().toLowerCase() as DevicePlatform;
  if (PLATFORMS.includes(platform)) extra.platform = platform;
  const osVersion = text(dto.osVersion, 64);
  if (osVersion) extra.osVersion = osVersion;
  const agentVersion = text(dto.agentVersion, 32);
  if (agentVersion) extra.agentVersion = agentVersion;
  const deviceId = text(dto.deviceId);
  if (deviceId) extra.deviceId = deviceId;
  const deviceName = text(dto.deviceName, 128);
  if (deviceName) extra.deviceName = deviceName;
  const timeZone = text(dto.timeZone, 64);
  if (timeZone) extra.timeZone = timeZone;
  const offset = wholeNumber(dto.utcOffsetMinutes);
  if (offset !== undefined && Math.abs(offset) <= MAX_UTC_OFFSET_MINUTES) extra.utcOffsetMinutes = offset;
  const idle = optionalCount(dto.idleSeconds);
  if (idle !== undefined) extra.idleSeconds = Math.min(idle, blockSeconds);
  const endReason = str(dto.endReason).trim().toLowerCase();
  if (Object.prototype.hasOwnProperty.call(END_REASON_LABELS, endReason)) extra.endReason = endReason as BlockEndReason;
  const skew = wholeNumber(dto.clockSkewSeconds);
  if (skew !== undefined) extra.clockSkewSeconds = skew;
  const injected = optionalCount(dto.injectedInputEvents);
  if (injected !== undefined) extra.injectedInputEvents = injected;
  return extra;
}

/**
 * Map one backend activity. Rows without a usable id/time range are dropped
 * (null) rather than rendered as NaN. Newer agents report focused seconds per
 * window; a legacy row reports none, so — exactly like the backend's app-usage
 * report — its focused app (first active window) is credited with the block's
 * whole duration and flagged `credited`.
 *
 * Activity % is `productivity`; when the server sends null (input not
 * measured) the row is flagged `activityMeasured: false` and its productivity
 * reads 0, which every metric skips (metrics.ts: activityPercentOf). `note` is
 * the member's own task note (null when none), while `description` keeps the
 * neutral headline used when there is no note.
 */
export function mapApiActivity(dto: ApiActivityDto | null | undefined): Activity | null {
  if (!dto || typeof dto !== "object" || !dto.id) return null;
  const startedAt = normalizeUtcIso(dto.startedAt);
  const endedAt = normalizeUtcIso(dto.endedAt);
  if (!startedAt || !endedAt) return null;

  const active = parseWindowList(dto.activeWindowsJson);
  const running = parseWindowList(dto.runningProgramsJson);
  const blockSeconds = Math.max(0, Math.round((Date.parse(endedAt) - Date.parse(startedAt)) / 1000));
  if (active.length > 0 && active.every((w) => w.seconds === 0)) {
    active[0] = { ...active[0], seconds: blockSeconds, credited: true };
  }
  const captures = parseCaptures(dto.captures);
  const measured = dto.productivity !== null && dto.productivity !== undefined && Number.isFinite(Number(dto.productivity));

  return {
    id: String(dto.id),
    userId: String(dto.userId ?? ""),
    projectId: String(dto.projectId ?? ""),
    startedAt,
    endedAt,
    description: str(dto.description).trim() || DEFAULT_SESSION_DESCRIPTION,
    productivity: measured ? Math.min(100, Math.max(0, Math.round(num(dto.productivity)))) : 0,
    mouseClicks: Math.max(0, Math.round(num(dto.mouseClicks))),
    keyboardHits: Math.max(0, Math.round(num(dto.keyboardHits))),
    activeWindows: active,
    runningPrograms: running,
    screen: screenFor(active),
    hasWebcam: captures.some((c) => c.kind === "webcam"),
    online: false,
    timeline: parseTimeline(dto.timelineJson),
    captures,
    note: normalizeActivityNote(dto.description),
    ...(measured ? {} : { activityMeasured: false }),
    ...agentContext(dto, blockSeconds),
  };
}

/** Map + filter a list payload; anything that is not an array maps to []. */
export function mapApiActivities(list: unknown): Activity[] {
  if (!Array.isArray(list)) return [];
  return list
    .map((dto) => mapApiActivity(dto as ApiActivityDto))
    .filter((a): a is Activity => a !== null)
    .sort((a, b) => Date.parse(b.endedAt) - Date.parse(a.endedAt));
}

/**
 * Map one backend project. `memberIds` come from the team roster (the project
 * DTO carries none), and are ALWAYS an array so `.includes` can never throw.
 */
export function mapApiProject(dto: ApiProjectDto, memberIds: string[] = []): Project {
  return {
    id: String(dto.id),
    title: str(dto.title).trim() || "Untitled project",
    description: str(dto.description),
    color: str(dto.color) || colorFromString(String(dto.id), 60, 50),
    archived: dto.isArchived === true,
    intervalMinutes: Math.max(1, Math.round(num(dto.intervalMinutes, 10))),
    permissions: {
      screenshot: dto.allowScreenshot !== false,
      webcam: dto.allowWebcam === true,
      keyboard: dto.allowKeyboard !== false,
      mouse: dto.allowMouse !== false,
      activeWindow: dto.allowActiveWindow !== false,
      runningPrograms: dto.allowRunningPrograms !== false,
    },
    memberIds: Array.isArray(memberIds) ? [...memberIds] : [],
    createdAt: (normalizeUtcIso(dto.creationTime) ?? "").slice(0, 10),
    loggedThisWeek: 0,
    loggedThisMonth: 0,
    loggedTotal: 0,
    ...projectTrackingExtras(dto),
  };
}

/** The v2 tracking settings (contract §2), only when the server sent them. */
function projectTrackingExtras(dto: ApiProjectDto): Partial<Project> {
  const extra: Partial<Project> = {};
  const idle = optionalCount(dto.idleThresholdMinutes);
  if (idle !== undefined) extra.idleThresholdMinutes = Math.min(60, idle);
  if (typeof dto.blurScreenshots === "boolean") extra.blurScreenshots = dto.blurScreenshots;
  const archivedAt = normalizeUtcIso(dto.archivedAt);
  if (archivedAt) extra.archivedAt = archivedAt;
  return extra;
}

/** Project id → member user ids, inverted from the roster's per-member projectIds. */
export function memberIdsByProject(members: ApiTeamMemberDto[]): Map<string, string[]> {
  const map = new Map<string, string[]>();
  for (const m of members) {
    for (const pid of Array.isArray(m.projectIds) ? m.projectIds : []) {
      const list = map.get(pid) ?? [];
      if (!list.includes(m.userId)) list.push(m.userId);
      map.set(pid, list);
    }
  }
  return map;
}

/** Display name from ABP identity fields: "Name Surname" → userName → email → fallback. */
export function displayName(
  fields: { name?: string | null; surname?: string | null; surName?: string | null; userName?: string | null; email?: string | null },
  fallback = "Member",
): string {
  const full = [fields.name, fields.surname ?? fields.surName].map((p) => str(p).trim()).filter(Boolean).join(" ");
  return full || str(fields.userName).trim() || str(fields.email).trim() || fallback;
}

/** Tenant role of a roster member: the owner flag wins, then manager → admin, else worker. */
export function memberRole(m: Pick<ApiTeamMemberDto, "isOwner" | "isManager">): Role {
  if (m.isOwner) return "owner";
  if (m.isManager) return "admin";
  return "worker";
}

export const roleDesignation: Record<Role, string> = {
  host: "Host · Super Admin",
  owner: "Workspace Owner",
  admin: "Administrator",
  worker: "Member",
  client: "Client",
};

/**
 * Without heartbeats, a member counts as "active" while their last block ended
 * at most one interval of its project ago, plus this grace for the upload —
 * a block only arrives when it ends, so a 30-minute project's member is
 * tracking even though their newest block is 25 minutes old.
 */
const ACTIVE_GRACE_MS = 5 * 60 * 1000;

/** The window used when the project's interval isn't known (and the floor for short intervals). */
const ACTIVE_WINDOW_MS = 15 * 60 * 1000;

export interface TeamMemberContext {
  /** IANA zone whose calendar day is "today" (default: UTC). */
  timeZone?: string;
  /** Live presence from the agents' heartbeats, by user id; wins over the row-based guess. */
  presence?: ReadonlyMap<string, UserStatus>;
  /** Project id → capture interval in minutes, to judge whether a member's newest block is still current. */
  intervals?: ReadonlyMap<string, number>;
}

/** Row-based presence: the newest block ended within its project's interval (+ grace). */
function statusFromRows(mine: Activity[], now: Date, intervals?: ReadonlyMap<string, number>): UserStatus {
  let last: Activity | null = null;
  let lastEnd = 0;
  for (const a of mine) {
    const end = Date.parse(a.endedAt);
    if (Number.isFinite(end) && end > lastEnd) {
      lastEnd = end;
      last = a;
    }
  }
  if (!last) return "offline";
  const interval = intervals?.get(last.projectId);
  const window = typeof interval === "number" && interval > 0 ? Math.max(ACTIVE_WINDOW_MS, interval * 60_000 + ACTIVE_GRACE_MS) : ACTIVE_WINDOW_MS;
  return now.getTime() - lastEnd <= window ? "active" : "offline";
}

/**
 * Map one roster member to the app's User, with today's figures derived from
 * their real rows (never authored). "Today" is the calendar day of
 * `context.timeZone` (UTC when none is given). Presence comes from the agents'
 * heartbeats when known, else from how recently a block of theirs ended.
 */
export function mapTeamMember(dto: ApiTeamMemberDto, activities: Activity[], now: Date, context: TeamMemberContext = {}): User {
  const zone = context.timeZone;
  const today = zone ? zonedDayKey(now, zone) : now.toISOString().slice(0, 10);
  const mine = activities.filter((a) => a.userId === dto.userId);
  const todays = zone ? activitiesOnZonedDay(mine, today, zone) : activitiesOnDay(mine, today);
  const role = memberRole(dto);
  const status: UserStatus =
    dto.isActive === false ? "offline" : (context.presence?.get(String(dto.userId)) ?? statusFromRows(mine, now, context.intervals));
  // The member's own zone is their device's, as their newest block reported it (newer agents only).
  const deviceZone = [...mine].sort((a, b) => Date.parse(b.endedAt) - Date.parse(a.endedAt)).find((a) => a.timeZone)?.timeZone;
  return {
    id: String(dto.userId),
    name: displayName(dto),
    email: str(dto.email),
    role,
    designation: roleDesignation[role],
    status,
    timezone: deviceZone ?? "UTC",
    trackedToday: Math.round(totalMinutes(todays)),
    productivity: averageProductivity(todays),
    joinedAt: "",
  };
}

/* ----------------------------- Presence ----------------------------- */

const STATUS_RANK: Record<UserStatus, number> = { offline: 0, idle: 1, active: 2 };

/**
 * Each member's status from GET /api/app/presence/team (one row per device):
 * an online device that is tracking makes them "active"; an online device
 * that is paused, idle, on a break, locked or choosing a project makes them
 * "idle"; anything else — offline, signed out, stopped — "offline". A member
 * with several devices takes the liveliest. Malformed rows are skipped.
 */
export function presenceStatusByUser(list: unknown): Map<string, UserStatus> {
  const out = new Map<string, UserStatus>();
  for (const row of asItems<unknown>(list)) {
    if (!row || typeof row !== "object") continue;
    const rec = row as Record<string, unknown>;
    const userId = text(rec.userId);
    if (!userId) continue;
    const status = str(rec.status).trim().toLowerCase();
    const next: UserStatus =
      rec.online !== true || status === "signedout" || status === "stopped" ? "offline" : status === "tracking" ? "active" : "idle";
    const current = out.get(userId);
    if (!current || STATUS_RANK[next] > STATUS_RANK[current]) out.set(userId, next);
  }
  return out;
}

/* ----------------------------- Derived series ----------------------------- */

/**
 * The last `days` calendar days of a zone ending today (oldest first; UTC by
 * default): tracked minutes, and the minutes of them with keyboard or mouse
 * input (Activity %, measured blocks only — an unmeasured block adds tracked
 * time but no active time).
 */
export function deriveDailyTrend(
  activities: Activity[],
  now: Date,
  days = 7,
  timeZone = "UTC",
): { day: string; tracked: number; productive: number }[] {
  const out: { day: string; tracked: number; productive: number }[] = [];
  const today = zonedDayKey(now, timeZone);
  for (let i = days - 1; i >= 0; i--) {
    const key = addZonedDays(today, -i);
    const rows = activitiesOnZonedDay(activities, key, timeZone);
    const tracked = rows.reduce((sum, a) => sum + durationMinutes(a), 0);
    const weekday = new Date(`${key}T00:00:00.000Z`).getUTCDay();
    out.push({ day: DAY_NAMES[weekday], tracked: Math.round(tracked), productive: Math.round(activeMinutes(rows)) });
  }
  return out;
}

/**
 * Minutes tracked per hour of the current day in a zone (the browser's by
 * default). A session that spans an hour boundary is split across both hours.
 * The axis always covers 8:00–19:00 and widens to include any hour that has
 * work outside it.
 */
export function deriveHourly(activities: Activity[], now: Date, timeZone = browserTimeZone()): { hour: string; minutes: number }[] {
  const today = zonedDayKey(now, timeZone);
  const dayStart = startOfZonedDay(today, timeZone).getTime();
  const dayEnd = startOfZonedDay(addZonedDays(today, 1), timeZone).getTime();
  const buckets = Array<number>(24).fill(0);
  for (const a of activities) {
    const start = Math.max(Date.parse(a.startedAt), dayStart);
    const end = Math.min(Date.parse(a.endedAt), dayEnd);
    if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) continue;
    let cursor = start;
    while (cursor < end) {
      const local = zonedParts(cursor, timeZone);
      // The next local hour boundary, read off the zone's own clock (exact for +5:45-style offsets).
      const toNextHour = ((60 - local.minute) * 60 - local.second) * 1000 - (cursor % 1000);
      const hourEnd = Math.min(end, cursor + Math.max(1, toNextHour));
      buckets[local.hour] += (hourEnd - cursor) / 60_000;
      cursor = hourEnd;
    }
  }
  const used = buckets.map((m, h) => (m > 0 ? h : -1)).filter((h) => h >= 0);
  const first = Math.min(8, ...used);
  const last = Math.max(19, ...used);
  const out: { hour: string; minutes: number }[] = [];
  for (let h = first; h <= last; h++) out.push({ hour: `${h}:00`, minutes: Math.round(buckets[h]) });
  return out;
}

/**
 * Focused time by application (same crediting rule as the backend's app-usage
 * report), with distinct-member reach, over the rows booked inside `range`
 * when one is given. Apps are grouped by their canonical name, and named,
 * coloured and categorized through lib/apps — the same identity every other
 * screen and the backend use (contract §8); an app nobody has classified
 * reads as neutral.
 */
export function deriveAppCatalog(activities: Activity[], range?: { from: Date; to: Date }): AppUsage[] {
  const rows = range ? activities.filter((a) => inRange(a, range.from, range.to)) : activities;
  const minutes = new Map<string, number>();
  const people = new Map<string, Set<string>>();
  const names = new Map<string, string>();
  for (const a of rows) {
    for (const w of a.activeWindows) {
      const name = friendlyAppName(w.appName);
      const key = appKey(name);
      if (!names.has(key)) names.set(key, name);
      if (w.seconds > 0) minutes.set(key, (minutes.get(key) ?? 0) + w.seconds / 60);
      const set = people.get(key) ?? new Set<string>();
      set.add(a.userId);
      people.set(key, set);
    }
  }
  return [...people.keys()]
    .map((key) => {
      const app = names.get(key) ?? key;
      return {
        app,
        category: appCategory(app),
        minutes: Math.round(minutes.get(key) ?? 0),
        color: appColor(app),
        activeUsers: people.get(key)?.size ?? 0,
      };
    })
    .filter((a) => a.minutes > 0)
    .sort((a, b) => b.minutes - a.minutes || a.app.localeCompare(b.app));
}

/** The six most-used apps (for the dashboard chart). */
export function deriveTopApps(catalog: AppUsage[]): { app: string; minutes: number; color: string }[] {
  return catalog.slice(0, 6).map((a) => ({ app: a.app, minutes: a.minutes, color: a.color }));
}

/** What the session reads off one GET /api/app/reporting/summary. */
export interface SummaryTotals {
  /** Tracked minutes per project id. */
  projects: Map<string, number>;
  /** Tracked minutes and Activity % (null = nothing measured) per user id. */
  users: Map<string, { minutes: number; activity: number | null }>;
  totalMinutes: number;
  /** Null when nothing in the period was measured (or the server sent no figure). */
  activity: number | null;
}

function percentOrNull(value: unknown): number | null {
  if (value === null || value === undefined || value === "") return null;
  const n = typeof value === "number" ? value : Number(value);
  return Number.isFinite(n) ? Math.min(100, Math.max(0, Math.round(n))) : null;
}

/**
 * Read a reporting summary (ReportSummaryDto). Rows without an id are
 * skipped; anything that isn't a summary at all yields null, so the caller
 * falls back to its own rows.
 */
export function mapReportSummary(raw: unknown): SummaryTotals | null {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const rec = raw as Record<string, unknown>;
  if (!Array.isArray(rec.perUser) && !Array.isArray(rec.perProject)) return null;
  const projects = new Map<string, number>();
  for (const row of recordsOf(rec.perProject)) {
    const id = text(row.projectId);
    if (id) projects.set(id, (projects.get(id) ?? 0) + Math.max(0, num(row.trackedMinutes)));
  }
  const users = new Map<string, { minutes: number; activity: number | null }>();
  for (const row of recordsOf(rec.perUser)) {
    const id = text(row.userId);
    if (id) users.set(id, { minutes: Math.max(0, num(row.trackedMinutes)), activity: percentOrNull(row.averageProductivity) });
  }
  return {
    projects,
    users,
    totalMinutes: Math.max(0, num(rec.totalTrackedMinutes)),
    activity: percentOrNull(rec.averageProductivity),
  };
}

/** Server-side totals per project id (minutes), from /api/app/reporting/summary over each period. */
export interface ProjectTotals {
  week?: ReadonlyMap<string, number> | null;
  month?: ReadonlyMap<string, number> | null;
  window?: ReadonlyMap<string, number> | null;
}

/**
 * Fill each project's totals: the last 7 days, this calendar month (in
 * `timeZone`, UTC by default) and the session's whole loaded window. A period
 * the server already totalled (`totals`, from the reporting summary) is taken
 * from there — exact however many rows exist — and otherwise summed from the
 * rows the session holds.
 */
export function applyProjectTotals(
  projects: Project[],
  activities: Activity[],
  now: Date,
  options: { timeZone?: string; totals?: ProjectTotals } = {},
): void {
  const weekAgo = new Date(now.getTime() - 7 * DAY_MS);
  const zone = options.timeZone;
  const monthStart = zone
    ? startOfZonedDay(`${zonedDayKey(now, zone).slice(0, 8)}01`, zone)
    : new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 1));
  const pick = (map: ReadonlyMap<string, number> | null | undefined, id: string, fallback: () => number) =>
    Math.round(map ? (map.get(id) ?? 0) : fallback());
  for (const p of projects) {
    p.loggedThisWeek = pick(options.totals?.week, p.id, () => minutesForProject(activities, p.id, weekAgo, now));
    p.loggedThisMonth = pick(options.totals?.month, p.id, () => minutesForProject(activities, p.id, monthStart, now));
    p.loggedTotal = pick(options.totals?.window, p.id, () => minutesForProject(activities, p.id));
  }
}

/* ----------------------------- Notifications ----------------------------- */

/** GET /api/app/notification/my-notifications item (NotificationDto). */
export interface ApiNotificationDto {
  id: string;
  message?: string | null;
  isRead?: boolean | null;
  creationTime?: string | null;
}

/** Map real notifications (newest first). The backend sends a single message; its type is inferred. */
export function mapApiNotifications(list: unknown): AppNotification[] {
  if (!Array.isArray(list)) return [];
  return list
    .filter((n): n is ApiNotificationDto => !!n && typeof n === "object" && typeof (n as ApiNotificationDto).id === "string")
    .map((n) => {
      const message = str(n.message).trim() || "Notification";
      const type: NotificationType = /invit|added|joined|member/i.test(message)
        ? "member"
        : /report/i.test(message)
          ? "report"
          : /limit|fail|overdue|past due|warning/i.test(message)
            ? "alert"
            : "system";
      return {
        id: n.id,
        type,
        title: message,
        body: "",
        at: normalizeUtcIso(n.creationTime) ?? new Date(0).toISOString(),
        read: n.isRead === true,
      };
    })
    .sort((a, b) => Date.parse(b.at) - Date.parse(a.at));
}

/**
 * The rows of a list response. `getApi` already unwraps ABP's `{ items }`
 * envelope, so callers get an array — reading `.items` again yields undefined
 * and silently empties the page. Accepts either shape; anything else is [].
 */
export function asItems<T = unknown>(res: unknown): T[] {
  if (Array.isArray(res)) return res as T[];
  const items = (res as { items?: unknown } | null)?.items;
  return Array.isArray(items) ? (items as T[]) : [];
}
