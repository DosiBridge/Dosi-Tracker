import { activities, projectById, projects, userById } from "./tenant-data";
import type { Activity } from "./types";
import { NOW } from "./demo-clock";
import type { RangeKey } from "./reports-data";
import { appKey, canonicalAppName } from "./apps";
import {
  addZonedDays,
  browserTimeZone,
  formatDayRange,
  isDayKey,
  startOfZonedDay,
  zonedDayKey,
  zonedParts,
  zonedPeriod,
} from "./time-zone";

/* ============================================================================
 * ACTIVITY FILTERS — the Activity page's filter engine.
 *
 * One rule for time, the server's: a session belongs to the day and the time
 * of day it STARTED, read in the reporting zone. The date presets are whole
 * calendar days of that zone ("Last 7 days" = today and the six days before),
 * so the bounds a live page sends as From/To and the bounds applied here are
 * the same instants (activityDateRange).
 * ========================================================================== */

export type ActivitySort = "recent" | "oldest" | "prod-high" | "prod-low" | "active-high";

export interface ActivityFilters {
  query: string;
  memberId: string; // "all" or user id
  projectId: string; // "all" or project id
  rangeKey: RangeKey; // date preset ("custom" uses customFrom/customTo)
  customFrom: string; // YYYY-MM-DD, a day of the reporting zone
  customTo: string; // YYYY-MM-DD, a day of the reporting zone
  startTime: string; // "HH:MM" time-of-day lower bound
  endTime: string; // "HH:MM" time-of-day upper bound
  minProductivity: number; // 0-100
  maxProductivity: number; // 0-100
  app: string; // "all" or an app's canonical name
  onlineOnly: boolean;
  webcamOnly: boolean;
  sort: ActivitySort;
}

export const defaultActivityFilters: ActivityFilters = {
  query: "",
  memberId: "all",
  projectId: "all",
  rangeKey: "7d",
  customFrom: "",
  customTo: "",
  startTime: "",
  endTime: "",
  minProductivity: 0,
  maxProductivity: 100,
  app: "all",
  onlineOnly: false,
  webcamOnly: false,
  sort: "recent",
};

/** The clock and zone the date presets are read from. */
export interface FilterClock {
  /** The instant "today" is read from (default: the demo's frozen clock; a live page passes the real one). */
  now?: Date;
  /** The zone the presets, custom days and time-of-day are read in (default: the viewer's). */
  timeZone?: string;
}

export interface ActivityFilterContext extends FilterClock {
  /** Whether a block belongs to a session running right now (from presence); default: the row's own `online` flag. */
  isLive?: (activity: Activity) => boolean;
  /** Whether a block's activity % was measured (default: always). Unmeasured blocks never pass a narrowed activity window and sort last by activity. */
  isMeasured?: (activity: Activity) => boolean;
}

/** The instants a filter's dates cover, [from, to); ±Infinity for an open side. */
export interface ActivityDateRange {
  from: number;
  to: number;
  /** The first and last calendar day covered (null for an open side). */
  fromKey: string | null;
  toKey: string | null;
}

/** Distinct application names (canonical) across `list` — the session's rows by default — for the app filter. */
export function activityApps(list: Activity[] = activities): string[] {
  const set = new Set<string>();
  const add = (name: string) => {
    const canonical = canonicalAppName(name);
    if (canonical && canonical !== "—") set.add(canonical);
  };
  for (const a of list) {
    add(a.screen.app);
    a.activeWindows.forEach((w) => add(w.appName));
    a.runningPrograms.forEach((w) => add(w.appName));
  }
  return [...set].sort();
}

function parseTime(hhmm: string): number | null {
  if (!hhmm) return null;
  const [h, m] = hhmm.split(":").map(Number);
  if (Number.isNaN(h)) return null;
  return h * 60 + (m || 0);
}

function span(fromKey: string, toKey: string, zone: string): ActivityDateRange {
  return {
    from: startOfZonedDay(fromKey, zone).getTime(),
    to: startOfZonedDay(addZonedDays(toKey, 1), zone).getTime(),
    fromKey,
    toKey,
  };
}

/**
 * The instants a filter's date setting covers, as whole days of the zone:
 * Today, Yesterday, the last 7 / 30 days (today included), this month so far,
 * or the custom days (a side left empty is open). "Custom" with no day at all
 * falls back to the last 7 days.
 */
export function activityDateRange(
  f: Pick<ActivityFilters, "rangeKey" | "customFrom" | "customTo">,
  clock: FilterClock = {},
): ActivityDateRange {
  const zone = clock.timeZone ?? browserTimeZone();
  const now = clock.now ?? NOW;
  const today = zonedDayKey(now, zone);
  const fromKey = isDayKey(f.customFrom) ? f.customFrom : null;
  const toKey = isDayKey(f.customTo) ? f.customTo : null;
  switch (f.rangeKey) {
    case "today":
      return span(today, today, zone);
    case "yesterday": {
      const yesterday = addZonedDays(today, -1);
      return span(yesterday, yesterday, zone);
    }
    case "30d":
      return span(zonedPeriod("30d", now, zone).fromKey, today, zone);
    case "month":
      return span(zonedPeriod("month", now, zone).fromKey, today, zone);
    case "custom":
      if (fromKey || toKey) {
        return {
          from: fromKey ? startOfZonedDay(fromKey, zone).getTime() : Number.NEGATIVE_INFINITY,
          to: toKey ? startOfZonedDay(addZonedDays(toKey, 1), zone).getTime() : Number.POSITIVE_INFINITY,
          fromKey,
          toKey,
        };
      }
      return span(zonedPeriod("7d", now, zone).fromKey, today, zone);
    default:
      return span(zonedPeriod("7d", now, zone).fromKey, today, zone);
  }
}

/** "29 Sep – 5 Oct 2026", "From 1 Jul 2026", "Until 5 Oct 2026" — the days a range covers. */
export function formatActivityRange(range: Pick<ActivityDateRange, "fromKey" | "toKey">): string {
  if (range.fromKey && range.toKey) return formatDayRange(range.fromKey, range.toKey);
  if (range.fromKey) return `From ${formatDayRange(range.fromKey, range.fromKey)}`;
  if (range.toKey) return `Until ${formatDayRange(range.toKey, range.toKey)}`;
  return "All time";
}

export function applyActivityFilters(list: Activity[], f: ActivityFilters, context: ActivityFilterContext = {}): Activity[] {
  const zone = context.timeZone ?? browserTimeZone();
  const q = f.query.trim().toLowerCase();
  const { from, to } = activityDateRange(f, { now: context.now, timeZone: zone });
  const startMin = parseTime(f.startTime);
  const endMin = parseTime(f.endTime);
  const appFilter = f.app !== "all" ? appKey(f.app) : null;
  const isLive = context.isLive ?? ((a: Activity) => a.online);
  const isMeasured = context.isMeasured ?? (() => true);
  const narrowed = f.minProductivity !== 0 || f.maxProductivity !== 100;

  const out = list.filter((a) => {
    if (f.memberId !== "all" && a.userId !== f.memberId) return false;
    if (f.projectId !== "all" && a.projectId !== f.projectId) return false;

    const started = Date.parse(a.startedAt);
    if (!Number.isFinite(started) || started < from || started >= to) return false;

    if (startMin !== null || endMin !== null) {
      const { hour, minute } = zonedParts(started, zone);
      const mod = hour * 60 + minute;
      if (startMin !== null && mod < startMin) return false;
      if (endMin !== null && mod > endMin) return false;
    }

    if (isMeasured(a)) {
      if (a.productivity < f.minProductivity || a.productivity > f.maxProductivity) return false;
    } else if (narrowed) {
      // Nothing was measured, so the block is neither inside nor outside an activity window.
      return false;
    }

    if (appFilter) {
      const inApp =
        appKey(a.screen.app) === appFilter ||
        a.activeWindows.some((w) => appKey(w.appName) === appFilter) ||
        a.runningPrograms.some((w) => appKey(w.appName) === appFilter);
      if (!inApp) return false;
    }

    if (f.onlineOnly && !isLive(a)) return false;
    if (f.webcamOnly && !a.hasWebcam) return false;

    if (q) {
      const hit =
        a.description.toLowerCase().includes(q) ||
        a.screen.app.toLowerCase().includes(q) ||
        canonicalAppName(a.screen.app).toLowerCase().includes(q) ||
        a.activeWindows.some(
          (w) =>
            w.appName.toLowerCase().includes(q) ||
            canonicalAppName(w.appName).toLowerCase().includes(q) ||
            w.windowTitle.toLowerCase().includes(q)
        ) ||
        (userById(a.userId)?.name.toLowerCase().includes(q) ?? false) ||
        (projectById(a.projectId)?.title.toLowerCase().includes(q) ?? false);
      if (!hit) return false;
    }

    return true;
  });

  const byEnd = (a: Activity) => new Date(a.endedAt).getTime();
  // Blocks without a measurement sort after every measured one, in either direction.
  const byActivity = (a: Activity, b: Activity, direction: 1 | -1) => {
    const ma = isMeasured(a);
    const mb = isMeasured(b);
    if (ma !== mb) return ma ? -1 : 1;
    return ma ? direction * (a.productivity - b.productivity) : 0;
  };
  out.sort((a, b) => {
    switch (f.sort) {
      case "oldest":
        return byEnd(a) - byEnd(b);
      case "prod-high":
        return byActivity(a, b, -1);
      case "prod-low":
        return byActivity(a, b, 1);
      case "active-high":
        return b.mouseClicks + b.keyboardHits - (a.mouseClicks + a.keyboardHits);
      default:
        return byEnd(b) - byEnd(a);
    }
  });

  return out;
}

/** Whether a filter narrows rows beyond what the server query (dates, member, project) already does. */
export function hasLocalFilters(f: ActivityFilters): boolean {
  const d = defaultActivityFilters;
  return (
    f.query.trim() !== "" ||
    !!f.startTime ||
    !!f.endTime ||
    f.minProductivity !== d.minProductivity ||
    f.maxProductivity !== d.maxProductivity ||
    f.app !== d.app ||
    f.onlineOnly ||
    f.webcamOnly
  );
}

/** Count how many filters differ from defaults (for the "N active" badge). */
export function countActiveFilters(f: ActivityFilters): number {
  let n = 0;
  const d = defaultActivityFilters;
  if (f.memberId !== d.memberId) n++;
  if (f.projectId !== d.projectId) n++;
  if (f.rangeKey !== d.rangeKey || f.customFrom || f.customTo) n++;
  if (f.startTime || f.endTime) n++;
  if (f.minProductivity !== d.minProductivity || f.maxProductivity !== d.maxProductivity) n++;
  if (f.app !== d.app) n++;
  if (f.onlineOnly) n++;
  if (f.webcamOnly) n++;
  return n;
}

export interface FilterChip {
  key: keyof ActivityFilters | "productivity" | "time";
  label: string;
}

/** Human-readable chips for the active (non-default) filters. */
export function activeFilterChips(f: ActivityFilters): FilterChip[] {
  const chips: FilterChip[] = [];
  if (f.memberId !== "all") chips.push({ key: "memberId", label: `Member: ${userById(f.memberId)?.name ?? f.memberId}` });
  if (f.projectId !== "all") chips.push({ key: "projectId", label: `Project: ${projectById(f.projectId)?.title ?? f.projectId}` });
  if (f.rangeKey !== "7d" && !(f.rangeKey === "custom" && !f.customFrom && !f.customTo)) {
    if (f.rangeKey === "custom") chips.push({ key: "rangeKey", label: `Date: ${f.customFrom || "…"} → ${f.customTo || "…"}` });
    else chips.push({ key: "rangeKey", label: `Date: ${rangeLabel(f.rangeKey)}` });
  }
  if (f.startTime || f.endTime) chips.push({ key: "time", label: `Time: ${f.startTime || "00:00"}–${f.endTime || "23:59"}` });
  if (f.minProductivity !== 0 || f.maxProductivity !== 100) chips.push({ key: "productivity", label: `Activity: ${f.minProductivity}–${f.maxProductivity}%` });
  if (f.app !== "all") chips.push({ key: "app", label: `App: ${f.app}` });
  if (f.onlineOnly) chips.push({ key: "onlineOnly", label: "Live only" });
  if (f.webcamOnly) chips.push({ key: "webcamOnly", label: "With webcam" });
  return chips;
}

function rangeLabel(key: RangeKey): string {
  const map: Record<RangeKey, string> = {
    today: "Today",
    yesterday: "Yesterday",
    "7d": "Last 7 days",
    "30d": "Last 30 days",
    month: "This month",
    custom: "Custom",
  };
  return map[key];
}

export const memberOptions = () => [
  { id: "all", name: "All members" },
  ...activities
    .reduce<string[]>((acc, a) => (acc.includes(a.userId) ? acc : [...acc, a.userId]), [])
    .map((id) => ({ id, name: userById(id)?.name ?? id })),
];

export const projectOptions = () => [
  { id: "all", title: "All projects" },
  ...projects.map((p) => ({ id: p.id, title: p.title })),
];
