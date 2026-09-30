import type { Activity, ActivityMinute, WindowInfo } from "./types";

/* ============================================================================
 * ACTIVITY REPORT — pure derivations behind one block's detail view.
 *
 * The activity drawer turns a single tracked block into a report: which apps
 * and windows held focus (and the input while they did), and a per-minute
 * timeline. Newer agents send all of it; legacy rows carry only a credited
 * focus window and demo rows carry no timeline, so every derivation here
 * degrades to "nothing to show" rather than inventing numbers.
 * ========================================================================== */

const MS_PER_MIN = 60_000;

/** Whole seconds between the block's start and end (0 for unusable times). */
export function blockSeconds(activity: Pick<Activity, "startedAt" | "endedAt">): number {
  const ms = Date.parse(activity.endedAt) - Date.parse(activity.startedAt);
  return Number.isFinite(ms) ? Math.max(0, Math.round(ms / 1000)) : 0;
}

/** "10:04 AM" in the viewer's zone ("—" for a bad timestamp). */
export function formatClock(iso: string): string {
  const d = new Date(iso);
  return Number.isFinite(d.getTime()) ? d.toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit" }) : "—";
}

/** "10:04" — an axis tick; the AM/PM half is already stated by the block's range. */
export function formatClockShort(iso: string): string {
  const d = new Date(iso);
  if (!Number.isFinite(d.getTime())) return "—";
  return `${d.getHours() % 12 || 12}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/** "10:00 AM – 10:05 AM". */
export function formatTimeRange(activity: Pick<Activity, "startedAt" | "endedAt">): string {
  return `${formatClock(activity.startedAt)} – ${formatClock(activity.endedAt)}`;
}

/* ----------------------------- Apps & windows ----------------------------- */

/** One application's share of a block, with the windows it was focused in. */
export interface AppWindowGroup {
  appName: string;
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
 * Group a block's focus windows by application (names compared
 * case-insensitively; the first spelling seen is kept), most-focused app
 * first. Ties keep the agent's order, which is already seconds-descending.
 */
export function groupWindowsByApp(windows: WindowInfo[]): AppWindowGroup[] {
  const groups = new Map<string, { order: number; windows: WindowInfo[] }>();
  for (const w of windows) {
    const key = w.appName.trim().toLowerCase();
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
