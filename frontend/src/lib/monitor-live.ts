import type { Activity, User, UserStatus, WindowInfo } from "./types";
import { DAY_END_MIN, DAY_START_MIN, type DaySegment } from "./monitor-data";
import { colorFromString } from "./utils";

/* ============================================================================
 * LIVE MEMBER MONITOR — real activity rows → the monitor's day model.
 *
 * In a live session /monitor draws ONLY what the desktop agents reported: each
 * real activity block becomes one "work" segment of its UTC day. Nothing is
 * generated — no meetings, breaks, idle gaps, browser tabs or screens — and a
 * member without rows has an empty day. Everything is a pure function (the
 * clock is a parameter) so each rule is unit-tested without a backend.
 * ========================================================================== */

const MINUTES_PER_DAY = 24 * 60;
const MS_PER_MIN = 60_000;
/** Axis ticks are drawn every two hours, so a widened axis snaps to them. */
const AXIS_SNAP_MIN = 120;

/** The most rows one day request asks for (the backend's page-size ceiling). */
export const DAY_ACTIVITY_LIMIT = 1000;

/** A member reads as "active" when one of their blocks ended at most this long ago. */
export const ACTIVE_WITHIN_MIN = 15;

/** The minutes-of-day span a timeline axis covers. */
export interface DayWindow {
  start: number;
  end: number;
}

/** The working-day axis (08:00–20:00) every demo day fits in. */
export const DEFAULT_DAY_WINDOW: DayWindow = { start: DAY_START_MIN, end: DAY_END_MIN };

/**
 * GET /api/app/activity for one UTC day. The backend keeps rows whose start is
 * in [From, To) and scopes members without "view all" to their own rows; a
 * `userId` narrows a manager's request to one member.
 */
export function dayActivityEndpoint(iso: string, userId?: string): string {
  const from = `${iso}T00:00:00.000Z`;
  const to = new Date(Date.parse(from) + MINUTES_PER_DAY * MS_PER_MIN).toISOString();
  const member = userId ? `&UserId=${encodeURIComponent(userId)}` : "";
  return `/api/app/activity?From=${encodeURIComponent(from)}&To=${encodeURIComponent(to)}${member}&MaxResultCount=${DAY_ACTIVITY_LIMIT}`;
}

const clampToDay = (min: number) => Math.min(Math.max(min, 0), MINUTES_PER_DAY);

/**
 * One real activity as a segment of the UTC day `iso` (minutes from midnight,
 * clipped to that day). Null when the row has unusable times or does not
 * overlap the day. The focused app is the window with the most seconds.
 */
export function activityToSegment(activity: Activity, iso: string): DaySegment | null {
  const dayStart = Date.parse(`${iso}T00:00:00Z`);
  const startMin = clampToDay(Math.round((Date.parse(activity.startedAt) - dayStart) / MS_PER_MIN));
  const endMin = clampToDay(Math.round((Date.parse(activity.endedAt) - dayStart) / MS_PER_MIN));
  // Written so NaN (a bad day key or timestamp) also lands here.
  if (!(endMin > startMin)) return null;

  const windows: WindowInfo[] = Array.isArray(activity.activeWindows) ? activity.activeWindows : [];
  const focused = windows.reduce<WindowInfo | undefined>(
    (best, w) => (best === undefined || w.seconds > best.seconds ? w : best),
    undefined,
  );
  const app = focused?.appName || "Tracked activity";

  return {
    id: activity.id,
    type: "work",
    startMin,
    endMin,
    minutes: endMin - startMin,
    app,
    windowTitle: focused?.windowTitle || activity.description || "Activity block",
    projectId: activity.projectId || null,
    productivity: Math.min(100, Math.max(0, Math.round(activity.productivity) || 0)),
    mouseClicks: activity.mouseClicks,
    keyboardHits: activity.keyboardHits,
    // A neutral descriptor only: live views draw an app glyph, never a synthetic screen.
    screen: { app, kind: "docs", accent: colorFromString(app, 55, 45) },
  };
}

/** Each member's segments for the UTC day `iso`, keyed by user id, in time order. */
export function segmentsByUser(activities: Activity[], iso: string): Map<string, DaySegment[]> {
  const byUser = new Map<string, DaySegment[]>();
  for (const activity of activities) {
    const seg = activityToSegment(activity, iso);
    if (!seg) continue;
    const list = byUser.get(activity.userId) ?? [];
    list.push(seg);
    byUser.set(activity.userId, list);
  }
  for (const list of byUser.values()) list.sort((a, b) => a.startMin - b.startMin);
  return byUser;
}

/** The latest block end (epoch ms) per member; rows with an unusable end are ignored. */
export function lastSeenByUser(activities: Activity[]): Map<string, number> {
  const out = new Map<string, number>();
  for (const activity of activities) {
    const end = Date.parse(activity.endedAt);
    if (!Number.isFinite(end)) continue;
    out.set(activity.userId, Math.max(end, out.get(activity.userId) ?? end));
  }
  return out;
}

/** "active" when the member's latest block ended within ACTIVE_WITHIN_MIN of `now`, else "offline". */
export function liveStatus(lastSeen: number | undefined, now: number): UserStatus {
  if (lastSeen === undefined) return "offline";
  return now - lastSeen <= ACTIVE_WITHIN_MIN * MS_PER_MIN ? "active" : "offline";
}

/** A row owner who is missing from the loaded roster (e.g. the roster call failed). */
export function unknownMember(id: string): User {
  return {
    id,
    name: `Member ${id.slice(0, 8)}`,
    email: "",
    role: "worker",
    designation: "Not on the loaded roster",
    status: "offline",
    timezone: "UTC",
    trackedToday: 0,
    productivity: 0,
    joinedAt: "",
  };
}

/**
 * The members a live monitor lists: the real roster (clients and the platform
 * host are never monitored), plus any owner of a fetched row the roster did not
 * include. Presence is derived from real rows only — never authored.
 */
export function liveRoster(
  members: User[],
  rowOwners: Iterable<string>,
  lastSeen: Map<string, number>,
  now: number,
): User[] {
  const roster = members.filter((u) => !!u && u.role !== "client" && u.role !== "host");
  const listed = new Set(roster.map((u) => u.id));
  for (const id of rowOwners) {
    if (listed.has(id)) continue;
    listed.add(id);
    roster.push(unknownMember(id));
  }
  return roster.map((u) => ({ ...u, status: liveStatus(lastSeen.get(u.id), now) }));
}

/**
 * The axis that shows every segment: the working day, widened to whole
 * two-hour marks when real work falls outside it (agents report UTC, so a
 * member's day can sit anywhere in it), and never beyond midnight–midnight.
 */
export function timelineWindow(segments: DaySegment[]): DayWindow {
  let start = DEFAULT_DAY_WINDOW.start;
  let end = DEFAULT_DAY_WINDOW.end;
  for (const s of segments) {
    start = Math.min(start, s.startMin);
    end = Math.max(end, s.endMin);
  }
  return {
    start: Math.max(0, Math.floor(start / AXIS_SNAP_MIN) * AXIS_SNAP_MIN),
    end: Math.min(MINUTES_PER_DAY, Math.ceil(end / AXIS_SNAP_MIN) * AXIS_SNAP_MIN),
  };
}
