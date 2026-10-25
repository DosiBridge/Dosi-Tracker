import type { Activity, User, UserStatus } from "./types";
import { DAY_END_MIN, DAY_START_MIN, type AppMinutes, type DaySegment } from "./monitor-data";
import { appCategory, appColor, appKey, canonicalAppName } from "./apps";
import {
  activitiesRoute,
  isPresenceOnline,
  type AgentPlatform,
  type PresenceDto,
  type PresenceStatus,
} from "./api-contract";
import { addZonedDays, isDayKey, startOfZonedDay, zonedParts } from "./time-zone";
import { blockSeconds, groupWindowsByApp, type SessionDetails } from "./activity-report";

/* ============================================================================
 * LIVE MEMBER MONITOR — real activity rows and presence → the monitor's model.
 *
 * In a live session /monitor draws ONLY what the desktop agents reported: each
 * real activity block becomes one "work" segment of a calendar day in the
 * reporting zone. Nothing is generated — no meetings, breaks, idle gaps,
 * browser tabs or screens — and a member without rows has an empty day.
 *
 * Who is working NOW comes from the agents' heartbeats (GET
 * /api/app/presence/team, contract §3). An older server without that endpoint
 * falls back to the uploads: a member whose latest block ended recently reads
 * as tracking. Everything here is a pure function (the clock is a parameter),
 * so each rule is unit-tested without a backend.
 * ========================================================================== */

const MINUTES_PER_DAY = 24 * 60;
const MS_PER_MIN = 60_000;

/** The page size of a day request (the backend's page-size ceiling). */
export const DAY_ACTIVITY_LIMIT = 1000;

/** The most blocks the monitor reads for one day; more is reported as partial, never silently cut. */
export const DAY_ACTIVITY_MAX = 5000;

/** Without heartbeats, a member reads as tracking when one of their blocks ended at most this long ago… */
export const ACTIVE_WITHIN_MIN = 15;

/** …or two of their project's intervals, when that is longer (a 30-minute project uploads every 30 minutes). */
export function presenceWindowMinutes(intervalMinutes?: number | null): number {
  const interval = typeof intervalMinutes === "number" && Number.isFinite(intervalMinutes) ? intervalMinutes : 0;
  return Math.max(ACTIVE_WITHIN_MIN, 2 * interval);
}

/** The minutes-of-day span a timeline axis covers. */
export interface DayWindow {
  start: number;
  end: number;
}

/** The working-day axis (08:00–20:00) every demo day fits in. */
export const DEFAULT_DAY_WINDOW: DayWindow = { start: DAY_START_MIN, end: DAY_END_MIN };

/** The instants a calendar day of `timeZone` spans, [from, to); null for a malformed day key. */
export function dayBounds(dayKey: string, timeZone = "UTC"): { from: Date; to: Date } | null {
  if (!isDayKey(dayKey)) return null;
  return { from: startOfZonedDay(dayKey, timeZone), to: startOfZonedDay(addZonedDays(dayKey, 1), timeZone) };
}

/**
 * GET /api/app/activity for one calendar day of `timeZone`. The backend keeps
 * rows whose start is in [From, To) and scopes members without "view all" to
 * their own rows; a `userId` narrows a manager's request to one member. Pages
 * of DAY_ACTIVITY_LIMIT rows.
 */
export function dayActivityEndpoint(dayKey: string, userId?: string, timeZone = "UTC"): string {
  const bounds = dayBounds(dayKey, timeZone);
  return activitiesRoute({
    from: bounds?.from ?? null,
    to: bounds?.to ?? null,
    userId,
    maxResultCount: DAY_ACTIVITY_LIMIT,
  });
}

/** Minutes after local midnight of an instant in `timeZone` (fractional). */
function wallMinutes(t: number, timeZone: string): number {
  const p = zonedParts(t, timeZone);
  return p.hour * 60 + p.minute + p.second / 60;
}

/**
 * One real activity as a segment of the calendar day `dayKey` of `timeZone`
 * (minutes from local midnight, clipped to that day). Null when the row has
 * unusable times or does not overlap the day. The segment is named after the
 * app with the most focused time (canonical name, summed over its windows);
 * `details` carries what newer agents report (note, idle, end reason).
 */
export function activityToSegment(
  activity: Activity,
  dayKey: string,
  timeZone = "UTC",
  details?: SessionDetails,
): DaySegment | null {
  const bounds = dayBounds(dayKey, timeZone);
  const start = Date.parse(activity.startedAt);
  const end = Date.parse(activity.endedAt);
  if (!bounds || !Number.isFinite(start) || !Number.isFinite(end) || !(end > start)) return null;
  const dayStart = bounds.from.getTime();
  const dayEnd = bounds.to.getTime();
  if (end <= dayStart || start >= dayEnd) return null;

  // The time it really took on this day; the wall clock only places it on the axis (they differ
  // on the nights the clock changes).
  const minutes = Math.round((Math.min(end, dayEnd) - Math.max(start, dayStart)) / MS_PER_MIN);
  const startMin = start <= dayStart ? 0 : Math.round(wallMinutes(start, timeZone));
  let endMin = end >= dayEnd ? MINUTES_PER_DAY : Math.round(wallMinutes(end, timeZone));
  // The clock fell back an hour inside the block: draw it at its real length.
  if (endMin <= startMin) endMin = Math.min(MINUTES_PER_DAY, startMin + minutes);
  if (!(endMin > startMin) || minutes <= 0) return null;

  const top = groupWindowsByApp(Array.isArray(activity.activeWindows) ? activity.activeWindows : [])[0];
  const app = top?.displayName || "Tracked activity";
  const title = top?.windows.find((w) => w.windowTitle.trim())?.windowTitle;
  const seconds = blockSeconds(activity);
  const segment: DaySegment = {
    id: activity.id,
    type: "work",
    startMin,
    endMin,
    minutes,
    app,
    windowTitle: title || details?.note || activity.description || "Activity block",
    projectId: activity.projectId || null,
    productivity: Math.min(100, Math.max(0, Math.round(activity.productivity) || 0)),
    mouseClicks: activity.mouseClicks,
    keyboardHits: activity.keyboardHits,
    // A neutral descriptor only: live views draw an app glyph, never a synthetic screen.
    screen: { app, kind: "docs", accent: appColor(app) },
  };
  if (details) {
    segment.measured = details.measured;
    segment.note = details.note;
    segment.endReason = details.endReason;
    if (details.idleSeconds !== null && seconds > 0) {
      // A block clipped to the day keeps its share of the idle time.
      segment.idleMinutes = Math.round(((Math.min(details.idleSeconds, seconds) / 60) * minutes * 60) / seconds);
    }
  }
  return segment;
}

/** Each member's segments for the day `dayKey` of `timeZone`, keyed by user id, in time order. */
export function segmentsByUser(
  activities: Activity[],
  dayKey: string,
  timeZone = "UTC",
  detailsOf?: (activity: Activity) => SessionDetails | undefined,
): Map<string, DaySegment[]> {
  const byUser = new Map<string, DaySegment[]>();
  for (const activity of activities) {
    const seg = activityToSegment(activity, dayKey, timeZone, detailsOf?.(activity));
    if (!seg) continue;
    const list = byUser.get(activity.userId) ?? [];
    list.push(seg);
    byUser.set(activity.userId, list);
  }
  for (const list of byUser.values()) list.sort((a, b) => a.startMin - b.startMin);
  return byUser;
}

/**
 * The apps a set of blocks used during the day `dayKey` of `timeZone`, from
 * the focused seconds the agents reported per window — not the block's top
 * app, so 6 min of Visual Studio Code and 4 of Slack in a 10-minute block
 * count as exactly that. Reported seconds never exceed the block (they are
 * scaled down when they do), and a block crossing midnight credits the day
 * with its share. A legacy row's one credited window counts the whole block.
 */
export function liveAppBreakdown(activities: Activity[], dayKey: string, timeZone = "UTC"): AppMinutes[] {
  const bounds = dayBounds(dayKey, timeZone);
  if (!bounds) return [];
  const dayStart = bounds.from.getTime();
  const dayEnd = bounds.to.getTime();
  const byApp = new Map<string, { app: string; seconds: number }>();
  for (const a of activities) {
    const start = Date.parse(a.startedAt);
    const end = Date.parse(a.endedAt);
    if (!Number.isFinite(start) || !Number.isFinite(end) || !(end > start)) continue;
    const overlap = Math.min(end, dayEnd) - Math.max(start, dayStart);
    if (overlap <= 0) continue;
    const length = (end - start) / 1000;
    const groups = groupWindowsByApp(Array.isArray(a.activeWindows) ? a.activeWindows : []);
    const reported = groups.reduce((s, g) => s + Math.max(0, g.seconds), 0);
    if (reported <= 0) continue;
    const scale = Math.min(1, length / reported) * (overlap / 1000 / length);
    for (const g of groups) {
      const key = appKey(g.appName);
      const cur = byApp.get(key) ?? { app: g.displayName, seconds: 0 };
      cur.seconds += Math.max(0, g.seconds) * scale;
      byApp.set(key, cur);
    }
  }
  return [...byApp.values()]
    .map(({ app, seconds }) => ({ app, minutes: Math.round(seconds / 60), accent: appColor(app), category: appCategory(app) }))
    .filter((r) => r.minutes > 0)
    .sort((a, b) => b.minutes - a.minutes || a.app.localeCompare(b.app));
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

/** Each member's latest block (by end), skipping rows with an unusable end. */
export function latestBlockByUser(activities: Activity[]): Map<string, Activity> {
  const out = new Map<string, Activity>();
  for (const activity of activities) {
    const end = Date.parse(activity.endedAt);
    if (!Number.isFinite(end)) continue;
    const current = out.get(activity.userId);
    if (!current || end > Date.parse(current.endedAt)) out.set(activity.userId, activity);
  }
  return out;
}

/** "active" when the member's latest block ended within `windowMinutes` of `now`, else "offline". */
export function liveStatus(lastSeen: number | undefined, now: number, windowMinutes = ACTIVE_WITHIN_MIN): UserStatus {
  if (lastSeen === undefined) return "offline";
  return now - lastSeen <= windowMinutes * MS_PER_MIN ? "active" : "offline";
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
 * include. Each status comes from `statusOf` (presence) when given, else from
 * the member's latest block — never authored.
 */
export function liveRoster(
  members: User[],
  rowOwners: Iterable<string>,
  lastSeen: Map<string, number>,
  now: number,
  statusOf?: (userId: string) => UserStatus,
): User[] {
  const roster = members.filter((u) => !!u && u.role !== "client" && u.role !== "host");
  const listed = new Set(roster.map((u) => u.id));
  for (const id of rowOwners) {
    if (listed.has(id)) continue;
    listed.add(id);
    roster.push(unknownMember(id));
  }
  return roster.map((u) => ({ ...u, status: statusOf ? statusOf(u.id) : liveStatus(lastSeen.get(u.id), now) }));
}

/** Snap interval used for the timeline axis boundaries (60 min = whole hours). */
const AXIS_SNAP_HOUR = 60;

/**
 * The axis that shows every segment: derived from the actual activity range
 * (with one-hour padding on each side, snapped to whole hours).
 * Falls back to the default working day when there are no segments.
 * Never expands beyond midnight–midnight.
 */
export function timelineWindow(segments: DaySegment[]): DayWindow {
  if (segments.length === 0) return DEFAULT_DAY_WINDOW;
  let start = Infinity;
  let end = -Infinity;
  for (const s of segments) {
    start = Math.min(start, s.startMin);
    end = Math.max(end, s.endMin);
  }
  // Add one hour of padding and snap to whole hours so axis ticks land cleanly.
  return {
    start: Math.max(0, Math.floor((start - 60) / AXIS_SNAP_HOUR) * AXIS_SNAP_HOUR),
    end: Math.min(MINUTES_PER_DAY, Math.ceil((end + 60) / AXIS_SNAP_HOUR) * AXIS_SNAP_HOUR),
  };
}

/* ----------------------------- Presence ----------------------------- */

/** What a member is doing right now, as the monitor shows it. */
export type PresenceState = "tracking" | "idle" | "paused" | "break" | "locked" | "choosing" | "offline";

export const PRESENCE_STATE_LABELS: Record<PresenceState, string> = {
  tracking: "Tracking",
  idle: "Idle",
  paused: "Paused",
  break: "On a break",
  locked: "Screen locked",
  choosing: "Choosing a project",
  offline: "Offline",
};

/** Online but not tracking: the agent is running and the member stepped away (or paused). */
export const AWAY_STATES: readonly PresenceState[] = ["idle", "paused", "break", "locked", "choosing"];

/** The filter groups the monitor offers. */
export type PresenceGroup = "tracking" | "away" | "offline";

export function presenceGroup(state: PresenceState): PresenceGroup {
  if (state === "tracking") return "tracking";
  return state === "offline" ? "offline" : "away";
}

/** The avatar dot for a state: tracking pulses green, away is amber, offline grey. */
export function presenceUserStatus(state: PresenceState): UserStatus {
  const group = presenceGroup(state);
  return group === "tracking" ? "active" : group === "away" ? "idle" : "offline";
}

/** One member's presence right now, merged over their devices. */
export interface MemberPresence {
  userId: string;
  state: PresenceState;
  /** When the current state began (ISO), when known. */
  since: string | null;
  /** The last sign of life (ISO): the latest heartbeat, or the latest upload without heartbeats. */
  lastSeenAt: string | null;
  /** The app in front (canonical name) — only while online, and only when the project allows it. */
  currentApp: string | null;
  projectId: string | null;
  projectTitle: string | null;
  deviceName: string | null;
  platform: AgentPlatform | null;
  agentVersion: string | null;
  /** Where the state came from: agent heartbeats, recent uploads (an older server), or the demo roster. */
  source: "heartbeat" | "uploads" | "demo";
}

const STATUS_STATES: Record<PresenceStatus, PresenceState> = {
  tracking: "tracking",
  idle: "idle",
  paused: "paused",
  break: "break",
  locked: "locked",
  choosing: "choosing",
  signedout: "offline",
  stopped: "offline",
};

/** A device's state at `now`: offline once its last heartbeat is too old (aged locally between refreshes). */
export function presenceStateOf(device: Pick<PresenceDto, "status" | "online" | "lastSeenAt">, now: Date): PresenceState {
  if (device.online !== true || !isPresenceOnline(device.lastSeenAt, now)) return "offline";
  return Object.prototype.hasOwnProperty.call(STATUS_STATES, device.status) ? STATUS_STATES[device.status] : "offline";
}

/** Tracking first, then the away states, then offline. */
function stateRank(state: PresenceState): number {
  const group = presenceGroup(state);
  return group === "tracking" ? 0 : group === "away" ? 1 : 2;
}

function isoOrNull(value: unknown): string | null {
  if (typeof value !== "string" || value.trim() === "") return null;
  const raw = value.trim();
  const ms = Date.parse(/^\d{4}-\d{2}-\d{2}T[\d:.]+$/.test(raw) ? `${raw}Z` : raw);
  return Number.isFinite(ms) ? new Date(ms).toISOString() : null;
}

function str(value: unknown): string | null {
  return typeof value === "string" && value.trim() !== "" ? value.trim() : null;
}

/**
 * GET /api/app/presence/team (one entry per member device) → one presence per
 * member: the device that is tracking wins, then one that is online, then the
 * one seen last. Malformed entries are skipped.
 */
export function mergePresence(devices: unknown, now: Date): Map<string, MemberPresence> {
  const best = new Map<string, MemberPresence>();
  /** When the chosen device was last seen, per member. */
  const chosenSeen = new Map<string, number>();
  /** The latest heartbeat across all of a member's devices. */
  const latestSeen = new Map<string, number>();
  for (const raw of Array.isArray(devices) ? devices : []) {
    if (!raw || typeof raw !== "object") continue;
    const d = raw as PresenceDto;
    if (typeof d.userId !== "string" || !d.userId) continue;
    const lastSeenAt = isoOrNull(d.lastSeenAt);
    const seen = lastSeenAt ? Date.parse(lastSeenAt) : Number.NEGATIVE_INFINITY;
    const state = presenceStateOf({ status: d.status, online: d.online, lastSeenAt: lastSeenAt ?? "" }, now);
    const candidate: MemberPresence = {
      userId: d.userId,
      state,
      since: state === "offline" ? null : isoOrNull(d.statusSince),
      lastSeenAt,
      currentApp: state !== "offline" ? canonicalAppName(d.currentApp) || null : null,
      projectId: str(d.projectId),
      projectTitle: str(d.projectTitle),
      deviceName: str(d.deviceName),
      platform: d.platform === "windows" || d.platform === "macos" ? d.platform : null,
      agentVersion: str(d.agentVersion),
      source: "heartbeat",
    };
    if (seen > (latestSeen.get(d.userId) ?? Number.NEGATIVE_INFINITY)) latestSeen.set(d.userId, seen);
    const current = best.get(d.userId);
    const rank = stateRank(state);
    if (
      !current ||
      rank < stateRank(current.state) ||
      (rank === stateRank(current.state) && seen > (chosenSeen.get(d.userId) ?? Number.NEGATIVE_INFINITY))
    ) {
      best.set(d.userId, candidate);
      chosenSeen.set(d.userId, seen);
    }
  }
  // Offline on every device: "last seen" is the most recent sign of life on any of them.
  for (const presence of best.values()) {
    const seen = latestSeen.get(presence.userId);
    if (presence.state === "offline" && seen !== undefined && Number.isFinite(seen)) {
      presence.lastSeenAt = new Date(seen).toISOString();
    }
  }
  return best;
}

/**
 * Presence from the uploads alone (an older server without heartbeats): a
 * member is tracking while their latest block ended within the presence
 * window of its project (see presenceWindowMinutes), else offline.
 */
export function uploadPresence(
  latest: Map<string, Activity>,
  now: Date,
  intervalOf: (projectId: string) => number | undefined,
): Map<string, MemberPresence> {
  const out = new Map<string, MemberPresence>();
  for (const [userId, block] of latest) {
    const end = Date.parse(block.endedAt);
    const window = presenceWindowMinutes(intervalOf(block.projectId));
    const tracking = liveStatus(end, now.getTime(), window) === "active";
    out.set(userId, {
      userId,
      state: tracking ? "tracking" : "offline",
      since: null,
      lastSeenAt: new Date(end).toISOString(),
      currentApp: null,
      projectId: block.projectId || null,
      projectTitle: null,
      deviceName: null,
      platform: null,
      agentVersion: null,
      source: "uploads",
    });
  }
  return out;
}

/**
 * The demo workspace's presence, from its seeded roster: an "active" member is
 * tracking (in the app of their latest block), an "idle" one is idle, the rest
 * are offline. Never used for a live session.
 */
export function demoPresence(users: User[], latest: Map<string, Activity>): Map<string, MemberPresence> {
  const out = new Map<string, MemberPresence>();
  for (const u of users) {
    const block = latest.get(u.id);
    const state: PresenceState = u.status === "active" ? "tracking" : u.status === "idle" ? "idle" : "offline";
    out.set(u.id, {
      userId: u.id,
      state,
      since: null,
      lastSeenAt: block ? block.endedAt : null,
      currentApp: state === "tracking" && block ? canonicalAppName(block.screen.app) || null : null,
      projectId: block?.projectId ?? null,
      projectTitle: null,
      deviceName: null,
      platform: null,
      agentVersion: null,
      source: "demo",
    });
  }
  return out;
}

/** How long after a block ends the next one may still be running: one interval (a block never exceeds it) plus a grace for the upload. */
const LIVE_GRACE_MIN = 5;

/**
 * The blocks that belong to a session still running: for each member who is
 * tracking right now, their latest block — when it ended no longer than one
 * project interval (plus a grace) ago, so yesterday's last block never reads
 * as live because the member started again this morning.
 */
export function liveBlockIds(
  activities: Activity[],
  presence: Map<string, Pick<MemberPresence, "state">>,
  now: Date,
  intervalOf: (projectId: string) => number | undefined,
): Set<string> {
  const ids = new Set<string>();
  for (const [userId, block] of latestBlockByUser(activities)) {
    if (presence.get(userId)?.state !== "tracking") continue;
    const interval = intervalOf(block.projectId) ?? 10;
    if (now.getTime() - Date.parse(block.endedAt) <= (interval + LIVE_GRACE_MIN) * MS_PER_MIN) ids.add(block.id);
  }
  return ids;
}
