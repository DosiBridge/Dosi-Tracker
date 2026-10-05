import type { Activity, ActivityMinute, WindowInfo } from "./types";
import { appKey, canonicalAppName } from "./apps";
import {
  END_REASON_LABELS,
  endReasonLabel,
  platformLabel,
  type ActivityEndReason,
  type AgentPlatform,
} from "./api-contract";
import { addZonedDays, browserTimeZone, formatDayKey, formatInZone, zonedDayKey, zonedParts } from "./time-zone";

/* ============================================================================
 * ACTIVITY REPORT — pure derivations behind one block's detail view.
 *
 * The activity drawer turns a single tracked block into a report: which apps
 * and windows held focus (and the input while they did), and a per-minute
 * timeline. Newer agents send all of it; legacy rows carry only a credited
 * focus window and demo rows carry no timeline, so every derivation here
 * degrades to "nothing to show" rather than inventing numbers.
 *
 * Clock times and calendar days read in the REPORTING zone when one is given
 * (the zone every report buckets by), else in the viewer's browser zone.
 * ========================================================================== */

const MS_PER_MIN = 60_000;

const CLOCK_FORMAT: Intl.DateTimeFormatOptions = { hour: "2-digit", minute: "2-digit" };

/** Whole seconds between the block's start and end (0 for unusable times). */
export function blockSeconds(activity: Pick<Activity, "startedAt" | "endedAt">): number {
  const ms = Date.parse(activity.endedAt) - Date.parse(activity.startedAt);
  return Number.isFinite(ms) ? Math.max(0, Math.round(ms / 1000)) : 0;
}

/** "10:04 AM" in `timeZone` (default: the viewer's zone); "—" for a bad timestamp. */
export function formatClock(iso: string, timeZone?: string): string {
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return "—";
  return timeZone ? formatInZone(t, timeZone, CLOCK_FORMAT, "en-US") : new Date(t).toLocaleTimeString("en", CLOCK_FORMAT);
}

/** "10:04" — an axis tick; the AM/PM half is already stated by the block's range. */
export function formatClockShort(iso: string, timeZone?: string): string {
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return "—";
  const d = new Date(t);
  const { hour, minute } = timeZone ? zonedParts(t, timeZone) : { hour: d.getHours(), minute: d.getMinutes() };
  return `${hour % 12 || 12}:${String(minute).padStart(2, "0")}`;
}

/** "10:00 AM – 10:05 AM" in `timeZone` (default: the viewer's zone). */
export function formatTimeRange(activity: Pick<Activity, "startedAt" | "endedAt">, timeZone?: string): string {
  return `${formatClock(activity.startedAt, timeZone)} – ${formatClock(activity.endedAt, timeZone)}`;
}

/** "Wed, 30 Sep 2026" — the calendar day a block started, in `timeZone` (default: the viewer's zone). */
export function formatSessionDate(iso: string, timeZone?: string, options: { year?: boolean } = {}): string {
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return "—";
  return formatDayKey(zonedDayKey(t, timeZone ?? browserTimeZone()), { weekday: true, year: options.year });
}

/* ----------------------------- Apps & windows ----------------------------- */

/** One application's share of a block, with the windows it was focused in. */
export interface AppWindowGroup {
  /** the first spelling the agent reported ("chrome.exe") */
  appName: string;
  /** the app's canonical name for display ("Google Chrome"; lib/apps) */
  displayName: string;
  /** focused seconds summed over the app's windows */
  seconds: number;
  /** input while the app was focused; undefined when none of its windows reported it */
  keyboardHits?: number;
  mouseClicks?: number;
  /** true when the time is a legacy row's whole-block credit, not a measurement */
  credited: boolean;
  /** the app's windows, most-focused first */
  windows: WindowInfo[];
}

function sumReported(windows: WindowInfo[], pick: (w: WindowInfo) => number | undefined): number | undefined {
  let total: number | undefined;
  for (const w of windows) {
    const v = pick(w);
    if (v !== undefined) total = (total ?? 0) + v;
  }
  return total;
}

/**
 * Group a block's focus windows by application — by the app's canonical
 * identity, so "chrome.exe" and "Google Chrome" are one app (the first
 * spelling seen is kept as `appName`) — most-focused app first. Ties keep the
 * agent's order, which is already seconds-descending.
 */
export function groupWindowsByApp(windows: WindowInfo[]): AppWindowGroup[] {
  const groups = new Map<string, { order: number; windows: WindowInfo[] }>();
  for (const w of windows) {
    const key = appKey(w.appName);
    if (!key) continue;
    const group = groups.get(key) ?? { order: groups.size, windows: [] };
    group.windows.push(w);
    groups.set(key, group);
  }
  return [...groups.values()]
    .map(({ order, windows: list }) => ({
      order,
      group: {
        appName: list[0].appName.trim(),
        displayName: friendlyAppName(list[0].appName),
        seconds: list.reduce((s, w) => s + w.seconds, 0),
        keyboardHits: sumReported(list, (w) => w.keyboardHits),
        mouseClicks: sumReported(list, (w) => w.mouseClicks),
        credited: list.some((w) => w.credited === true),
        windows: list
          .map((w, i) => ({ w, i }))
          .sort((a, b) => b.w.seconds - a.w.seconds || a.i - b.i)
          .map(({ w }) => w),
      } satisfies AppWindowGroup,
    }))
    .sort((a, b) => b.group.seconds - a.group.seconds || a.order - b.order)
    .map(({ group }) => group);
}

/** The block's main app: the one with the most focused time (undefined when nothing was focused). */
export function mainAppOf(windows: WindowInfo[]): string | undefined {
  return groupWindowsByApp(windows)[0]?.appName;
}

/** A share of the block as a whole percent, clamped to 0–100 (0 for an empty block). */
export function shareOfBlock(seconds: number, total: number): number {
  if (!(total > 0) || !(seconds > 0)) return 0;
  return Math.min(100, Math.round((seconds / total) * 100));
}

/* ----------------------------- Timeline ----------------------------- */

/** One drawable minute of the timeline. */
export interface TimelineSlot extends ActivityMinute {
  /** when the minute started on the clock (ISO) */
  at: string;
  /** keyboard + mouse */
  total: number;
  /** the agent sent no bucket for this minute (drawn as idle, labelled "no data") */
  missing: boolean;
}

/**
 * Contiguous per-minute slots from minute 0 to the last reported bucket. A
 * minute the agent skipped becomes an idle "no data" slot so the axis keeps
 * real time. No timeline → [].
 */
export function timelineSlots(activity: Pick<Activity, "startedAt" | "timeline">): TimelineSlot[] {
  const buckets = Array.isArray(activity.timeline) ? activity.timeline : [];
  const start = Date.parse(activity.startedAt);
  if (buckets.length === 0 || !Number.isFinite(start)) return [];
  const byMinute = new Map(buckets.map((b) => [b.minute, b]));
  const last = Math.max(...buckets.map((b) => b.minute));
  const slots: TimelineSlot[] = [];
  for (let minute = 0; minute <= last; minute++) {
    const at = new Date(start + minute * MS_PER_MIN).toISOString();
    const b = byMinute.get(minute);
    slots.push(
      b
        ? { ...b, at, total: b.keyboardHits + b.mouseClicks, missing: false }
        : { minute, keyboardHits: 0, mouseClicks: 0, active: false, at, total: 0, missing: true },
    );
  }
  return slots;
}

export interface TimelineSummary {
  minutes: number;
  activeMinutes: number;
  keyboardHits: number;
  mouseClicks: number;
  /** the minute with the most input (undefined when there was none) */
  peak?: TimelineSlot;
}

export function summarizeTimeline(slots: TimelineSlot[]): TimelineSummary {
  let peak: TimelineSlot | undefined;
  for (const s of slots) if (s.total > 0 && (!peak || s.total > peak.total)) peak = s;
  return {
    minutes: slots.length,
    activeMinutes: slots.filter((s) => s.active).length,
    keyboardHits: slots.reduce((sum, s) => sum + s.keyboardHits, 0),
    mouseClicks: slots.reduce((sum, s) => sum + s.mouseClicks, 0),
    peak,
  };
}

/** Candidate x-axis label steps (minutes): the smallest that keeps ≤ `maxLabels` ticks wins. */
const TICK_STEPS = [1, 2, 5, 10, 15, 30, 60];

/** Every how-many minutes the timeline's axis carries a clock label. */
export function timelineTickStep(slotCount: number, maxLabels = 7): number {
  return TICK_STEPS.find((step) => Math.ceil(slotCount / step) <= maxLabels) ?? TICK_STEPS[TICK_STEPS.length - 1];
}

/* ----------------------------- Presentation ----------------------------- */

/**
 * A readable app name for display — the ONE canonical name every page uses
 * (lib/apps, mirrored by the backend's reports): "msedge.exe" → "Microsoft
 * Edge", "Code.exe" → "Visual Studio Code", anything else loses ".exe" and is
 * title-cased. Filters and search keep using the raw name; this is
 * presentation only. A blank name stays blank.
 */
export function friendlyAppName(appName: string): string {
  return canonicalAppName(appName) || appName.trim();
}

/** "45s", "10m", "1h 5m" — a block's length at a glance. */
export function formatBlockLength(seconds: number): string {
  if (!(seconds > 0)) return "0m";
  if (seconds < 60) return `${Math.round(seconds)}s`;
  const minutes = Math.round(seconds / 60);
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h === 0) return `${m}m`;
  return m === 0 ? `${h}h` : `${h}h ${m}m`;
}

/** The description a live block gets when the agent sent none. */
export const DEFAULT_SESSION_DESCRIPTION = "Tracked session";

/**
 * What a session was about, for its card: its description when someone wrote
 * one, else the main app's most-focused window title. Browsers append their own
 * name to every title ("Docs - Personal - Microsoft Edge"), which the card
 * already shows as the app, so that suffix is dropped.
 */
export function sessionHeadline(activity: Pick<Activity, "activeWindows" | "description">): string {
  const described = activity.description.trim();
  if (described && described !== DEFAULT_SESSION_DESCRIPTION) return described;
  const top = groupWindowsByApp(activity.activeWindows)[0];
  const title = top?.windows.find((w) => w.windowTitle.trim() && w.windowTitle.trim() !== "—")?.windowTitle.trim();
  if (!title) return described || DEFAULT_SESSION_DESCRIPTION;
  const app = friendlyAppName(top.appName);
  const suffix = new RegExp(`\\s*[-–—|]\\s*${app.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*$`, "i");
  return title.replace(/​/g, "").replace(suffix, "").trim() || title;
}

/** Whether a live block has a stored screen capture to show (full or thumbnail). */
export function hasScreenshot(activity: Pick<Activity, "captures">): boolean {
  return (activity.captures ?? []).some((c) => c.kind === "screen" || c.kind === "thumb");
}

/** One calendar day of sessions, in the order they were given. */
export interface SessionDay<T> {
  /** the day in the grouping zone, "YYYY-MM-DD" */
  key: string;
  /** "Today", "Yesterday", or e.g. "Wed, 30 Sep" */
  label: string;
  items: T[];
  /** tracked seconds across the day's blocks */
  seconds: number;
  /** duration-weighted average activity % over the blocks whose input was measured; null when none was */
  activity: number | null;
  /** idle seconds the agents reported for the day's blocks (0 when none reported any) */
  idleSeconds: number;
}

export interface GroupSessionsOptions<T> {
  /** The zone whose calendar days the sessions are grouped by (default: the viewer's zone). */
  timeZone?: string;
  /** Whether a block's activity % was measured (default: always); unmeasured blocks stay out of the average. */
  isMeasured?: (item: T) => boolean;
  /** The block's reported idle seconds, when known. */
  idleSecondsOf?: (item: T) => number | null | undefined;
}

/**
 * Group sessions by the day they started in the grouping zone, keeping the
 * given order (so a "most recent" list stays newest-first within and across
 * days). `now` sets what counts as today.
 */
export function groupSessionsByDay<T extends Pick<Activity, "startedAt" | "endedAt" | "productivity">>(
  items: T[],
  now: Date = new Date(),
  options: GroupSessionsOptions<T> = {},
): SessionDay<T>[] {
  const zone = options.timeZone ?? browserTimeZone();
  const today = zonedDayKey(now, zone);
  const yesterday = addZonedDays(today, -1);
  const days = new Map<string, SessionDay<T> & { weighted: number; measuredSeconds: number }>();
  for (const item of items) {
    const start = Date.parse(item.startedAt);
    if (!Number.isFinite(start)) continue;
    const key = zonedDayKey(start, zone);
    let day = days.get(key);
    if (!day) {
      const label = key === today ? "Today" : key === yesterday ? "Yesterday" : formatDayKey(key, { weekday: true });
      day = { key, label, items: [], seconds: 0, activity: null, idleSeconds: 0, weighted: 0, measuredSeconds: 0 };
      days.set(key, day);
    }
    const seconds = blockSeconds(item);
    day.items.push(item);
    day.seconds += seconds;
    if (options.isMeasured?.(item) ?? true) {
      day.measuredSeconds += seconds;
      day.weighted += seconds * item.productivity;
    }
    const idle = options.idleSecondsOf?.(item);
    if (typeof idle === "number" && idle > 0) day.idleSeconds += Math.min(idle, seconds);
  }
  return [...days.values()].map(({ weighted, measuredSeconds, ...day }) => ({
    ...day,
    activity: measuredSeconds > 0 ? Math.round(weighted / measuredSeconds) : null,
  }));
}

/* ----------------------------- Newer agents' block details ----------------------------- */

/**
 * What newer agents report about a block beyond the core row (contract §1,
 * §5). Every field is optional on the wire: a block from an older agent (or an
 * older server) reads as "not reported" (null), never as zero.
 */
export interface SessionDetails {
  /** The task note the member wrote ("What are you working on?"); null when there is none. */
  note: string | null;
  /** False when the agent couldn't measure input (activity % null: capture unavailable or not allowed). */
  measured: boolean;
  /** Seconds of the block without keyboard or mouse input; null when not reported. */
  idleSeconds: number | null;
  /** Why the block ended; null when unknown. */
  endReason: ActivityEndReason | null;
  platform: AgentPlatform | null;
  osVersion: string | null;
  agentVersion: string | null;
  deviceId: string | null;
  deviceName: string | null;
  /** The device's own zone and UTC offset when the block started. */
  timeZone: string | null;
  utcOffsetMinutes: number | null;
  /** Server minus device clock at upload (positive = the device clock was behind). */
  clockSkewSeconds: number | null;
  /** Software-injected input seen during the block (left out of the counts and activity %). */
  injectedInputEvents: number | null;
  /** The block's screen capture (the full one, else its thumbnail): when it was taken, and whether it was blurred. */
  capture: { capturedAt: string | null; blurred: boolean } | null;
}

/** A block with nothing reported beyond the core row (an older agent, or the demo). */
export const NO_SESSION_DETAILS: SessionDetails = {
  note: null,
  measured: true,
  idleSeconds: null,
  endReason: null,
  platform: null,
  osVersion: null,
  agentVersion: null,
  deviceId: null,
  deviceName: null,
  timeZone: null,
  utcOffsetMinutes: null,
  clockSkewSeconds: null,
  injectedInputEvents: null,
  capture: null,
};

function text(value: unknown): string | null {
  return typeof value === "string" && value.trim() !== "" ? value.trim() : null;
}

function finite(value: unknown): number | null {
  if (value === null || value === undefined || value === "") return null;
  const n = typeof value === "number" ? value : Number(value);
  return Number.isFinite(n) ? n : null;
}

/**
 * An ISO instant, or null. ABP may write a UTC DateTime without its zone
 * ("2026-09-23T08:00:00"); read that as UTC, never as the viewer's local time.
 */
function utcInstant(value: unknown): string | null {
  const raw = text(value);
  if (!raw) return null;
  const zoned = /([zZ]|[+-]\d{2}:?\d{2})$/.test(raw) || !/^\d{4}-\d{2}-\d{2}T/.test(raw) ? raw : `${raw}Z`;
  const ms = Date.parse(zoned);
  return Number.isFinite(ms) ? new Date(ms).toISOString() : null;
}

/**
 * Read a GET /api/app/activity row's details (ActivityDto, contract §5).
 * Anything malformed reads as not reported; nothing here throws.
 */
export function readSessionDetails(dto: unknown): SessionDetails {
  if (!dto || typeof dto !== "object") return { ...NO_SESSION_DETAILS, measured: false };
  const row = dto as Record<string, unknown>;
  const idle = finite(row.idleSeconds);
  const offset = finite(row.utcOffsetMinutes);
  const injected = finite(row.injectedInputEvents);
  const skew = finite(row.clockSkewSeconds);
  const reason = text(row.endReason)?.toLowerCase() ?? null;
  const platform = text(row.platform)?.toLowerCase() ?? null;
  const captures = Array.isArray(row.captures)
    ? row.captures.filter((c): c is Record<string, unknown> => !!c && typeof c === "object")
    : [];
  const kindOf = (c: Record<string, unknown>) => (typeof c.kind === "string" ? c.kind.toLowerCase() : "");
  const screen = captures.find((c) => kindOf(c) === "screen") ?? captures.find((c) => kindOf(c) === "thumb");
  return {
    note: text(row.description),
    // null = not measured (contract §1); an absent field is no measurement either.
    measured: finite(row.productivity) !== null,
    idleSeconds: idle !== null && idle >= 0 ? Math.round(idle) : null,
    endReason: endReasonLabel(reason) ? (reason as ActivityEndReason) : null,
    platform: platformLabel(platform) ? (platform as AgentPlatform) : null,
    osVersion: text(row.osVersion),
    agentVersion: text(row.agentVersion),
    deviceId: text(row.deviceId),
    deviceName: text(row.deviceName),
    timeZone: text(row.timeZone),
    utcOffsetMinutes: offset !== null && Math.abs(offset) <= 840 ? Math.round(offset) : null,
    clockSkewSeconds: skew !== null ? Math.round(skew) : null,
    injectedInputEvents: injected !== null && injected >= 0 ? Math.round(injected) : null,
    capture: screen ? { capturedAt: utcInstant(screen.capturedAt), blurred: screen.blurred === true } : null,
  };
}

/** Each listed row's details, by row id (rows without a string id are skipped). */
export function sessionDetailsById(rows: unknown): Map<string, SessionDetails> {
  const out = new Map<string, SessionDetails>();
  for (const row of Array.isArray(rows) ? rows : []) {
    const id = row && typeof row === "object" ? (row as { id?: unknown }).id : undefined;
    if (typeof id === "string" && id) out.set(id, readSessionDetails(row));
  }
  return out;
}

/**
 * A demo row's details, from what the demo data holds: its description is the
 * member's note, and idle time follows the contract's minute rule — the block's
 * minutes without input are the inactive share its activity % leaves.
 */
export function demoSessionDetails(activity: Pick<Activity, "startedAt" | "endedAt" | "productivity" | "description">): SessionDetails {
  const seconds = blockSeconds(activity);
  const minutes = Math.max(1, Math.round(seconds / 60));
  const inactive = Math.round((minutes * (100 - Math.min(100, Math.max(0, activity.productivity)))) / 100);
  return {
    ...NO_SESSION_DETAILS,
    note: text(activity.description) === DEFAULT_SESSION_DESCRIPTION ? null : text(activity.description),
    idleSeconds: Math.min(seconds, inactive * 60),
  };
}

/** End reasons worth a chip: everything but a block that simply reached its interval. */
export function notableEndReason(reason: ActivityEndReason | null | undefined): ActivityEndReason | null {
  return reason && reason !== "interval" && Object.prototype.hasOwnProperty.call(END_REASON_LABELS, reason) ? reason : null;
}

/** The chip tone of an end reason: idle stands out, pauses and breaks read as information, the rest stay neutral. */
export function endReasonTone(reason: ActivityEndReason): "warning" | "info" | "muted" {
  if (reason === "idle") return "warning";
  if (reason === "pause" || reason === "break") return "info";
  return "muted";
}

/** "Windows 11 Pro · DESKTOP-7Q2 · agent 0.2.0" — whichever parts were reported; null when none. */
export function deviceSummary(details: Pick<SessionDetails, "platform" | "osVersion" | "deviceName" | "agentVersion">): string | null {
  const parts = [
    details.osVersion ?? platformLabel(details.platform),
    details.deviceName,
    details.agentVersion ? `agent ${details.agentVersion}` : null,
  ].filter((p): p is string => !!p);
  return parts.length > 0 ? parts.join(" · ") : null;
}

/** "Device clock 3 min behind" when the device clock was off by more than two minutes (contract §1); else null. */
export function clockSkewNote(skewSeconds: number | null | undefined): string | null {
  if (typeof skewSeconds !== "number" || !Number.isFinite(skewSeconds) || Math.abs(skewSeconds) <= 120) return null;
  const minutes = Math.round(Math.abs(skewSeconds) / 60);
  const amount = minutes >= 120 ? `${Math.round(minutes / 60)} h` : `${minutes} min`;
  return `Device clock ${amount} ${skewSeconds > 0 ? "behind" : "ahead"}`;
}
