import type { Activity } from "./types";
import { normalizeActivityNote } from "./api-contract";
import { normalizeUtcIso } from "./live-dataset";
import { addZonedDays, countDays, isDayKey, zonedDayKey, zonedDayRange, zonedParts, zonedPeriod } from "./time-zone";

/* ============================================================================
 * WORK REPORT — the pure derivations behind the client-ready daily hours report.
 *
 * A per-day report is only honest if every row is a WHOLE day, so the period is
 * snapped to complete calendar days of ONE reporting zone (the viewer's
 * preference, else the workspace default, else the browser's — see
 * hooks/use-reporting-zone). The backend buckets the same way when it is sent
 * that zone (contract §6): a block belongs wholly to the day it STARTED on, and
 * a period holds the blocks that started inside it.
 *
 * Rounding rule: every displayed cell is rounded to whole minutes FIRST and the
 * totals are sums of those rounded cells, so a printed Total row always equals
 * the sum of the rows above it — in decimal hours too, where a total is the sum
 * of the rounded decimals it closes.
 *
 * Activity % that was not measured (input capture off or unavailable) is null,
 * never 0: it is left out of every average and reads "—". Idle minutes are
 * null wherever a source has no measurement of them.
 *
 * Everything here is a pure function of its inputs (no clock, no React, no
 * network): demo mode feeds it seeded activities, live mode the
 * /api/app/reporting/* payloads, and the unit tests feed it edge cases.
 * ========================================================================== */

const DAY_MS = 86_400_000;
const MINUTE_MS = 60_000;

/** The backend's longest accepted report span (GetReportSummaryInput.MaxRangeDays). */
export const MAX_REPORT_DAYS = 366;

/** A full working day, drawn as the chart's reference line. */
export const TARGET_DAY_MINUTES = 8 * 60;

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] as const;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"] as const;
const MONTH_NAMES = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
] as const;

/* ----------------------------- Sanitising ----------------------------- */

function toNumber(value: unknown): number {
  if (typeof value === "number") return value;
  if (typeof value === "string" && value.trim() !== "") return Number(value);
  return Number.NaN;
}

/** A non-negative finite number of minutes; anything else (NaN, negative, garbage) is 0. */
export function cleanMinutes(value: unknown): number {
  const n = toNumber(value);
  return Number.isFinite(n) && n > 0 ? n : 0;
}

/** A 0–100 score; non-finite input is 0 and out-of-range input is clamped. */
export function cleanPercent(value: unknown): number {
  const n = toNumber(value);
  if (!Number.isFinite(n)) return 0;
  return Math.min(100, Math.max(0, n));
}

/**
 * A 0–100 score, or null for "not measured": null, undefined and anything
 * unreadable. A measured score is clamped to 0–100.
 */
export function optionalPercent(value: unknown): number | null {
  if (value === null || value === undefined) return null;
  const n = toNumber(value);
  return Number.isFinite(n) ? Math.min(100, Math.max(0, n)) : null;
}

/** Non-negative minutes, or null when the value is absent or unreadable (an older server, or not measured). */
export function optionalMinutes(value: unknown): number | null {
  if (value === null || value === undefined) return null;
  const n = toNumber(value);
  return Number.isFinite(n) ? Math.max(0, n) : null;
}

/** A non-negative whole count. */
function cleanCount(value: unknown): number {
  const n = toNumber(value);
  return Number.isFinite(n) && n > 0 ? Math.round(n) : 0;
}

/* ----------------------------- Days ----------------------------- */

function keyFromUtc(ms: number): string {
  return new Date(ms).toISOString().slice(0, 10);
}

function utcMidnight(key: string): number {
  return Date.UTC(Number(key.slice(0, 4)), Number(key.slice(5, 7)) - 1, Number(key.slice(8, 10)));
}

/**
 * The calendar day ("2026-09-28") a server date names, or a Date's UTC day;
 * null when unreadable. Server dates are already days of the zone they were
 * asked for, so the date part is read straight off the string — parsing
 * "2026-09-28T00:00:00" (no zone) as a local time would shift it.
 */
export function utcDayKey(value: unknown): string | null {
  if (value instanceof Date) {
    return Number.isFinite(value.getTime()) ? value.toISOString().slice(0, 10) : null;
  }
  if (typeof value !== "string") return null;
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(value.trim());
  if (!m) return null;
  const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const back = new Date(Date.UTC(y, mo - 1, d));
  if (back.getUTCFullYear() !== y || back.getUTCMonth() !== mo - 1 || back.getUTCDate() !== d) return null;
  return `${m[1]}-${m[2]}-${m[3]}`;
}

/** Day of week (0 = Sunday) of a "YYYY-MM-DD" key. */
function weekdayIndex(key: string): number {
  return new Date(utcMidnight(key)).getUTCDay();
}

/** Saturday or Sunday. */
export function isWeekend(key: string): boolean {
  const d = weekdayIndex(key);
  return d === 0 || d === 6;
}

/** "Mon". */
export function weekdayShort(key: string): string {
  return WEEKDAYS[weekdayIndex(key)];
}

/** "Mon, 28 Sep" — or "Mon, 28 Sep 2026" with `year`. Deterministic (no locale data). */
export function formatDayLabel(key: string, options: { year?: boolean } = {}): string {
  const date = new Date(utcMidnight(key));
  const base = `${WEEKDAYS[date.getUTCDay()]}, ${date.getUTCDate()} ${MONTHS[date.getUTCMonth()]}`;
  return options.year ? `${base} ${date.getUTCFullYear()}` : base;
}

/** Compact axis label: "Mon 28" for short periods, "28 Sep" for long ones. */
export function formatAxisDay(key: string, dense: boolean): string {
  const date = new Date(utcMidnight(key));
  return dense
    ? `${date.getUTCDate()} ${MONTHS[date.getUTCMonth()]}`
    : `${WEEKDAYS[date.getUTCDay()]} ${date.getUTCDate()}`;
}

/**
 * The document's period line: "Mon, 28 Sep – Sun, 4 Oct 2026". The year is
 * written once unless the period crosses New Year; a single day reads
 * "Sun, 4 Oct 2026"; no days read "—".
 */
export function formatPeriod(days: readonly string[]): string {
  if (!days.length) return "—";
  const first = days[0];
  const last = days[days.length - 1];
  if (first === last) return formatDayLabel(first, { year: true });
  const sameYear = first.slice(0, 4) === last.slice(0, 4);
  return `${formatDayLabel(first, { year: !sameYear })} – ${formatDayLabel(last, { year: true })}`;
}

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

/** "4 Oct 2026, 15:30" in the viewer's local time (when the document was produced). */
export function formatStamp(date: Date): string {
  if (!Number.isFinite(date.getTime())) return "—";
  return `${date.getDate()} ${MONTHS[date.getMonth()]} ${date.getFullYear()}, ${pad2(date.getHours())}:${pad2(date.getMinutes())}`;
}

/** "4 Oct 2026, 15:30" as the clock reads in the reporting zone. */
export function formatStampInZone(date: Date, timeZone: string): string {
  if (!Number.isFinite(date.getTime())) return "—";
  const p = zonedParts(date, timeZone);
  return `${p.day} ${MONTHS[p.month - 1]} ${p.year}, ${pad2(p.hour)}:${pad2(p.minute)}`;
}

export interface ReportSpan {
  /** Every day of the period, oldest first ("YYYY-MM-DD"). */
  days: string[];
  /** Query bounds: the start of the first day … the start of the day after the last. */
  from: Date;
  to: Date;
}

/**
 * Snap a browser-local range to WHOLE days, reported as the UTC days of the
 * same names (the rule before reports had a time zone; screens still on UTC
 * days use it). A rolling window that starts mid-day begins on the next date,
 * so "last 7 days" is exactly 7 full days ending today. At most `max` days
 * (the most recent ones) are kept.
 */
export function reportSpan(from: Date, to: Date, max: number = MAX_REPORT_DAYS): ReportSpan {
  const empty = { days: [], from, to };
  if (!Number.isFinite(from.getTime()) || !Number.isFinite(to.getTime()) || to.getTime() < from.getTime()) {
    return empty;
  }
  let start = Date.UTC(from.getFullYear(), from.getMonth(), from.getDate());
  const end = Date.UTC(to.getFullYear(), to.getMonth(), to.getDate());
  const midDay =
    from.getHours() !== 0 || from.getMinutes() !== 0 || from.getSeconds() !== 0 || from.getMilliseconds() !== 0;
  if (midDay && start + DAY_MS <= end) start += DAY_MS;

  const limit = Math.max(0, Math.floor(max));
  if (limit === 0) return empty;
  const count = Math.min(Math.round((end - start) / DAY_MS) + 1, limit);
  const first = end - (count - 1) * DAY_MS;
  const days: string[] = [];
  for (let i = 0; i < count; i++) days.push(keyFromUtc(first + i * DAY_MS));
  return { days, from: new Date(first), to: new Date(end + DAY_MS) };
}

/* ----------------------------- Periods ----------------------------- */

/** The periods the report steps through. "week" is Monday–Sunday. */
export type PeriodPreset = "week" | "7d" | "30d" | "month";

export const PERIOD_PRESETS: readonly { key: PeriodPreset; label: string }[] = [
  { key: "week", label: "Week" },
  { key: "7d", label: "7 days" },
  { key: "30d", label: "30 days" },
  { key: "month", label: "Month" },
];

function addLocalDays(date: Date, days: number): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate() + days);
}

/**
 * The browser-local calendar dates a period covers, `offset` periods back from
 * the one containing `now` (0 = current; positive offsets are treated as 0 —
 * there is no future to report). The current week and month run up to today
 * rather than into days that have not happened; earlier ones are complete.
 */
export function periodBounds(preset: PeriodPreset, now: Date, offset = 0): { from: Date; to: Date } {
  const back = Math.min(0, Math.trunc(Number.isFinite(offset) ? offset : 0));
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  switch (preset) {
    case "week": {
      const monday = addLocalDays(today, -((today.getDay() + 6) % 7) + back * 7);
      return { from: monday, to: back === 0 ? today : addLocalDays(monday, 6) };
    }
    case "30d": {
      const end = addLocalDays(today, back * 30);
      return { from: addLocalDays(end, -29), to: end };
    }
    case "month": {
      const first = new Date(today.getFullYear(), today.getMonth() + back, 1);
      const last = new Date(first.getFullYear(), first.getMonth() + 1, 0);
      return { from: first, to: back === 0 ? today : last };
    }
    default: {
      const end = addLocalDays(today, back * 7);
      return { from: addLocalDays(end, -6), to: end };
    }
  }
}

/** The whole-day UTC span of a browser-local period (see {@link periodBounds} and {@link reportSpan}). */
export function periodSpan(preset: PeriodPreset, now: Date, offset = 0): ReportSpan {
  const { from, to } = periodBounds(preset, now, offset);
  return reportSpan(from, to);
}

/**
 * "This week", "Last month", "3 weeks ago", "July 2026"… A month further back
 * is named as the reporting zone reads it when `timeZone` is given (the
 * browser's calendar otherwise).
 */
export function periodTitle(preset: PeriodPreset, now: Date, offset = 0, timeZone?: string): string {
  const back = -Math.min(0, Math.trunc(Number.isFinite(offset) ? offset : 0));
  switch (preset) {
    case "week":
      return back === 0 ? "This week" : back === 1 ? "Last week" : `${back} weeks ago`;
    case "30d":
      return back === 0 ? "Last 30 days" : "30-day period";
    case "month": {
      if (back === 0) return "This month";
      if (back === 1) return "Last month";
      if (timeZone) {
        const first = zonedPeriod("month", now, timeZone, -back).fromKey;
        return `${MONTH_NAMES[Number(first.slice(5, 7)) - 1]} ${Number(first.slice(0, 4))}`;
      }
      const first = periodBounds("month", now, -back).from;
      return `${MONTH_NAMES[first.getMonth()]} ${first.getFullYear()}`;
    }
    default:
      return back === 0 ? "Last 7 days" : "7-day period";
  }
}

/**
 * What a browser-local period is compared with: the period before it, cut to
 * the same number of days — so a week or month still in progress is set
 * against the same days of the previous one, not against a whole period.
 */
export function comparisonSpan(preset: PeriodPreset, now: Date, offset: number, dayCount: number): ReportSpan {
  const previous = periodSpan(preset, now, Math.min(0, offset) - 1);
  const days = previous.days.slice(0, Math.max(0, dayCount));
  if (!days.length) return { days, from: previous.from, to: previous.from };
  return { days, from: previous.from, to: new Date(utcMidnight(days[days.length - 1]) + DAY_MS) };
}

/** How the comparison reads next to the change. */
export function comparisonLabel(preset: PeriodPreset, offset: number): string {
  const current = Math.min(0, offset) === 0;
  switch (preset) {
    case "week":
      return current ? "vs same days last week" : "vs the week before";
    case "month":
      return current ? "vs same days last month" : "vs the month before";
    case "30d":
      return "vs the 30 days before";
    default:
      return "vs the 7 days before";
  }
}

/** A period of whole calendar days in the reporting zone. */
export interface ReportPeriod extends ReportSpan {
  /** First and last day, both included ("YYYY-MM-DD"). */
  fromKey: string;
  toKey: string;
}

/** What the viewer picked: a preset stepped `offset` periods back, or their own range of days (both included). */
export type PeriodSelection =
  | { kind: "preset"; preset: PeriodPreset; offset: number }
  | { kind: "custom"; from: string; to: string };

/**
 * The selected period's whole days in `timeZone`, with their exact query
 * bounds (local midnight to local midnight — exact across DST). A custom range
 * longer than the backend accepts keeps its most recent days; an unreadable
 * one falls back to the last 7 days.
 */
export function resolvePeriod(selection: PeriodSelection, now: Date, timeZone: string): ReportPeriod {
  if (selection.kind === "custom") {
    const { from, to } = selection;
    if (isDayKey(from) && isDayKey(to) && from <= to) {
      const fromKey = countDays(from, to) > MAX_REPORT_DAYS ? addZonedDays(to, -(MAX_REPORT_DAYS - 1)) : from;
      return { fromKey, toKey: to, ...zonedDayRange(fromKey, to, timeZone) };
    }
    return zonedPeriod("7d", now, timeZone);
  }
  return zonedPeriod(selection.preset, now, timeZone, selection.offset);
}

/**
 * What a period is compared with — never overlapping it. A preset meets the
 * period before it cut to the same number of days (a week or month in
 * progress meets the same days of the previous one; a 31-day month meets all
 * of a 28-day February, never the first days of itself); a custom range meets
 * the same number of days just before it.
 */
export function comparisonFor(selection: PeriodSelection, now: Date, timeZone: string, current: ReportPeriod): ReportPeriod {
  const count = current.days.length;
  const none: ReportPeriod = { days: [], fromKey: current.fromKey, toKey: current.fromKey, from: current.from, to: current.from };
  if (count === 0) return none;
  const before =
    selection.kind === "custom"
      ? Array.from({ length: count }, (_, i) => addZonedDays(current.fromKey, i - count))
      : zonedPeriod(selection.preset, now, timeZone, Math.min(0, selection.offset) - 1).days.slice(0, count);
  // Whatever a preset's calendar rules say, a day of the period itself is never its own baseline.
  const days = before.filter((d) => d < current.fromKey);
  if (!days.length) return none;
  const fromKey = days[0];
  const toKey = days[days.length - 1];
  return { fromKey, toKey, ...zonedDayRange(fromKey, toKey, timeZone) };
}

/** How the comparison reads next to a change: "vs same days last week", "vs the 12 days before"… */
export function comparisonText(selection: PeriodSelection, dayCount: number): string {
  if (selection.kind === "custom") return `vs the ${dayCount} ${dayCount === 1 ? "day" : "days"} before`;
  return comparisonLabel(selection.preset, selection.offset);
}

/** The toolbar's title for a selection: "This week", "Last month", "July 2026"… or "Custom range". */
export function selectionTitle(selection: PeriodSelection, now: Date, timeZone: string): string {
  return selection.kind === "custom" ? "Custom range" : periodTitle(selection.preset, now, selection.offset, timeZone);
}

export interface Change {
  /** Whole percent, signed. */
  percent: number;
  direction: "up" | "down" | "flat";
}

/** The change from `previous` to `current`; null when there is nothing to compare against. */
export function changeVs(current: number, previous: number | null | undefined): Change | null {
  if (previous === null || previous === undefined) return null;
  const before = cleanMinutes(previous);
  if (before <= 0) return null;
  const percent = Math.round(((cleanMinutes(current) - before) / before) * 100);
  return { percent, direction: percent > 0 ? "up" : percent < 0 ? "down" : "flat" };
}

/**
 * Mon-first calendar weeks covering `days`: each week has 7 slots, holding a
 * day of the period or null for padding before the first / after the last day.
 */
export function calendarWeeks(days: readonly string[]): (string | null)[][] {
  if (!days.length) return [];
  const lead = (weekdayIndex(days[0]) + 6) % 7;
  const slots: (string | null)[] = [...Array<null>(lead).fill(null), ...days];
  while (slots.length % 7) slots.push(null);
  const weeks: (string | null)[][] = [];
  for (let i = 0; i < slots.length; i += 7) weeks.push(slots.slice(i, i + 7));
  return weeks;
}

/* ----------------------------- Formatting ----------------------------- */

/** How hours read on screen: "7h 32m", "7:32" (timesheets) or "7.53" (billing). */
export type HoursFormat = "duration" | "clock" | "decimal";

export const HOURS_FORMATS: readonly { key: HoursFormat; label: string; description: string }[] = [
  { key: "duration", label: "7h 30m", description: "Hours and minutes" },
  { key: "clock", label: "7:30", description: "Clock time (h:mm)" },
  { key: "decimal", label: "7.50", description: "Decimal hours, for billing" },
];

/** Minutes in the chosen hours format. */
export function formatHours(minutes: unknown, format: HoursFormat): string {
  if (format === "clock") return formatHM(minutes);
  if (format === "decimal") return decimalHours(minutes).toFixed(2);
  return formatHoursLong(minutes);
}

/** Hours as "h:mm" ("7:32", "0:05", "41:00"); garbage reads "0:00". */
export function formatHM(minutes: unknown): string {
  const total = Math.round(cleanMinutes(minutes));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

/** Hours as "7h 32m" ("45m", "8h", "0m"). Rounds to whole minutes before splitting (never "1h 60m"). */
export function formatHoursLong(minutes: unknown): string {
  const total = Math.round(cleanMinutes(minutes));
  const h = Math.floor(total / 60);
  const m = total % 60;
  if (h === 0) return `${m}m`;
  return m === 0 ? `${h}h` : `${h}h ${m}m`;
}

/** Decimal hours to 2 places (7:32 → 7.53), for spreadsheets. */
export function decimalHours(minutes: unknown): number {
  return Math.round((cleanMinutes(minutes) / 60) * 100) / 100;
}

/**
 * The decimal hours of a column of cells, as the SUM of the rounded cells —
 * five rows of 7.53 total 37.65, not the 37.67 the minutes would round to —
 * so a printed or exported total always equals the figures above it.
 */
export function sumDecimalHours(cells: readonly number[]): number {
  let cents = 0;
  for (const c of cells) cents += Math.round(decimalHours(c) * 100);
  return cents / 100;
}

/** A total in the viewer's hours format; in decimal hours it is the sum of the rounded cells it closes. */
export function formatTotal(cells: readonly number[], format: HoursFormat): string {
  if (format === "decimal") return sumDecimalHours(cells).toFixed(2);
  return formatHours(cells.reduce((s, c) => s + cleanMinutes(c), 0), format);
}

/* ----------------------------- Activity level ----------------------------- */

export type ActivityLevel = "good" | "ok" | "low" | "none";

/**
 * Same thresholds as the rest of the app (utils.scoreTone); "none" when no
 * time was tracked or the activity was not measured.
 */
export function activityLevel(percent: number | null | undefined, minutes = 1): ActivityLevel {
  if (cleanMinutes(minutes) <= 0 || percent === null || percent === undefined) return "none";
  const p = cleanPercent(percent);
  if (p >= 75) return "good";
  if (p >= 50) return "ok";
  return "low";
}

export const activityLevelLabel: Record<ActivityLevel, string> = {
  good: "Good",
  ok: "Fair",
  low: "Low",
  none: "—",
};

/** "73%", or "—" for activity that was not measured. */
export function formatActivity(percent: number | null | undefined): string {
  return percent === null || percent === undefined || !Number.isFinite(percent) ? "—" : `${Math.round(percent)}%`;
}

/* ----------------------------- Daily log ----------------------------- */

/** One input point: a day's (partial) minutes. Several points on one day are merged. */
export interface DailyPoint {
  date: unknown;
  minutes: unknown;
  /** 0–100 activity of the measured minutes; null or absent when nothing was measured. */
  activity?: unknown;
  /** The minutes `activity` was measured over (default: all of `minutes`). */
  measuredMinutes?: unknown;
  /** Minutes without keyboard or mouse input; null or absent when unknown. */
  idleMinutes?: unknown;
  /** Tracked blocks these minutes came from. */
  sessions?: unknown;
}

export interface DayRow {
  date: string;
  weekday: string;
  label: string;
  weekend: boolean;
  /** Whole minutes worked. */
  minutes: number;
  /** Activity weighted by measured minutes, 0–100; null when nothing that day was measured (or no time). */
  activity: number | null;
  /** The minutes `activity` stands for (0 without a measurement). */
  measuredMinutes: number;
  /** Whole minutes without input; null when the source doesn't measure idle time. */
  idleMinutes: number | null;
  sessions: number;
  level: ActivityLevel;
}

interface DayAcc {
  minutes: number;
  weighted: number;
  measured: number;
  idle: number;
  idleKnown: boolean;
  sessions: number;
}

const freshDay = (): DayAcc => ({ minutes: 0, weighted: 0, measured: 0, idle: 0, idleKnown: false, sessions: 0 });

/**
 * One row per day of the period. Points are merged per day; activity is
 * weighted by the minutes it was measured over, and unmeasured points are
 * left out of it. A point that falls outside `days` but carries tracked time
 * is kept as an extra row (tracked time is never silently dropped); empty
 * points outside the period are ignored.
 */
export function buildDailyLog(days: readonly string[], points: readonly DailyPoint[]): DayRow[] {
  const acc = new Map<string, DayAcc>();
  for (const d of days) {
    const key = utcDayKey(d);
    if (key && !acc.has(key)) acc.set(key, freshDay());
  }
  for (const p of points) {
    if (!p || typeof p !== "object") continue;
    const key = utcDayKey(p.date);
    if (!key) continue;
    const minutes = cleanMinutes(p.minutes);
    const sessions = cleanCount(p.sessions);
    let entry = acc.get(key);
    if (!entry) {
      if (minutes <= 0 && sessions <= 0) continue;
      entry = freshDay();
      acc.set(key, entry);
    }
    entry.minutes += minutes;
    entry.sessions += sessions;
    const activity = optionalPercent(p.activity);
    if (activity !== null) {
      const measured = p.measuredMinutes === undefined || p.measuredMinutes === null ? minutes : Math.min(minutes, cleanMinutes(p.measuredMinutes));
      entry.measured += measured;
      entry.weighted += measured * activity;
    }
    const idle = optionalMinutes(p.idleMinutes);
    if (idle !== null) {
      entry.idle += Math.min(idle, minutes);
      entry.idleKnown = true;
    }
  }
  return [...acc.entries()]
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([date, e]) => {
      const minutes = Math.round(e.minutes);
      const activity = minutes > 0 && e.measured > 0 ? Math.round(e.weighted / e.measured) : null;
      return {
        date,
        weekday: weekdayShort(date),
        label: formatDayLabel(date),
        weekend: isWeekend(date),
        minutes,
        activity,
        measuredMinutes: activity === null ? 0 : e.measured,
        idleMinutes: e.idleKnown ? Math.min(minutes, Math.round(e.idle)) : null,
        sessions: e.sessions,
        level: activityLevel(activity, minutes),
      };
    });
}

export interface WorkSummary {
  totalMinutes: number;
  /** Days in the period. */
  dayCount: number;
  /** Days with any tracked time. */
  workedDays: number;
  /** Monday–Friday days in the period. */
  weekdayCount: number;
  /** Whole minutes per worked day (0 without worked days). */
  averagePerWorkedDay: number;
  /** Activity over the whole period, 0–100; null when nothing was measured (or no time). */
  averageActivity: number | null;
  level: ActivityLevel;
  sessions: number;
  /** The longest day (earliest on a tie); null when nothing was tracked. */
  busiest: DayRow | null;
  maxDayMinutes: number;
  /** Days that reached the 8h target. */
  targetDays: number;
  weekendMinutes: number;
  /** Whole minutes without input over the period (the sum of the days); null when unknown. */
  idleMinutes: number | null;
}

/**
 * Totals of a daily log. Every figure is derived from the rows, so the Total
 * row and the KPIs agree — except the period's activity, which a source that
 * knows it exactly (the server, weighting by measured minutes across the whole
 * period) passes in as `totals.activity`.
 */
export function summarizeLog(rows: readonly DayRow[], totals: { activity?: number | null } = {}): WorkSummary {
  let totalMinutes = 0;
  let weighted = 0;
  let measured = 0;
  let workedDays = 0;
  let weekdayCount = 0;
  let sessions = 0;
  let targetDays = 0;
  let weekendMinutes = 0;
  let idle = 0;
  let idleKnown = false;
  let busiest: DayRow | null = null;
  for (const r of rows) {
    totalMinutes += r.minutes;
    sessions += r.sessions;
    if (r.activity !== null && r.measuredMinutes > 0) {
      weighted += r.activity * r.measuredMinutes;
      measured += r.measuredMinutes;
    }
    if (r.idleMinutes !== null) {
      idle += r.idleMinutes;
      idleKnown = true;
    }
    if (!r.weekend) weekdayCount++;
    else weekendMinutes += r.minutes;
    if (r.minutes > 0) workedDays++;
    if (r.minutes >= TARGET_DAY_MINUTES) targetDays++;
    if (r.minutes > 0 && (!busiest || r.minutes > busiest.minutes)) busiest = r;
  }
  const derived = measured > 0 ? Math.round(weighted / measured) : null;
  const given = totals.activity === undefined ? derived : optionalPercent(totals.activity);
  const averageActivity = totalMinutes > 0 && given !== null ? Math.round(given) : null;
  return {
    totalMinutes,
    dayCount: rows.length,
    workedDays,
    weekdayCount,
    averagePerWorkedDay: workedDays > 0 ? Math.round(totalMinutes / workedDays) : 0,
    averageActivity,
    level: activityLevel(averageActivity, totalMinutes),
    sessions,
    busiest,
    maxDayMinutes: busiest?.minutes ?? 0,
    targetDays,
    weekendMinutes,
    idleMinutes: idleKnown ? idle : null,
  };
}

/** Width (0–100 %) of a proportional bar; a non-zero value is never drawn thinner than 2 %. */
export function barPercent(minutes: number, max: number): number {
  const m = cleanMinutes(minutes);
  const top = cleanMinutes(max);
  if (m <= 0 || top <= 0) return 0;
  return Math.max(2, Math.min(100, Math.round((m / top) * 100)));
}

/** Heat-map intensity 0 (none) … 4 (top quarter of `max`). */
export function heatLevel(minutes: number, max: number): 0 | 1 | 2 | 3 | 4 {
  const m = cleanMinutes(minutes);
  const top = cleanMinutes(max);
  if (m <= 0 || top <= 0) return 0;
  return Math.min(4, Math.max(1, Math.ceil((Math.min(m, top) / top) * 4))) as 1 | 2 | 3 | 4;
}

/**
 * A tidy hour axis: round steps (1, 2, 4, 8, 12, 24, then multiples of 24)
 * covering `maxHours` and at least `minTop`, with at most ~5 intervals.
 */
export function hourTicks(maxHours: number, minTop = 0): { top: number; ticks: number[] } {
  const want = Math.max(cleanMinutes(maxHours), cleanMinutes(minTop), 1);
  const steps = [1, 2, 4, 8, 12, 24];
  let step = steps.find((s) => want / s <= 5) ?? 24 * Math.ceil(want / (24 * 5));
  if (!Number.isFinite(step) || step <= 0) step = 1;
  const top = Math.ceil(want / step) * step;
  const ticks: number[] = [];
  for (let t = 0; t <= top; t += step) ticks.push(t);
  return { top, ticks };
}

export interface ChartDay {
  date: string;
  label: string;
  hours: number;
  minutes: number;
  activity: number | null;
  idleMinutes: number | null;
  weekend: boolean;
}

/** Bar-chart rows: hours per day (2 dp) plus what the tooltip needs. */
export function chartSeries(rows: readonly DayRow[]): ChartDay[] {
  return rows.map((r) => ({
    date: r.date,
    label: r.label,
    hours: Math.round((r.minutes / 60) * 100) / 100,
    minutes: r.minutes,
    activity: r.activity,
    idleMinutes: r.idleMinutes,
    weekend: r.weekend,
  }));
}

/* ----------------------------- Team timesheet matrix ----------------------------- */

export interface MemberDayPoint {
  userId: unknown;
  date: unknown;
  minutes: unknown;
  /** A display name the source already knows (e.g. the server's user name). */
  name?: unknown;
  /** Minutes without input that day; null or absent when unknown. */
  idleMinutes?: unknown;
}

export interface MatrixRow {
  userId: string;
  name: string;
  /** Whole minutes per day, aligned with `days`. */
  cells: number[];
  total: number;
  workedDays: number;
  /** Whole minutes without input over the period; null when unknown. */
  idleMinutes: number | null;
}

export interface TimesheetMatrix {
  days: string[];
  rows: MatrixRow[];
  dayTotals: number[];
  grandTotal: number;
  /** The largest single cell (for heat shading). */
  maxCell: number;
  /** The members' idle minutes together; null when no member's idle time is known. */
  idleTotal: number | null;
}

/**
 * Members × days. Cells are whole minutes; row, column and grand totals are
 * sums of those cells. Members without tracked time are left out; rows are
 * ordered by total (desc), then name. Days with time that fall outside `days`
 * are added as extra columns rather than dropped.
 */
export function buildTimesheetMatrix(
  days: readonly string[],
  points: readonly MemberDayPoint[],
  nameOf: (userId: string, hint: string | null) => string,
): TimesheetMatrix {
  const raw = new Map<string, Map<string, number>>();
  const idle = new Map<string, number>();
  const hints = new Map<string, string>();
  const daySet = new Set<string>();
  for (const d of days) {
    const key = utcDayKey(d);
    if (key) daySet.add(key);
  }
  for (const p of points) {
    if (!p || typeof p !== "object") continue;
    const userId = typeof p.userId === "string" && p.userId ? p.userId : null;
    const key = utcDayKey(p.date);
    const minutes = cleanMinutes(p.minutes);
    if (!userId || !key || minutes <= 0) continue;
    daySet.add(key);
    const perDay = raw.get(userId) ?? new Map<string, number>();
    perDay.set(key, (perDay.get(key) ?? 0) + minutes);
    raw.set(userId, perDay);
    const idleMinutes = optionalMinutes(p.idleMinutes);
    if (idleMinutes !== null) idle.set(userId, (idle.get(userId) ?? 0) + Math.min(idleMinutes, minutes));
    if (typeof p.name === "string" && p.name.trim() && !hints.has(userId)) hints.set(userId, p.name.trim());
  }
  const axis = [...daySet].sort();
  const rows: MatrixRow[] = [];
  for (const [userId, perDay] of raw) {
    const cells = axis.map((d) => Math.round(perDay.get(d) ?? 0));
    const total = cells.reduce((s, c) => s + c, 0);
    if (total <= 0) continue;
    const idleMinutes = idle.get(userId);
    rows.push({
      userId,
      name: nameOf(userId, hints.get(userId) ?? null),
      cells,
      total,
      workedDays: cells.filter((c) => c > 0).length,
      idleMinutes: idleMinutes === undefined ? null : Math.min(total, Math.round(idleMinutes)),
    });
  }
  rows.sort((a, b) => b.total - a.total || a.name.localeCompare(b.name));
  const dayTotals = axis.map((_, i) => rows.reduce((s, r) => s + r.cells[i], 0));
  const knownIdle = rows.filter((r) => r.idleMinutes !== null);
  return {
    days: axis,
    rows,
    dayTotals,
    grandTotal: dayTotals.reduce((s, c) => s + c, 0),
    maxCell: rows.reduce((m, r) => Math.max(m, ...r.cells), 0),
    idleTotal: knownIdle.length ? knownIdle.reduce((s, r) => s + (r.idleMinutes ?? 0), 0) : null,
  };
}

/**
 * A team's average day per person: total ÷ member-days worked (a member-day
 * is one member with tracked time on one day). Whole minutes; 0 when nothing.
 */
export function perMemberDay(matrix: TimesheetMatrix): { minutes: number; memberDays: number } {
  const memberDays = matrix.rows.reduce((s, r) => s + r.workedDays, 0);
  return { minutes: memberDays > 0 ? Math.round(matrix.grandTotal / memberDays) : 0, memberDays };
}

/* ----------------------------- Projects ----------------------------- */

export interface ProjectMinutes {
  projectId: unknown;
  minutes: unknown;
  /** A title the source already knows (the server's projectTitle). */
  title?: unknown;
  /** The source marks the project archived. */
  archived?: unknown;
}

export interface ProjectShare {
  projectId: string;
  name: string;
  color: string | null;
  minutes: number;
  /** Whole-percent share; the shares of a breakdown always add up to exactly 100. */
  share: number;
  archived: boolean;
}

/** Project title when known, otherwise a stable short label (like report-math's memberName). */
export function projectLabel(projectId: string, known?: { title?: string | null } | null): string {
  const title = known?.title?.trim();
  return title || `Project ${String(projectId).slice(0, 8)}`;
}

/**
 * Whole-number percentages of `values` that sum to exactly 100 (largest
 * remainder), so a printed breakdown never reads 99 % or 101 %. All-zero
 * input yields zeros.
 */
export function apportionPercent(values: readonly number[]): number[] {
  const clean = values.map(cleanMinutes);
  const total = clean.reduce((s, v) => s + v, 0);
  if (total <= 0) return clean.map(() => 0);
  const exact = clean.map((v) => (v / total) * 100);
  const floors = exact.map(Math.floor);
  let left = 100 - floors.reduce((s, v) => s + v, 0);
  const order = exact
    .map((v, i) => ({ i, rem: v - floors[i] }))
    .sort((a, b) => b.rem - a.rem || a.i - b.i);
  for (const { i } of order) {
    if (left <= 0) break;
    floors[i] += 1;
    left -= 1;
  }
  return floors;
}

/**
 * Hours per project with their share of the total, largest first. Zero-time
 * projects are left out; archived ones with time stay in (and say so), so the
 * shares always cover every tracked minute.
 */
export function projectBreakdown(
  rows: readonly ProjectMinutes[],
  lookup: (projectId: string) => { title?: string | null; color?: string | null; archived?: boolean | null } | null | undefined,
): ProjectShare[] {
  const acc = new Map<string, { minutes: number; title: string | null; archived: boolean }>();
  for (const r of rows) {
    if (!r || typeof r !== "object") continue;
    const id = typeof r.projectId === "string" && r.projectId ? r.projectId : null;
    if (!id) continue;
    const entry = acc.get(id) ?? { minutes: 0, title: null, archived: false };
    entry.minutes += cleanMinutes(r.minutes);
    if (!entry.title && typeof r.title === "string" && r.title.trim()) entry.title = r.title.trim();
    if (r.archived === true) entry.archived = true;
    acc.set(id, entry);
  }
  const list = [...acc.entries()]
    .map(([projectId, e]) => {
      const known = lookup(projectId);
      return {
        projectId,
        name: projectLabel(projectId, { title: known?.title?.trim() || e.title }),
        color: known?.color ?? null,
        minutes: Math.round(e.minutes),
        archived: known?.archived === true || e.archived,
      };
    })
    .filter((p) => p.minutes > 0)
    .sort((a, b) => b.minutes - a.minutes || a.name.localeCompare(b.name));
  const shares = apportionPercent(list.map((p) => p.minutes));
  return list.map((p, i) => ({ ...p, share: shares[i] }));
}

/* ----------------------------- Scope ----------------------------- */

/** "All projects · All members", or the chosen names. */
export function scopeLabel(projectName?: string | null, memberName?: string | null): string {
  return `${projectName?.trim() || "All projects"} · ${memberName?.trim() || "All members"}`;
}

/** Distinct members with tracked time. */
export function countMembers(rows: readonly { userId: unknown; minutes: unknown }[]): number {
  const ids = new Set<string>();
  for (const r of rows) {
    if (r && typeof r.userId === "string" && r.userId && cleanMinutes(r.minutes) > 0) ids.add(r.userId);
  }
  return ids.size;
}

/* ----------------------------- Aggregates ----------------------------- */

/**
 * One tracked block, reduced to what the totals need (Work Analytics' blocks
 * carry these fields too). Times are epoch milliseconds.
 */
export interface TrackedRow {
  userId: string;
  projectId: string;
  start: number;
  end: number;
  /** Block length in minutes. */
  minutes: number;
  /** 0–100; null when not measured. */
  activity: number | null;
  /** Minutes without input; null when not measured. */
  idleMinutes: number | null;
  keys: number;
  clicks: number;
}

export interface ReportTotals {
  minutes: number;
  sessions: number;
  /** Weighted by measured minutes; null when nothing was measured. */
  activity: number | null;
  idleMinutes: number | null;
  keys: number;
  clicks: number;
}

export interface DailyAggregate {
  day: string;
  minutes: number;
  sessions: number;
  activity: number | null;
  /** The minutes `activity` was measured over, when the source knows them (blocks do; the server's series doesn't say). */
  measuredMinutes?: number;
  idleMinutes: number | null;
  keys: number;
  clicks: number;
}

export interface MemberAggregate {
  userId: string;
  /** A name the source knows (the server's name or user name); null otherwise. */
  name: string | null;
  minutes: number;
  sessions: number;
  activity: number | null;
  idleMinutes: number | null;
}

export interface ProjectAggregate {
  projectId: string;
  title: string | null;
  archived: boolean;
  minutes: number;
  sessions: number;
  activity: number | null;
  idleMinutes: number | null;
}

export interface MemberDayAggregate {
  userId: string;
  name: string | null;
  day: string;
  minutes: number;
  sessions: number;
  idleMinutes: number | null;
  /** The day's first block start and last block end (epoch ms, UTC instants); null when unknown. */
  firstStart: number | null;
  lastEnd: number | null;
}

export interface MemberProjectAggregate {
  userId: string;
  projectId: string;
  minutes: number;
}

/**
 * The figures a period is drawn from: the /api/app/reporting/* payloads in a
 * live session, or the same figures computed from the demo blocks with the
 * backend's rules (see {@link aggregateTracked}) — one shape, so both modes
 * render through the same code.
 */
export interface ReportAggregates {
  totals: ReportTotals;
  daily: DailyAggregate[];
  members: MemberAggregate[];
  projects: ProjectAggregate[];
  memberDays: MemberDayAggregate[];
  /** Minutes per member per project; null when the source has no such breakdown (an older server). */
  memberProjects: MemberProjectAggregate[] | null;
}

interface Acc {
  minutes: number;
  sessions: number;
  weighted: number;
  measured: number;
  idle: number;
  idleKnown: boolean;
  keys: number;
  clicks: number;
}

const freshAcc = (): Acc => ({ minutes: 0, sessions: 0, weighted: 0, measured: 0, idle: 0, idleKnown: false, keys: 0, clicks: 0 });

function addTo(acc: Acc, row: TrackedRow): void {
  acc.minutes += row.minutes;
  acc.sessions += 1;
  acc.keys += row.keys;
  acc.clicks += row.clicks;
  if (row.activity !== null) {
    acc.weighted += row.minutes * row.activity;
    acc.measured += row.minutes;
  }
  if (row.idleMinutes !== null) {
    acc.idle += row.idleMinutes;
    acc.idleKnown = true;
  }
}

/** Two decimals, as the server rounds its figures. */
function round2(value: number): number {
  return Math.round(value * 100) / 100;
}

const accActivity = (acc: Acc): number | null => (acc.measured > 0 ? round2(acc.weighted / acc.measured) : null);
const accIdle = (acc: Acc): number | null => (acc.idleKnown ? round2(acc.idle) : null);
const byMinutes = <T extends { minutes: number }>(a: T, b: T) => b.minutes - a.minutes;

/**
 * The reporting figures of a set of blocks, by the backend's rules: each
 * block counts wholly toward the day it STARTED on (`dayOf` reads that day in
 * the reporting zone), activity is weighted by measured minutes (unmeasured
 * blocks left out), and idle time sums the blocks that measured it.
 */
export function aggregateTracked(rows: readonly TrackedRow[], dayOf: (ms: number) => string): ReportAggregates {
  const total = freshAcc();
  const daily = new Map<string, Acc>();
  const members = new Map<string, Acc>();
  const projects = new Map<string, Acc>();
  const memberDays = new Map<string, { userId: string; day: string; acc: Acc; first: number; last: number }>();
  const memberProjects = new Map<string, MemberProjectAggregate>();
  const into = <K,>(map: Map<K, Acc>, key: K): Acc => {
    let acc = map.get(key);
    if (!acc) {
      acc = freshAcc();
      map.set(key, acc);
    }
    return acc;
  };
  for (const row of rows) {
    if (!row || !(row.minutes > 0)) continue;
    const day = dayOf(row.start);
    addTo(total, row);
    addTo(into(daily, day), row);
    addTo(into(members, row.userId), row);
    addTo(into(projects, row.projectId), row);
    const md = `${row.userId}\u0000${day}`;
    let entry = memberDays.get(md);
    if (!entry) {
      entry = { userId: row.userId, day, acc: freshAcc(), first: row.start, last: row.end };
      memberDays.set(md, entry);
    }
    addTo(entry.acc, row);
    entry.first = Math.min(entry.first, row.start);
    entry.last = Math.max(entry.last, row.end);
    const mp = `${row.userId}\u0000${row.projectId}`;
    const pair = memberProjects.get(mp) ?? { userId: row.userId, projectId: row.projectId, minutes: 0 };
    pair.minutes += row.minutes;
    memberProjects.set(mp, pair);
  }
  return {
    totals: {
      minutes: round2(total.minutes),
      sessions: total.sessions,
      activity: accActivity(total),
      idleMinutes: accIdle(total),
      keys: Math.round(total.keys),
      clicks: Math.round(total.clicks),
    },
    daily: [...daily.entries()]
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
      .map(([day, acc]) => ({
        day,
        minutes: round2(acc.minutes),
        sessions: acc.sessions,
        activity: accActivity(acc),
        measuredMinutes: round2(acc.measured),
        idleMinutes: accIdle(acc),
        keys: Math.round(acc.keys),
        clicks: Math.round(acc.clicks),
      })),
    members: [...members.entries()]
      .map(([userId, acc]) => ({
        userId,
        name: null,
        minutes: round2(acc.minutes),
        sessions: acc.sessions,
        activity: accActivity(acc),
        idleMinutes: accIdle(acc),
      }))
      .sort((a, b) => byMinutes(a, b) || a.userId.localeCompare(b.userId)),
    projects: [...projects.entries()]
      .map(([projectId, acc]) => ({
        projectId,
        title: null,
        archived: false,
        minutes: round2(acc.minutes),
        sessions: acc.sessions,
        activity: accActivity(acc),
        idleMinutes: accIdle(acc),
      }))
      .sort((a, b) => byMinutes(a, b) || a.projectId.localeCompare(b.projectId)),
    memberDays: [...memberDays.values()]
      .map((e) => ({
        userId: e.userId,
        name: null,
        day: e.day,
        minutes: round2(e.acc.minutes),
        sessions: e.acc.sessions,
        idleMinutes: accIdle(e.acc),
        firstStart: e.first,
        lastEnd: e.last,
      }))
      .sort((a, b) => (a.day < b.day ? -1 : a.day > b.day ? 1 : byMinutes(a, b) || a.userId.localeCompare(b.userId))),
    memberProjects: [...memberProjects.values()]
      .map((p) => ({ ...p, minutes: round2(p.minutes) }))
      .sort((a, b) => byMinutes(a, b) || a.userId.localeCompare(b.userId) || a.projectId.localeCompare(b.projectId)),
  };
}

/** A per-minute timeline bucket, as far as idle time is concerned. */
interface TimelineBucket {
  minute: number;
  active: boolean;
}

/**
 * A block's idle minutes by the agents' own rule (contract §1): the idle
 * seconds it reported; else the inactive buckets of its per-minute timeline
 * (buckets are whole minutes, the last absorbing the remainder — the server
 * derives it the same way); else, with `fromActivity`, the share of the block
 * without input (100 − activity %), which is what an agent reports for such a
 * block. Null when there is nothing to go on. Never more than the block.
 */
export function blockIdleMinutes(input: {
  minutes: number;
  idleSeconds?: unknown;
  timeline?: readonly TimelineBucket[] | null;
  activity: number | null;
  fromActivity?: boolean;
}): number | null {
  const minutes = cleanMinutes(input.minutes);
  const reported = optionalMinutes(input.idleSeconds);
  if (reported !== null) return Math.min(minutes, reported / 60);
  const timeline = Array.isArray(input.timeline) ? input.timeline : [];
  if (timeline.length) {
    const buckets = Math.max(1, Math.round(minutes));
    const seen = new Set<number>();
    let seconds = 0;
    for (const bucket of timeline) {
      if (!bucket || bucket.active !== false || !Number.isInteger(bucket.minute) || bucket.minute < 0 || bucket.minute >= buckets) continue;
      if (seen.has(bucket.minute)) continue;
      seen.add(bucket.minute);
      seconds += bucket.minute < buckets - 1 ? 60 : Math.max(0, minutes * 60 - 60 * (buckets - 1));
    }
    return Math.min(minutes, seconds / 60);
  }
  if (input.fromActivity && input.activity !== null) return (minutes * (100 - input.activity)) / 100;
  return null;
}

/** Fields a demo or tenant row may carry beyond the core of lib/types' Activity (newer datasets fill them in). */
interface ActivityExtras {
  idleSeconds?: number | null;
  /** False when the row's input was not measured (its productivity then reads 0). */
  activityMeasured?: boolean;
}

function nonNegative(value: unknown): number {
  const n = toNumber(value);
  return Number.isFinite(n) && n > 0 ? n : 0;
}

/**
 * A demo or tenant activity as a tracked row; null when its times are
 * unusable. Its idle time follows {@link blockIdleMinutes}, falling back to the
 * activity % the demo dataset carries.
 */
export function trackedRowFromActivity(activity: Activity): TrackedRow | null {
  if (!activity || typeof activity !== "object") return null;
  const start = Date.parse(activity.startedAt);
  const end = Date.parse(activity.endedAt);
  if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) return null;
  const minutes = (end - start) / MINUTE_MS;
  const extras = activity as Activity & ActivityExtras;
  // A row flagged as unmeasured carries productivity 0: that is "not measured", never 0 %.
  const measured = extras.activityMeasured === false ? null : optionalPercent((activity as { productivity?: unknown }).productivity);
  return {
    userId: String(activity.userId ?? ""),
    projectId: String(activity.projectId ?? ""),
    start,
    end,
    minutes,
    activity: measured,
    idleMinutes: blockIdleMinutes({
      minutes,
      idleSeconds: extras.idleSeconds,
      timeline: activity.timeline,
      activity: measured,
      fromActivity: true,
    }),
    keys: nonNegative(activity.keyboardHits),
    clicks: nonNegative(activity.mouseClicks),
  };
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}

function rowsOf(value: unknown): Record<string, unknown>[] {
  return Array.isArray(value) ? value.filter((v): v is Record<string, unknown> => !!v && typeof v === "object") : [];
}

function idOf(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function textOf(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function instantOf(value: unknown): number | null {
  const iso = normalizeUtcIso(value);
  return iso ? Date.parse(iso) : null;
}

/**
 * GET /api/app/reporting/summary: the period's totals, and the rows per
 * member, per project and (newer servers) per member and project. Tolerant of
 * missing or malformed parts; fields an older server doesn't send read as
 * unknown (null), never as zero.
 */
export function parseSummary(payload: unknown): Pick<ReportAggregates, "totals" | "members" | "projects" | "memberProjects"> {
  const s = record(payload);
  return {
    totals: {
      minutes: cleanMinutes(s.totalTrackedMinutes),
      sessions: cleanCount(s.totalActivities),
      activity: optionalPercent(s.averageProductivity),
      idleMinutes: optionalMinutes(s.totalIdleMinutes),
      keys: cleanCount(s.totalKeyboardHits),
      clicks: cleanCount(s.totalMouseClicks),
    },
    members: rowsOf(s.perUser).flatMap((u) => {
      const userId = idOf(u.userId);
      if (!userId) return [];
      return [
        {
          userId,
          name: textOf(u.name) ?? textOf(u.userName),
          minutes: cleanMinutes(u.trackedMinutes),
          sessions: cleanCount(u.activityCount),
          activity: optionalPercent(u.averageProductivity),
          idleMinutes: optionalMinutes(u.idleMinutes),
        },
      ];
    }),
    projects: rowsOf(s.perProject).flatMap((p) => {
      const projectId = idOf(p.projectId);
      if (!projectId) return [];
      return [
        {
          projectId,
          title: textOf(p.projectTitle),
          archived: p.isArchived === true,
          minutes: cleanMinutes(p.trackedMinutes),
          sessions: cleanCount(p.activityCount),
          activity: optionalPercent(p.averageProductivity),
          idleMinutes: optionalMinutes(p.idleMinutes),
        },
      ];
    }),
    memberProjects: Array.isArray(s.perUserProject)
      ? rowsOf(s.perUserProject).flatMap((r) => {
          const userId = idOf(r.userId);
          const projectId = idOf(r.projectId);
          return userId && projectId ? [{ userId, projectId, minutes: cleanMinutes(r.trackedMinutes) }] : [];
        })
      : null,
  };
}

/** GET /api/app/reporting/daily-series: one row per day of the requested zone. */
export function parseDailySeries(payload: unknown): DailyAggregate[] {
  return rowsOf(payload).flatMap((d) => {
    const day = utcDayKey(d.date);
    if (!day) return [];
    return [
      {
        day,
        minutes: cleanMinutes(d.trackedMinutes),
        sessions: cleanCount(d.activityCount),
        activity: optionalPercent(d.averageProductivity),
        idleMinutes: optionalMinutes(d.idleMinutes),
        keys: cleanCount(d.keyboardHits),
        clicks: cleanCount(d.mouseClicks),
      },
    ];
  });
}

/** GET /api/app/reporting/user-daily-series: one row per member per day, with the day's first start and last end. */
export function parseUserDailySeries(payload: unknown): MemberDayAggregate[] {
  return rowsOf(payload).flatMap((d) => {
    const userId = idOf(d.userId);
    const day = utcDayKey(d.date);
    if (!userId || !day) return [];
    return [
      {
        userId,
        name: textOf(d.userName),
        day,
        minutes: cleanMinutes(d.trackedMinutes),
        sessions: cleanCount(d.activityCount),
        idleMinutes: optionalMinutes(d.idleMinutes),
        firstStart: instantOf(d.firstStartedAt),
        lastEnd: instantOf(d.lastEndedAt),
      },
    ];
  });
}

/** The live aggregates of a period: summary, daily series and per-member series (any of them may be missing). */
export function aggregatesFromLive(summary: unknown, series: unknown, userDays: unknown): ReportAggregates {
  return { ...parseSummary(summary), daily: parseDailySeries(series), memberDays: parseUserDailySeries(userDays) };
}

/* ----------------------------- Sources ----------------------------- */

export interface WorkReportInputs {
  daily: DailyPoint[];
  memberDays: MemberDayPoint[];
  projects: ProjectMinutes[];
  members: { userId: unknown; minutes: unknown }[];
  /**
   * The period's activity as its source computed it (weighted by measured
   * minutes across the whole period; null = nothing measured). Absent: the
   * daily log derives it from its days.
   */
  activity?: number | null;
}

/** The Work Report's inputs from a period's aggregates (live or demo — see {@link ReportAggregates}). */
export function inputsFromAggregates(agg: ReportAggregates): WorkReportInputs {
  return {
    daily: agg.daily.map((d) => ({
      date: d.day,
      minutes: d.minutes,
      activity: d.activity,
      measuredMinutes: d.measuredMinutes,
      idleMinutes: d.idleMinutes,
      sessions: d.sessions,
    })),
    memberDays: agg.memberDays.map((d) => ({ userId: d.userId, date: d.day, minutes: d.minutes, name: d.name, idleMinutes: d.idleMinutes })),
    projects: agg.projects.map((p) => ({ projectId: p.projectId, minutes: p.minutes, title: p.title, archived: p.archived })),
    members: agg.members.map((m) => ({ userId: m.userId, minutes: m.minutes })),
    activity: agg.totals.activity,
  };
}

/**
 * Demo mode: the seeded activities, each booked wholly to the day it started
 * on in `timeZone` (the backend's rule).
 */
export function inputsFromActivities(list: readonly Activity[], timeZone = "UTC"): WorkReportInputs {
  const rows = list.map(trackedRowFromActivity).filter((r): r is TrackedRow => r !== null);
  return inputsFromAggregates(aggregateTracked(rows, (ms) => zonedDayKey(ms, timeZone)));
}

/**
 * Live mode: the /reporting/summary, /daily-series and /user-daily-series
 * payloads, tolerant of missing or malformed parts (they read as empty).
 */
export function inputsFromLive(summary: unknown, series: unknown, userDays: unknown): WorkReportInputs {
  return inputsFromAggregates(aggregatesFromLive(summary, series, userDays));
}

/** One member's day: hours, idle time, and the first start / last end (UTC instants). */
export interface MemberDay {
  userId: string;
  name: string;
  /** Whole minutes. */
  minutes: number;
  idleMinutes: number | null;
  sessions: number;
  firstStart: number | null;
  lastEnd: number | null;
}

/** Who worked on `day`, most time first (members without whole minutes that day are left out). */
export function membersOnDay(
  memberDays: readonly MemberDayAggregate[],
  day: string,
  nameOf: (userId: string, hint: string | null) => string,
): MemberDay[] {
  const merged = new Map<string, MemberDay>();
  for (const d of memberDays) {
    if (d.day !== day) continue;
    const e = merged.get(d.userId) ?? {
      userId: d.userId,
      name: nameOf(d.userId, d.name),
      minutes: 0,
      idleMinutes: null,
      sessions: 0,
      firstStart: null,
      lastEnd: null,
    };
    e.minutes += d.minutes;
    e.sessions += d.sessions;
    if (d.idleMinutes !== null) e.idleMinutes = (e.idleMinutes ?? 0) + d.idleMinutes;
    if (d.firstStart !== null) e.firstStart = e.firstStart === null ? d.firstStart : Math.min(e.firstStart, d.firstStart);
    if (d.lastEnd !== null) e.lastEnd = e.lastEnd === null ? d.lastEnd : Math.max(e.lastEnd, d.lastEnd);
    merged.set(d.userId, e);
  }
  return [...merged.values()]
    .map((m) => ({
      ...m,
      minutes: Math.round(m.minutes),
      idleMinutes: m.idleMinutes === null ? null : Math.min(Math.round(m.minutes), Math.round(m.idleMinutes)),
    }))
    .filter((m) => m.minutes > 0)
    .sort((a, b) => b.minutes - a.minutes || a.name.localeCompare(b.name));
}

/* ----------------------------- What was worked on ----------------------------- */

/** One block's part in the notes breakdown: who, which day of the reporting zone, how long, and its task note. */
export interface NoteBlock {
  userId: string;
  day: string;
  minutes: number;
  note: string | null;
}

/** How blocks without a task note are listed. */
export const NO_NOTE = "No note";

export interface NoteRow {
  /** The grouping key: the note as the server stores it, lower-cased; "" for blocks without a note. */
  key: string;
  /** The note as written (its most-tracked spelling); null for blocks without one. */
  note: string | null;
  /** Whole minutes. */
  minutes: number;
  /** Whole-percent share of the notes' time; the shares add up to 100. */
  share: number;
  sessions: number;
  /** Days the note was worked on, oldest first. */
  days: string[];
  /** Members who worked on it, most time first. */
  members: string[];
}

export interface NoteEntry {
  key: string;
  note: string | null;
  /** Whole minutes. */
  minutes: number;
  sessions: number;
}

interface NoteGroup {
  minutes: number;
  sessions: number;
  spellings: Map<string, number>;
  days: Set<string>;
  members: Map<string, number>;
}

/** The note as the server stores it (trimmed, whitespace collapsed), and its grouping key. */
function noteKey(note: string | null): { key: string; text: string | null } {
  const text = normalizeActivityNote(note);
  return { key: text ? text.toLocaleLowerCase() : "", text };
}

function topSpelling(spellings: Map<string, number>): string | null {
  let best: string | null = null;
  let most = -1;
  for (const [text, minutes] of spellings) {
    if (minutes > most || (minutes === most && best !== null && text < best)) {
      best = text;
      most = minutes;
    }
  }
  return best;
}

function groupNotes(blocks: readonly NoteBlock[]): Map<string, NoteGroup> {
  const groups = new Map<string, NoteGroup>();
  for (const b of blocks) {
    if (!b || typeof b !== "object") continue;
    const minutes = cleanMinutes(b.minutes);
    if (minutes <= 0) continue;
    const { key, text } = noteKey(b.note);
    const g = groups.get(key) ?? { minutes: 0, sessions: 0, spellings: new Map(), days: new Set(), members: new Map() };
    g.minutes += minutes;
    g.sessions += 1;
    if (typeof b.day === "string" && b.day) g.days.add(b.day);
    g.members.set(b.userId, (g.members.get(b.userId) ?? 0) + minutes);
    if (text) g.spellings.set(text, (g.spellings.get(text) ?? 0) + minutes);
    groups.set(key, g);
  }
  return groups;
}

/** Notes first by time (most first), then by text; blocks without a note always last. */
function noteOrder(a: { key: string; minutes: number; note: string | null }, b: { key: string; minutes: number; note: string | null }): number {
  return (a.key === "" ? 1 : 0) - (b.key === "" ? 1 : 0) || b.minutes - a.minutes || (a.note ?? "").localeCompare(b.note ?? "");
}

function entriesOf(groups: Map<string, NoteGroup>): NoteEntry[] {
  return [...groups.entries()]
    .map(([key, g]) => ({ key, note: key ? topSpelling(g.spellings) : null, minutes: Math.round(g.minutes), sessions: g.sessions }))
    .filter((e) => e.minutes > 0)
    .sort(noteOrder);
}

/**
 * What was worked on: time per task note over the period, most first, with
 * the days and members behind each. Notes are grouped as the server stores
 * them, ignoring letter case; blocks without a note are listed last.
 */
export function noteBreakdown(blocks: readonly NoteBlock[]): NoteRow[] {
  const groups = groupNotes(blocks);
  const list = [...groups.entries()]
    .map(([key, g]) => ({
      key,
      note: key ? topSpelling(g.spellings) : null,
      minutes: Math.round(g.minutes),
      sessions: g.sessions,
      days: [...g.days].sort(),
      members: [...g.members.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([id]) => id),
    }))
    .filter((r) => r.minutes > 0)
    .sort(noteOrder);
  const shares = apportionPercent(list.map((r) => r.minutes));
  return list.map((r, i) => ({ ...r, share: shares[i] }));
}

export interface NoteSection {
  /** A day ("YYYY-MM-DD") or a member's id. */
  id: string;
  /** Whole minutes: the sum of the section's notes. */
  minutes: number;
  notes: NoteEntry[];
}

function sectionsBy(blocks: readonly NoteBlock[], keyOf: (b: NoteBlock) => string): NoteSection[] {
  const buckets = new Map<string, NoteBlock[]>();
  for (const b of blocks) {
    if (!b || typeof b !== "object") continue;
    const id = keyOf(b);
    if (!id) continue;
    const list = buckets.get(id) ?? [];
    list.push(b);
    buckets.set(id, list);
  }
  return [...buckets.entries()]
    .map(([id, list]) => {
      const notes = entriesOf(groupNotes(list));
      return { id, minutes: notes.reduce((s, n) => s + n.minutes, 0), notes };
    })
    .filter((s) => s.minutes > 0);
}

/** The notes of each day with time, oldest day first. */
export function notesByDay(blocks: readonly NoteBlock[]): NoteSection[] {
  return sectionsBy(blocks, (b) => b.day).sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
}

/** The notes of each member, most time first. */
export function notesByMember(blocks: readonly NoteBlock[]): NoteSection[] {
  return sectionsBy(blocks, (b) => b.userId).sort((a, b) => b.minutes - a.minutes || a.id.localeCompare(b.id));
}

export const NOTES_CSV_HEADERS = ["Date", "Weekday", "Member", "Note", "Hours (decimal)", "Hours (h:mm)", "Sessions"];

/** The notes as CSV rows — one per day, member and note — closed by a Total row. */
export function notesCsvRows(blocks: readonly NoteBlock[], nameOf: (userId: string) => string): (string | number)[][] {
  const rows: { day: string; name: string; note: string | null; minutes: number; sessions: number }[] = [];
  for (const day of notesByDay(blocks)) {
    const ofDay = blocks.filter((b) => b && b.day === day.id);
    for (const member of notesByMember(ofDay)) {
      const name = nameOf(member.id);
      for (const n of member.notes) rows.push({ day: day.id, name, note: n.note, minutes: n.minutes, sessions: n.sessions });
    }
  }
  rows.sort((a, b) => (a.day < b.day ? -1 : a.day > b.day ? 1 : a.name.localeCompare(b.name)));
  const body: (string | number)[][] = rows.map((r) => [
    r.day,
    weekdayShort(r.day),
    r.name,
    r.note ?? "",
    decimalHours(r.minutes),
    formatHM(r.minutes),
    r.sessions,
  ]);
  const minutes = rows.map((r) => r.minutes);
  body.push([
    "Total",
    "",
    "",
    "",
    sumDecimalHours(minutes),
    formatHM(minutes.reduce((s, m) => s + m, 0)),
    rows.reduce((s, r) => s + r.sessions, 0),
  ]);
  return body;
}

/* ----------------------------- Highlights ----------------------------- */

export type HighlightKind = "busiest" | "project" | "member" | "fullDays" | "weekend";

export interface Highlight {
  kind: HighlightKind;
  label: string;
  value: string;
  detail: string;
}

/**
 * The plain-language takeaways shown beside the chart: the busiest day, the
 * main project, the top contributor (team) or full days (one person), and
 * weekend work. Nothing is said about a period without tracked time.
 */
export function buildHighlights(input: {
  summary: WorkSummary;
  projects: readonly ProjectShare[];
  matrix: TimesheetMatrix;
  team: boolean;
  /** Saturdays and Sundays in the period. */
  weekendDays: number;
  fmt: (minutes: number) => string;
}): Highlight[] {
  const { summary, projects, matrix, team, weekendDays, fmt } = input;
  if (summary.totalMinutes <= 0) return [];
  const out: Highlight[] = [];
  const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

  if (summary.busiest) {
    out.push({
      kind: "busiest",
      label: "Busiest day",
      value: formatDayLabel(summary.busiest.date),
      detail: `${fmt(summary.busiest.minutes)} tracked`,
    });
  }
  const top = projects[0];
  if (top) {
    out.push({
      kind: "project",
      label: projects.length > 1 ? "Main project" : "Project",
      value: top.name,
      detail: `${top.share}% of hours · ${fmt(top.minutes)}`,
    });
  }
  const lead = matrix.rows[0];
  if (team && lead) {
    out.push({
      kind: "member",
      label: "Most hours",
      value: lead.name,
      detail: `${fmt(lead.total)} over ${plural(lead.workedDays, "day")}`,
    });
  } else if (!team && summary.workedDays > 0) {
    out.push({
      kind: "fullDays",
      label: "Full days (8h+)",
      value: `${summary.targetDays} of ${summary.workedDays}`,
      detail: summary.targetDays ? `worked ${summary.workedDays === 1 ? "day" : "days"} reached 8 hours` : "no day reached 8 hours",
    });
  }
  if (weekendDays > 0) {
    out.push(
      summary.weekendMinutes > 0
        ? {
            kind: "weekend",
            label: "Weekend work",
            value: fmt(summary.weekendMinutes),
            detail: `${Math.round((summary.weekendMinutes / summary.totalMinutes) * 100)}% of all hours`,
          }
        : { kind: "weekend", label: "Weekend work", value: "None", detail: "no time tracked on weekends" },
    );
  }
  return out;
}

/** Idle time as a whole-percent share of the tracked time (0 without time). */
export function idleShare(idleMinutes: number, totalMinutes: number): number {
  const total = cleanMinutes(totalMinutes);
  return total > 0 ? Math.min(100, Math.round((cleanMinutes(idleMinutes) / total) * 100)) : 0;
}

/** Notes named in the copied summary, at most. */
const SUMMARY_NOTES = 5;

/**
 * A plain-text digest of the report for pasting into an email or chat
 * ("Copy summary"). Uses the viewer's hours format.
 */
export function summaryText(input: {
  workspace: string;
  period: string;
  scope: string;
  summary: WorkSummary;
  projects: readonly ProjectShare[];
  change: Change | null;
  comparison: string;
  perMember: { minutes: number; memberDays: number } | null;
  fmt: (minutes: number) => string;
  /** What was worked on (task notes); notes without text are left out. */
  notes?: readonly NoteRow[];
}): string {
  const { workspace, period, scope, summary, projects, change, comparison, perMember, fmt } = input;
  const lines = [`Work Report — ${workspace}`, `${period} · ${scope}`, ""];
  if (summary.totalMinutes <= 0) {
    lines.push("No time was tracked in this period.");
    return lines.join("\n");
  }
  lines.push(`Total worked: ${fmt(summary.totalMinutes)} on ${summary.workedDays} of ${summary.dayCount} days`);
  lines.push(
    perMember
      ? `Average: ${fmt(perMember.minutes)} per member per day worked`
      : `Average: ${fmt(summary.averagePerWorkedDay)} per day worked`,
  );
  lines.push(
    summary.averageActivity === null
      ? "Activity: not measured"
      : `Activity: ${summary.averageActivity}% (${activityLevelLabel[summary.level]})`,
  );
  if (summary.idleMinutes !== null) {
    lines.push(`Idle: ${fmt(summary.idleMinutes)} (${idleShare(summary.idleMinutes, summary.totalMinutes)}% of tracked time)`);
  }
  if (summary.busiest) lines.push(`Busiest day: ${formatDayLabel(summary.busiest.date)} (${fmt(summary.busiest.minutes)})`);
  if (change) {
    const sign = change.percent > 0 ? "+" : "";
    lines.push(`Change: ${sign}${change.percent}% ${comparison}`);
  }
  if (projects.length) {
    lines.push("", "By project:");
    for (const p of projects) lines.push(`• ${p.name} — ${fmt(p.minutes)} (${p.share}%)`);
  }
  const notes = (input.notes ?? []).filter((n) => n.note).slice(0, SUMMARY_NOTES);
  if (notes.length) {
    lines.push("", "Worked on:");
    for (const n of notes) lines.push(`• ${n.note} — ${fmt(n.minutes)}`);
  }
  return lines.join("\n");
}

/* ----------------------------- CSV ----------------------------- */

export const DAILY_LOG_HEADERS = [
  "Date",
  "Weekday",
  "Hours (decimal)",
  "Hours (h:mm)",
  "Activity %",
  "Sessions",
  "Idle (decimal)",
  "Idle (h:mm)",
];

/**
 * The daily log as CSV rows, closed by a Total row whose decimal hours are the
 * sum of the rounded rows above it. Activity is blank on days without time or
 * without a measurement; idle is blank where it is unknown.
 */
export function dailyLogCsvRows(rows: readonly DayRow[], summary: WorkSummary): (string | number)[][] {
  const body: (string | number)[][] = rows.map((r) => [
    r.date,
    r.weekday,
    decimalHours(r.minutes),
    formatHM(r.minutes),
    r.minutes > 0 && r.activity !== null ? r.activity : "",
    r.sessions,
    r.idleMinutes === null ? "" : decimalHours(r.idleMinutes),
    r.idleMinutes === null ? "" : formatHM(r.idleMinutes),
  ]);
  const idle = rows.filter((r) => r.idleMinutes !== null).map((r) => r.idleMinutes ?? 0);
  body.push([
    "Total",
    "",
    sumDecimalHours(rows.map((r) => r.minutes)),
    formatHM(summary.totalMinutes),
    summary.totalMinutes > 0 && summary.averageActivity !== null ? summary.averageActivity : "",
    summary.sessions,
    summary.idleMinutes === null ? "" : sumDecimalHours(idle),
    summary.idleMinutes === null ? "" : formatHM(summary.idleMinutes),
  ]);
  return body;
}

/** The team timesheet as CSV: decimal hours per member per day, with totals that add up the rounded cells. */
export function matrixCsv(matrix: TimesheetMatrix): { headers: string[]; rows: (string | number)[][] } {
  const showIdle = matrix.idleTotal !== null;
  const headers = [
    "Member",
    ...matrix.days.map((d) => `${d} (${weekdayShort(d)})`),
    "Total (hours)",
    ...(showIdle ? ["Idle (hours)"] : []),
  ];
  const rows: (string | number)[][] = matrix.rows.map((r) => [
    r.name,
    ...r.cells.map(decimalHours),
    sumDecimalHours(r.cells),
    ...(showIdle ? [r.idleMinutes === null ? "" : decimalHours(r.idleMinutes)] : []),
  ]);
  rows.push([
    "Total",
    ...matrix.days.map((_, i) => sumDecimalHours(matrix.rows.map((r) => r.cells[i]))),
    sumDecimalHours(matrix.rows.flatMap((r) => r.cells)),
    ...(showIdle ? [sumDecimalHours(matrix.rows.filter((r) => r.idleMinutes !== null).map((r) => r.idleMinutes ?? 0))] : []),
  ]);
  return { headers, rows };
}
