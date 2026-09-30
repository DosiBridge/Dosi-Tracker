import type { Activity, ActivityMinute, CaptureKind, CaptureRef, Project, Role, ScreenMock, User, WindowInfo } from "./types";
import type { AppNotification, AppUsage, NotificationType } from "./reports-data";
import { mainAppOf } from "./activity-report";
import { activitiesOnDay, averageProductivity, durationMinutes, minutesForProject, minutesOnDay } from "./metrics";
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
  productivity?: number | null;
  mouseClicks?: number | null;
  keyboardHits?: number | null;
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
  /** The block's stored captures `[{ id, kind: "screen" | "webcam" | "thumb" }]`; absent on older backends. */
  captures?: unknown;
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

/**
 * Parse a block's per-minute timeline (`timelineJson`), any key casing, sorted
 * by minute. Buckets without a valid minute index are dropped, a duplicated
 * minute keeps its last bucket, and a missing `Active` flag is inferred from
 * the counts. Missing, "[]" or corrupt input → [].
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
    byMinute.set(minute, bucket);
  }
  return [...byMinute.values()].sort((a, b) => a.minute - b.minute);
}

const CAPTURE_KINDS: readonly CaptureKind[] = ["screen", "webcam", "thumb"];

/**
 * Parse a block's capture refs. Entries need an id and a known kind; anything
 * else — including the property older backends never send — yields [].
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
    out.push({ id, kind });
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

/**
 * Map one backend activity. Rows without a usable id/time range are dropped
 * (null) rather than rendered as NaN. Newer agents report focused seconds per
 * window; a legacy row reports none, so — exactly like the backend's app-usage
 * report — its focused app (first active window) is credited with the block's
 * whole duration and flagged `credited`.
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

  return {
    id: String(dto.id),
    userId: String(dto.userId ?? ""),
    projectId: String(dto.projectId ?? ""),
    startedAt,
    endedAt,
    description: str(dto.description).trim() || "Tracked session",
    productivity: Math.min(100, Math.max(0, Math.round(num(dto.productivity)))),
    mouseClicks: Math.max(0, Math.round(num(dto.mouseClicks))),
    keyboardHits: Math.max(0, Math.round(num(dto.keyboardHits))),
    activeWindows: active,
    runningPrograms: running,
    screen: screenFor(active),
    hasWebcam: captures.some((c) => c.kind === "webcam"),
    online: false,
    timeline: parseTimeline(dto.timelineJson),
    captures,
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
  };
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

/** A member is "active" when a block of theirs ended within this window. */
const ACTIVE_WINDOW_MS = 15 * 60 * 1000;

/**
 * Map one roster member to the app's User, with today's figures derived from
 * their real rows (never authored). `today` is a UTC day key.
 */
export function mapTeamMember(dto: ApiTeamMemberDto, activities: Activity[], now: Date): User {
  const today = now.toISOString().slice(0, 10);
  const mine = activities.filter((a) => a.userId === dto.userId);
  const todays = activitiesOnDay(mine, today);
  const lastEnd = mine.reduce((max, a) => Math.max(max, Date.parse(a.endedAt)), 0);
  const role = memberRole(dto);
  return {
    id: String(dto.userId),
    name: displayName(dto),
    email: str(dto.email),
    role,
    designation: roleDesignation[role],
    status:
      dto.isActive === false
        ? "offline"
        : lastEnd > 0 && now.getTime() - lastEnd <= ACTIVE_WINDOW_MS
          ? "active"
          : "offline",
    timezone: "UTC",
    trackedToday: Math.round(minutesOnDay(todays, today)),
    productivity: averageProductivity(todays),
    joinedAt: "",
  };
}

/* ----------------------------- Derived series ----------------------------- */

/**
 * The last `days` UTC days ending today (oldest first): tracked minutes, and
 * the productive share of them (duration × productivity).
 */
export function deriveDailyTrend(
  activities: Activity[],
  now: Date,
  days = 7,
): { day: string; tracked: number; productive: number }[] {
  const out: { day: string; tracked: number; productive: number }[] = [];
  for (let i = days - 1; i >= 0; i--) {
    const d = new Date(now.getTime() - i * DAY_MS);
    const key = d.toISOString().slice(0, 10);
    const rows = activitiesOnDay(activities, key);
    let tracked = 0;
    let productive = 0;
    for (const a of rows) {
      const mins = durationMinutes(a);
      tracked += mins;
      productive += (mins * a.productivity) / 100;
    }
    out.push({ day: DAY_NAMES[d.getUTCDay()], tracked: Math.round(tracked), productive: Math.round(productive) });
  }
  return out;
}

/**
 * Minutes tracked per local hour of the viewer's current day. A session that
 * spans an hour boundary is split across both hours. The axis always covers
 * 8:00–19:00 and widens to include any hour that has work outside it.
 */
export function deriveHourly(activities: Activity[], now: Date): { hour: string; minutes: number }[] {
  const dayStart = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const dayEnd = dayStart + DAY_MS;
  const buckets = Array<number>(24).fill(0);
  for (const a of activities) {
    const start = Math.max(Date.parse(a.startedAt), dayStart);
    const end = Math.min(Date.parse(a.endedAt), dayEnd);
    if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) continue;
    let cursor = start;
    while (cursor < end) {
      const hour = new Date(cursor).getHours();
      const hourEnd = Math.min(end, new Date(cursor).setMinutes(60, 0, 0));
      buckets[hour] += (hourEnd - cursor) / 60_000;
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
 * Focused-time-by-application across the rows (same crediting rule as the
 * backend's app-usage report), with distinct-member reach. Apps are not
 * categorized in live mode, so every app is reported as "neutral" rather than
 * guessing productive/unproductive.
 */
export function deriveAppCatalog(activities: Activity[]): AppUsage[] {
  const minutes = new Map<string, number>();
  const people = new Map<string, Set<string>>();
  for (const a of activities) {
    for (const w of a.activeWindows) {
      if (w.seconds > 0) minutes.set(w.appName, (minutes.get(w.appName) ?? 0) + w.seconds / 60);
      const set = people.get(w.appName) ?? new Set<string>();
      set.add(a.userId);
      people.set(w.appName, set);
    }
  }
  return [...people.keys()]
    .map((app) => ({
      app,
      category: "neutral" as const,
      minutes: Math.round(minutes.get(app) ?? 0),
      color: colorFromString(app, 60, 50),
      activeUsers: people.get(app)?.size ?? 0,
    }))
    .filter((a) => a.minutes > 0)
    .sort((a, b) => b.minutes - a.minutes || a.app.localeCompare(b.app));
}

/** The six most-used apps (for the dashboard chart). */
export function deriveTopApps(catalog: AppUsage[]): { app: string; minutes: number; color: string }[] {
  return catalog.slice(0, 6).map((a) => ({ app: a.app, minutes: a.minutes, color: a.color }));
}

/** Fill each project's week / month / loaded-window totals from the real rows. */
export function applyProjectTotals(projects: Project[], activities: Activity[], now: Date): void {
  const weekAgo = new Date(now.getTime() - 7 * DAY_MS);
  const monthStart = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 1));
  for (const p of projects) {
    p.loggedThisWeek = Math.round(minutesForProject(activities, p.id, weekAgo, now));
    p.loggedThisMonth = Math.round(minutesForProject(activities, p.id, monthStart, now));
    p.loggedTotal = Math.round(minutesForProject(activities, p.id));
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
