import type { Activity, Project, Role, ScreenMock, User, WindowInfo } from "./types";
import type { AppNotification, AppUsage, NotificationType } from "./reports-data";
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
  activeWindowsJson?: unknown;
  runningProgramsJson?: unknown;
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

/**
 * Parse an agent window list. The backend stores whatever the desktop agent
 * sent as a JSON string (camelCase from the agents, PascalCase when re-serialized
 * by .NET), so this accepts a JSON string OR an already-parsed array, either
 * casing, and silently drops anything malformed. Corrupt JSON yields [] — it
 * must never crash a page.
 */
export function parseWindowList(raw: unknown): WindowInfo[] {
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
  const out: WindowInfo[] = [];
  for (const item of value) {
    if (!item || typeof item !== "object") continue;
    const rec = item as Record<string, unknown>;
    const appName = str(rec.appName ?? rec.AppName ?? rec.app_name).trim();
    if (!appName) continue;
    out.push({
      appName,
      windowTitle: str(rec.windowTitle ?? rec.WindowTitle ?? rec.window_title),
      seconds: Math.max(0, num(rec.seconds ?? rec.Seconds, 0)),
    });
  }
  return out;
}

/** A neutral, honest stand-in for the screenshot tile of a real activity. */
function screenFor(active: WindowInfo[]): ScreenMock {
  const app = active[0]?.appName ?? "Desktop";
  return { app, kind: "editor", accent: colorFromString(app, 55, 45) };
}

/**
 * Map one backend activity. Rows without a usable id/time range are dropped
 * (null) rather than rendered as NaN. The agents do not report per-window
 * seconds, so — exactly like the backend's app-usage report — the focused app
 * (first active window) is credited with the block's whole duration.
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
    active[0] = { ...active[0], seconds: blockSeconds };
  }

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
    hasWebcam: false,
    online: false,
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
