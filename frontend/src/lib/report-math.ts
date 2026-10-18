import type { Role } from "./types";
import { ALL } from "./api-contract";
import { appCategory, appColor, appKey, friendlyAppName, isClassifiedApp, type AppCategory } from "./apps";
import {
  addZonedDays,
  countDays,
  formatDayRange,
  isDayKey,
  startOfZonedDay,
  zonedDayKey,
  zonedDayRange,
  zonedPeriod,
  type ZonedPeriodPreset,
} from "./time-zone";

// Dependencies stay leaf modules (types, api-contract, apps, time-zone), so
// any screen can use the report maths without an import cycle.

/* ============================================================================
 * REPORT MATH — pure helpers shared by every report screen.
 *
 * The reports used to carry their own copy of the range presets, their own
 * query strings and their own percentage/money arithmetic (with a
 * divide-by-zero "NaN%" and a payroll that multiplied a member's HIGHEST
 * project rate by ALL of their hours). One definition lives here, as pure
 * functions of their inputs (the clock and the reporting zone are
 * parameters), so it is unit-tested without React or a backend.
 *
 * Shapes: the live reports read the /api/app/reporting/* payloads through the
 * tolerant readers below (a field an older server doesn't send reads as null,
 * never as an invented 0), and the demo reports derive the SAME shapes from
 * the seeded activity — so each screen renders one shape in both modes.
 *
 * Definitions (contract §1, §6, §8), fixed here and nowhere else:
 *  - "Activity %" is the share of a block's minutes with keyboard or mouse
 *    input; blocks whose input wasn't measured are left out of averages.
 *  - "Active time" is tracked time × activity % (keyboard/mouse time);
 *    "idle time" is the minutes without input; the rest of the tracked time
 *    is "other tracked".
 *  - "Productive" is an APP CATEGORY (lib/apps), never an activity score.
 *  - A day is a calendar day of the reporting zone; a period is whole days.
 * ========================================================================== */

/** The "no filter" value of the report dropdowns (defined with the API contract). */
export { ALL };

/**
 * Query string for the /api/app/reporting/* endpoints. A dropdown left on
 * "all" (or empty) sends no filter; a real selection is applied server-side.
 */
export function reportQuery(params: { from: Date; to: Date; projectId?: string | null; userId?: string | null }): string {
  const qs = new URLSearchParams({ From: params.from.toISOString(), To: params.to.toISOString() });
  if (params.projectId && params.projectId !== ALL) qs.set("ProjectId", params.projectId);
  if (params.userId && params.userId !== ALL) qs.set("UserId", params.userId);
  return qs.toString();
}

/** A list payload, or [] for anything that is not an array. */
export function asArray<T>(value: unknown): T[] {
  return Array.isArray(value) ? (value as T[]) : [];
}

/** `part` as a whole-number percentage of `whole`; 0 (never NaN/Infinity) when there is nothing to divide by. */
export function safePercent(part: number, whole: number): number {
  if (!Number.isFinite(part) || !Number.isFinite(whole) || whole <= 0) return 0;
  return Math.round((part / whole) * 100);
}

/** A number from a payload field (numeric strings included); NaN for anything else. */
function numeric(value: unknown): number {
  if (typeof value === "number") return value;
  if (typeof value === "string" && value.trim() !== "") return Number(value);
  return Number.NaN;
}

/** A non-negative finite number of minutes (anything else counts as 0). */
function minutesOf(value: unknown): number {
  const n = numeric(value);
  return Number.isFinite(n) && n > 0 ? n : 0;
}

/** A field the server may not send: null when absent or unreadable, otherwise clean minutes. */
function optionalMinutes(value: unknown): number | null {
  if (value === null || value === undefined) return null;
  const n = numeric(value);
  return Number.isFinite(n) ? Math.max(0, n) : null;
}

/** A 0–100 score, or null when it was not measured (null, absent, unreadable). */
function optionalPercent(value: unknown): number | null {
  if (value === null || value === undefined) return null;
  const n = numeric(value);
  return Number.isFinite(n) ? Math.min(100, Math.max(0, n)) : null;
}

/** A non-negative whole count. */
function countOf(value: unknown): number {
  const n = numeric(value);
  return Number.isFinite(n) && n > 0 ? Math.round(n) : 0;
}

/** A trimmed non-empty string, or null. */
function textOf(value: unknown): string | null {
  return typeof value === "string" && value.trim() !== "" ? value.trim() : null;
}

/** Records of a list payload (anything that isn't an object is skipped). */
function recordsOf(value: unknown): Record<string, unknown>[] {
  return Array.isArray(value) ? value.filter((v): v is Record<string, unknown> => !!v && typeof v === "object") : [];
}

/** Roster name when known, otherwise a stable short label (never a demo person). */
export function memberName(userId: string, known?: { name?: string | null } | null): string {
  const name = known?.name?.trim();
  return name || `Member ${String(userId).slice(0, 8)}`;
}

/* ----------------------------- Periods ----------------------------- */

/** The periods a report steps through ("week" is Monday–Sunday). */
export type ReportPeriodPreset = ZonedPeriodPreset;

/** The longest period the backend accepts, in days (GetReportSummaryInput.MaxRangeDays). */
export const MAX_PERIOD_DAYS = 366;

const MONTH_NAMES = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
] as const;

/** "This week", "Last month", "July 2026", "30-day period"… for a preset `offset` periods back. */
export function periodTitleFor(preset: ReportPeriodPreset, offset: number, fromKey: string): string {
  const back = -Math.min(0, Math.trunc(Number.isFinite(offset) ? offset : 0));
  switch (preset) {
    case "week":
      return back === 0 ? "This week" : back === 1 ? "Last week" : `${back} weeks ago`;
    case "30d":
      return back === 0 ? "Last 30 days" : "30-day period";
    case "month": {
      if (back === 0) return "This month";
      if (back === 1) return "Last month";
      const month = MONTH_NAMES[Number(fromKey.slice(5, 7)) - 1];
      return month ? `${month} ${Number(fromKey.slice(0, 4))}` : "Month";
    }
    default:
      return back === 0 ? "Last 7 days" : "7-day period";
  }
}

/** A report's period: whole calendar days of the reporting zone. */
export interface ReportPeriod {
  /** Every day of the period, oldest first ("YYYY-MM-DD"). */
  days: string[];
  /** Query bounds: the first day's start … the start of the day after the last (UTC instants). */
  from: Date;
  to: Date;
  fromKey: string;
  toKey: string;
  /** Today in the reporting zone. */
  today: string;
  /** "This week", "Last month"… — or a custom range's dates. */
  title: string;
  /** The period's dates: "29 Sep – 5 Oct 2026". */
  label: string;
  /** Days the viewer picked themselves (no stepping). */
  custom: boolean;
}

/**
 * The period a report shows: a preset `offset` periods back (0 = the current
 * one, which runs up to today), or a custom from/to pair of day keys. A
 * custom range longer than the server's limit keeps its most recent days; an
 * unusable one falls back to the last 7 days. Exact across DST (time-zone.ts).
 */
export function resolveReportPeriod(input: {
  preset: ReportPeriodPreset | "custom";
  offset?: number;
  range?: { from: string; to: string } | null;
  now: Date | number;
  timeZone: string;
}): ReportPeriod {
  const today = zonedDayKey(input.now, input.timeZone);
  const range = input.range;
  if (input.preset === "custom" && range && isDayKey(range.from) && isDayKey(range.to) && range.from <= range.to) {
    const fromKey = countDays(range.from, range.to) > MAX_PERIOD_DAYS ? addZonedDays(range.to, -(MAX_PERIOD_DAYS - 1)) : range.from;
    const span = zonedDayRange(fromKey, range.to, input.timeZone);
    const label = formatDayRange(fromKey, range.to);
    return { ...span, fromKey, toKey: range.to, today, title: label, label, custom: true };
  }
  const preset: ReportPeriodPreset = input.preset === "custom" ? "7d" : input.preset;
  const raw = input.offset ?? 0;
  const offset = Math.min(0, Math.trunc(Number.isFinite(raw) ? raw : 0));
  const p = zonedPeriod(preset, input.now, input.timeZone, offset);
  return {
    days: p.days,
    from: p.from,
    to: p.to,
    fromKey: p.fromKey,
    toKey: p.toKey,
    today,
    title: periodTitleFor(preset, offset, p.fromKey),
    label: formatDayRange(p.fromKey, p.toKey),
    custom: false,
  };
}

/** The period's days that have begun (today included): a day still ahead has nothing to report. */
export function elapsedDays(period: Pick<ReportPeriod, "days" | "today">): string[] {
  return period.days.filter((d) => d <= period.today);
}

/** Saturday or Sunday, for a "YYYY-MM-DD" day key (the calendar date it names, whatever the zone). */
export function isWeekendDay(key: string): boolean {
  const day = new Date(Date.UTC(Number(key.slice(0, 4)), Number(key.slice(5, 7)) - 1, Number(key.slice(8, 10)))).getUTCDay();
  return day === 0 || day === 6;
}

/** The day key of a server date ("2026-07-14T00:00:00", "2026-07-14T00:00:00Z"); null when unreadable. */
export function serverDayKey(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const key = value.trim().slice(0, 10);
  return isDayKey(key) ? key : null;
}

/** An instant the server sent, read as UTC even without a zone suffix (ABP may omit it); null when unreadable. */
export function utcInstant(value: unknown): number | null {
  if (typeof value !== "string" || value.trim() === "") return null;
  const raw = value.trim();
  const at = Date.parse(/^\d{4}-\d{2}-\d{2}T[\d:.]+$/.test(raw) ? `${raw}Z` : raw);
  return Number.isFinite(at) ? at : null;
}

/* ----------------------------- Hours ----------------------------- */

/** How hours read: "7h 32m", "7:32" or decimal "7.53" (the same keys as work-report's HOURS_FORMATS). */
export type HoursFormat = "duration" | "clock" | "decimal";

/** Decimal hours to 2 places (7:32 → 7.53), as the reports and their CSVs show them. */
function decimalHours(minutes: unknown): number {
  return Math.round((minutesOf(minutes) / 60) * 100) / 100;
}

/**
 * The Total of a column of durations, as displayed: the sum of the cells
 * ABOVE it after their own rounding — 2-place decimal hours in the decimal
 * format, whole minutes otherwise — so a printed total always adds up.
 * Returned as minutes, ready for formatHours(total, format).
 */
export function displayedTotal(minutes: readonly unknown[], format: HoursFormat): number {
  if (format === "decimal") {
    const hundredths = minutes.reduce<number>((s, m) => s + Math.round(decimalHours(m) * 100), 0);
    return (hundredths * 60) / 100;
  }
  return minutes.reduce<number>((s, m) => s + Math.round(minutesOf(m)), 0);
}

/** Decimal hours of a column total for a CSV: the sum of the rows' own 2-place values. */
export function decimalHoursTotal(minutes: readonly unknown[]): number {
  return minutes.reduce<number>((s, m) => s + Math.round(decimalHours(m) * 100), 0) / 100;
}

/* ----------------------------- Activity & idle ----------------------------- */

/** How a stretch of tracked time splits (whole minutes). */
export interface ActivityBreakdown {
  tracked: number;
  /** Keyboard/mouse time (tracked × activity %); null when the input wasn't measured. */
  active: number | null;
  /** Minutes without input; null when the source doesn't report idle time. */
  idle: number | null;
  /** The rest: tracked time that is neither measured as active nor as idle. */
  other: number;
  /** Activity %, 0–100 (whole); null when nothing was measured. */
  activity: number | null;
}

/**
 * Split tracked minutes by the activity score and the reported idle minutes.
 * Nothing is invented: an unmeasured score leaves `active` null and an idle
 * figure the server didn't send leaves `idle` null. Idle minutes are measured
 * (minute buckets without input), the active time is derived from an average,
 * so when the two overlap it is the active time that gives way.
 */
export function activityBreakdown(trackedMinutes: unknown, averageProductivity: unknown, idleMinutes?: unknown): ActivityBreakdown {
  const tracked = Math.round(minutesOf(trackedMinutes));
  const score = optionalPercent(averageProductivity);
  const idleRaw = optionalMinutes(idleMinutes);
  const idle = idleRaw === null ? null : Math.min(tracked, Math.round(idleRaw));
  const active = score === null ? null : Math.min(tracked - (idle ?? 0), Math.round((tracked * score) / 100));
  return {
    tracked,
    active,
    idle,
    activity: score === null ? null : Math.round(score),
    other: tracked - (active ?? 0) - (idle ?? 0),
  };
}

/**
 * A block's idle minutes implied by its activity %: the share of its minute
 * buckets without input (the agents' own definition, contract §1). For the
 * demo dataset, whose blocks carry no timeline; 0 for an unmeasured block.
 */
export function idleFromActivity(minutes: number, productivity: number | null | undefined): number {
  const score = optionalPercent(productivity);
  if (score === null) return 0;
  return (minutesOf(minutes) * (100 - score)) / 100;
}

/* ----------------------------- Reporting shapes ----------------------------- */

/** The figures every reporting row carries. */
export interface SummaryFigures {
  activityCount: number;
  trackedMinutes: number;
  /** null when the server doesn't report idle time (older servers). */
  idleMinutes: number | null;
  /** Minute-weighted over measured blocks; null when nothing was measured. */
  averageProductivity: number | null;
}

export interface MemberFigures extends SummaryFigures {
  userId: string;
  /** The server's display name (or login name); the roster's name is preferred when known. */
  name: string | null;
}

export interface ProjectFigures extends SummaryFigures {
  projectId: string;
  title: string | null;
  /** null when the server doesn't say. */
  isArchived: boolean | null;
}

export interface MemberProjectFigures extends SummaryFigures {
  userId: string;
  projectId: string;
}

export interface ReportSummary extends SummaryFigures {
  perUser: MemberFigures[];
  perProject: ProjectFigures[];
  /** null when the server sends no member × project breakdown (older servers). */
  perUserProject: MemberProjectFigures[] | null;
}

export interface DailyFigures extends SummaryFigures {
  /** The day ("YYYY-MM-DD") in the reporting zone. */
  date: string;
}

export interface MemberDayFigures {
  userId: string;
  name: string | null;
  date: string;
  trackedMinutes: number;
  activityCount: number;
  idleMinutes: number | null;
}

function figuresOf(row: Record<string, unknown>, counts: "activityCount" | "totalActivities" = "activityCount", minutes = "trackedMinutes", idle = "idleMinutes"): SummaryFigures {
  return {
    activityCount: countOf(row[counts]),
    trackedMinutes: minutesOf(row[minutes]),
    idleMinutes: optionalMinutes(row[idle]),
    averageProductivity: optionalPercent(row.averageProductivity),
  };
}

/** GET /api/app/reporting/summary, tolerant of missing or malformed parts. */
export function readSummary(raw: unknown): ReportSummary {
  const s = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const perUserProject = Array.isArray(s.perUserProject)
    ? recordsOf(s.perUserProject)
        .filter((r) => typeof r.userId === "string" && typeof r.projectId === "string")
        .map((r) => ({ userId: r.userId as string, projectId: r.projectId as string, ...figuresOf(r) }))
    : null;
  return {
    ...figuresOf(s, "totalActivities", "totalTrackedMinutes", "totalIdleMinutes"),
    perUser: recordsOf(s.perUser)
      .filter((r) => typeof r.userId === "string")
      .map((r) => ({ userId: r.userId as string, name: textOf(r.name) ?? textOf(r.userName), ...figuresOf(r) })),
    perProject: recordsOf(s.perProject)
      .filter((r) => typeof r.projectId === "string")
      .map((r) => ({
        projectId: r.projectId as string,
        title: textOf(r.projectTitle),
        isArchived: typeof r.isArchived === "boolean" ? r.isArchived : null,
        ...figuresOf(r),
      })),
    perUserProject,
  };
}

/** GET /api/app/reporting/daily-series. Rows without a readable date are skipped. */
export function readDaily(raw: unknown): DailyFigures[] {
  return recordsOf(raw).flatMap((r) => {
    const date = serverDayKey(r.date);
    return date ? [{ date, ...figuresOf(r) }] : [];
  });
}

/** GET /api/app/reporting/user-daily-series. */
export function readMemberDays(raw: unknown): MemberDayFigures[] {
  return recordsOf(raw).flatMap((r) => {
    const date = serverDayKey(r.date);
    if (!date || typeof r.userId !== "string") return [];
    return [
      {
        userId: r.userId,
        name: textOf(r.userName),
        date,
        trackedMinutes: minutesOf(r.trackedMinutes),
        activityCount: countOf(r.activityCount),
        idleMinutes: optionalMinutes(r.idleMinutes),
      },
    ];
  });
}

/** The minimal block the demo figures are derived from (a seeded activity). */
export interface ReportBlock {
  userId: string;
  projectId: string;
  startedAt: string;
  endedAt: string;
  /** 0–100, or null when the block's input was not measured. */
  productivity: number | null;
  /** Minutes without input; derived from the activity % when not given (see idleFromActivity). */
  idleMinutes?: number | null;
}

function blockMinutes(b: { startedAt: string; endedAt: string }): number {
  const start = Date.parse(b.startedAt);
  const end = Date.parse(b.endedAt);
  return Number.isFinite(start) && Number.isFinite(end) ? Math.max(0, (end - start) / 60_000) : 0;
}

interface FigureAcc {
  count: number;
  minutes: number;
  idle: number;
  measured: number;
  weighted: number;
}

const newAcc = (): FigureAcc => ({ count: 0, minutes: 0, idle: 0, measured: 0, weighted: 0 });

function addBlock(acc: FigureAcc, minutes: number, productivity: number | null, idle: number): void {
  acc.count += 1;
  acc.minutes += minutes;
  acc.idle += idle;
  if (productivity !== null) {
    acc.measured += minutes;
    acc.weighted += minutes * productivity;
  }
}

/** Two decimals, as the server rounds its figures. */
function round2(n: number): number {
  return Math.round(n * 100) / 100;
}

function figuresFrom(acc: FigureAcc): SummaryFigures {
  return {
    activityCount: acc.count,
    trackedMinutes: round2(acc.minutes),
    idleMinutes: round2(acc.idle),
    averageProductivity: acc.measured > 0 ? round2(acc.weighted / acc.measured) : null,
  };
}

function bump<K>(map: Map<K, FigureAcc>, key: K): FigureAcc {
  let acc = map.get(key);
  if (!acc) {
    acc = newAcc();
    map.set(key, acc);
  }
  return acc;
}

/**
 * The reporting shapes (summary, daily series, member-day series) derived from
 * blocks — the demo's counterpart of the /reporting/* endpoints. A block is
 * booked to the day its END falls on in `timeZone` (metrics.ts' rule), and its
 * activity % is duration-weighted over measured blocks only.
 */
export function summarizeBlocks(
  blocks: readonly ReportBlock[],
  timeZone: string,
): { summary: ReportSummary; daily: DailyFigures[]; memberDays: MemberDayFigures[] } {
  const total = newAcc();
  const users = new Map<string, FigureAcc>();
  const projects = new Map<string, FigureAcc>();
  const pairs = new Map<string, FigureAcc>();
  const days = new Map<string, FigureAcc>();
  const memberDays = new Map<string, FigureAcc>();
  for (const b of blocks) {
    const end = Date.parse(b.endedAt);
    if (!Number.isFinite(end) || typeof b.userId !== "string" || typeof b.projectId !== "string") continue;
    const minutes = blockMinutes(b);
    const productivity = optionalPercent(b.productivity);
    const idle = Math.min(minutes, b.idleMinutes === null || b.idleMinutes === undefined ? idleFromActivity(minutes, productivity) : minutesOf(b.idleMinutes));
    const day = zonedDayKey(end, timeZone);
    for (const acc of [
      total,
      bump(users, b.userId),
      bump(projects, b.projectId),
      bump(pairs, `${b.userId}\u0000${b.projectId}`),
      bump(days, day),
      bump(memberDays, `${b.userId}\u0000${day}`),
    ]) {
      addBlock(acc, minutes, productivity, idle);
    }
  }
  const split = (key: string) => key.split("\u0000") as [string, string];
  return {
    summary: {
      ...figuresFrom(total),
      perUser: [...users].map(([userId, acc]) => ({ userId, name: null, ...figuresFrom(acc) })),
      perProject: [...projects].map(([projectId, acc]) => ({ projectId, title: null, isArchived: null, ...figuresFrom(acc) })),
      perUserProject: [...pairs].map(([key, acc]) => {
        const [userId, projectId] = split(key);
        return { userId, projectId, ...figuresFrom(acc) };
      }),
    },
    daily: [...days].sort(([a], [b]) => (a < b ? -1 : 1)).map(([date, acc]) => ({ date, ...figuresFrom(acc) })),
    memberDays: [...memberDays].map(([key, acc]) => {
      const [userId, date] = split(key);
      return { userId, name: null, date, trackedMinutes: round2(acc.minutes), activityCount: acc.count, idleMinutes: round2(acc.idle) };
    }),
  };
}

/** One day of a period's chart. */
export interface DayBar {
  date: string;
  weekend: boolean;
  /** Whole minutes tracked. */
  minutes: number;
  active: number | null;
  idle: number | null;
  other: number;
  activity: number | null;
  sessions: number;
}

/**
 * One bar per day of the period (days without time read 0). A row on a day
 * outside the period that carries time is kept as an extra bar rather than
 * dropped — tracked time never silently disappears from a chart.
 */
export function dayBars(days: readonly string[], rows: readonly DailyFigures[]): DayBar[] {
  const byDay = new Map<string, DailyFigures[]>();
  for (const r of rows) byDay.set(r.date, [...(byDay.get(r.date) ?? []), r]);
  const keys = new Set(days);
  for (const r of rows) if (r.trackedMinutes > 0) keys.add(r.date);
  return [...keys].sort().map((date) => {
    const parts = byDay.get(date) ?? [];
    const minutes = parts.reduce((s, r) => s + r.trackedMinutes, 0);
    const measured = parts.filter((r) => r.averageProductivity !== null);
    const measuredMinutes = measured.reduce((s, r) => s + r.trackedMinutes, 0);
    const score = measuredMinutes > 0 ? measured.reduce((s, r) => s + r.trackedMinutes * (r.averageProductivity ?? 0), 0) / measuredMinutes : null;
    const idleKnown = parts.some((r) => r.idleMinutes !== null);
    const split = activityBreakdown(minutes, score, idleKnown ? parts.reduce((s, r) => s + (r.idleMinutes ?? 0), 0) : null);
    return {
      date,
      weekend: isWeekendDay(date),
      minutes: split.tracked,
      active: split.active,
      idle: split.idle,
      other: split.other,
      activity: split.activity,
      sessions: parts.reduce((s, r) => s + r.activityCount, 0),
    };
  });
}

/** userId → distinct days with tracked time. */
export function activeDaysByUser(rows: readonly { userId: string; date: string; trackedMinutes?: number }[]): Map<string, number> {
  const days = new Map<string, Set<string>>();
  for (const r of rows) {
    if (!r || typeof r.userId !== "string" || typeof r.date !== "string") continue;
    if (r.trackedMinutes !== undefined && !(r.trackedMinutes > 0)) continue;
    const set = days.get(r.userId) ?? new Set<string>();
    set.add(r.date.slice(0, 10));
    days.set(r.userId, set);
  }
  return new Map([...days].map(([id, set]) => [id, set.size]));
}

/** Each member's project with the most time in a member × project breakdown (ties: the earlier id). */
export function topProjectByMember(rows: readonly MemberProjectFigures[]): Map<string, { projectId: string; minutes: number }> {
  const top = new Map<string, { projectId: string; minutes: number }>();
  for (const r of rows) {
    if (!(r.trackedMinutes > 0)) continue;
    const current = top.get(r.userId);
    if (!current || r.trackedMinutes > current.minutes || (r.trackedMinutes === current.minutes && r.projectId < current.projectId)) {
      top.set(r.userId, { projectId: r.projectId, minutes: r.trackedMinutes });
    }
  }
  return top;
}

/* ----------------------------- Money ----------------------------- */

/**
 * The currency report amounts are in. Rates and pay are stored without a
 * currency (the workspace has one, unnamed in the API), and the product
 * prices everything in US dollars — one constant, so a workspace currency
 * setting only has to be threaded through here.
 */
export const REPORT_CURRENCY = "USD";

/** Round to cents; anything non-finite is 0. */
export function toCents(n: number): number {
  return Number.isFinite(n) ? Math.round(n * 100) / 100 : 0;
}

/** A sum of amounts, added up in whole cents so the total is exactly the sum of the shown cents. */
export function sumMoney(values: readonly number[]): number {
  return values.reduce((s, v) => s + Math.round(toCents(v) * 100), 0) / 100;
}

const moneyFormatters = new Map<string, Intl.NumberFormat>();

function moneyFormatter(currency: string): Intl.NumberFormat {
  let formatter = moneyFormatters.get(currency);
  if (!formatter) {
    try {
      formatter = new Intl.NumberFormat("en-US", { style: "currency", currency, minimumFractionDigits: 2, maximumFractionDigits: 2 });
    } catch {
      formatter = new Intl.NumberFormat("en-US", { style: "currency", currency: REPORT_CURRENCY, minimumFractionDigits: 2, maximumFractionDigits: 2 });
    }
    moneyFormatters.set(currency, formatter);
  }
  return formatter;
}

/** "$1,234.50": always two decimals, one locale for every viewer (a payroll must read the same everywhere). */
export function formatMoney(amount: number, currency: string = REPORT_CURRENCY): string {
  const cents = toCents(amount);
  return moneyFormatter(currency).format(cents === 0 ? 0 : cents);
}

/** "$40.00/hr". */
export function formatRate(rate: number, currency: string = REPORT_CURRENCY): string {
  return `${formatMoney(rate, currency)}/hr`;
}

/* ----------------------------- Payroll ----------------------------- */

/** One project membership row (GET /api/app/team/project-members/{projectId}). */
export interface MemberRate {
  userId: string;
  hourlyRate?: number | null;
}

/** One member's tracked minutes (a reporting summary `perUser` row). */
export interface MinutesRow {
  userId: string;
  trackedMinutes: number;
}

/**
 * userId → hourly rate on ONE project. Missing, negative or non-numeric rates
 * count as 0 (unpriced); a duplicate membership keeps the higher rate.
 */
export function ratesByMember(rates: readonly MemberRate[]): Map<string, number> {
  const map = new Map<string, number>();
  for (const r of rates) {
    if (!r || typeof r.userId !== "string") continue;
    const n = Number(r.hourlyRate);
    const rate = Number.isFinite(n) && n > 0 ? n : 0;
    map.set(r.userId, Math.max(map.get(r.userId) ?? 0, rate));
  }
  return map;
}

/** One member's pay on one project over the period. */
export interface PayrollLine {
  userId: string;
  /** The server's name for the member (the roster's is preferred when known). */
  userName: string | null;
  projectId: string;
  projectTitle: string | null;
  minutes: number;
  /** In cents: Σ(block minutes × the rate in effect when the block started) / 60. */
  amount: number;
  /** The effective hourly rate, minute-weighted over the period (0 = no rate). */
  rate: number;
  activityCount: number;
}

/** GET /api/app/reporting/payroll (PayrollReportDto), sanitised; amounts rounded to cents. */
export function readPayroll(raw: unknown): PayrollLine[] {
  const rows = raw && typeof raw === "object" && !Array.isArray(raw) ? (raw as { rows?: unknown }).rows : raw;
  return recordsOf(rows).flatMap((r) => {
    if (typeof r.userId !== "string" || typeof r.projectId !== "string") return [];
    const amount = numeric(r.amount);
    const rate = numeric(r.hourlyRate);
    return [
      {
        userId: r.userId,
        userName: textOf(r.userName),
        projectId: r.projectId,
        projectTitle: textOf(r.projectTitle),
        minutes: minutesOf(r.trackedMinutes),
        amount: Number.isFinite(amount) && amount > 0 ? toCents(amount) : 0,
        rate: Number.isFinite(rate) && rate > 0 ? rate : 0,
        activityCount: countOf(r.activityCount),
      },
    ];
  });
}

/**
 * Pay lines computed in the browser from each project's CURRENT rates — only
 * for a server without the payroll endpoint. (It reprices past hours when a
 * rate changes, which is why the server prices each block instead.)
 */
export function legacyPayrollLines(
  projects: readonly { projectId: string; projectTitle?: string | null; perUser: readonly MinutesRow[]; rates: ReadonlyMap<string, number> }[],
): PayrollLine[] {
  const lines: PayrollLine[] = [];
  for (const p of projects) {
    for (const row of p.perUser) {
      const minutes = minutesOf(row?.trackedMinutes);
      if (!row || typeof row.userId !== "string" || minutes <= 0) continue;
      const rate = p.rates.get(row.userId) ?? 0;
      lines.push({
        userId: row.userId,
        userName: null,
        projectId: p.projectId,
        projectTitle: p.projectTitle ?? null,
        minutes,
        amount: toCents((minutes / 60) * rate),
        rate,
        activityCount: 0,
      });
    }
  }
  return lines;
}

/** Pay lines from blocks and a rate lookup (the demo's payroll: each member's rate on each project). */
export function payrollLinesFromBlocks(
  blocks: readonly Pick<ReportBlock, "userId" | "projectId" | "startedAt" | "endedAt">[],
  rateOf: (userId: string, projectId: string) => number,
): PayrollLine[] {
  const acc = new Map<string, { userId: string; projectId: string; minutes: number; count: number }>();
  for (const b of blocks) {
    const key = `${b.userId}\u0000${b.projectId}`;
    const entry = acc.get(key) ?? { userId: b.userId, projectId: b.projectId, minutes: 0, count: 0 };
    entry.minutes += blockMinutes(b);
    entry.count += 1;
    acc.set(key, entry);
  }
  return [...acc.values()]
    .filter((e) => e.minutes > 0)
    .map((e) => {
      const n = rateOf(e.userId, e.projectId);
      const rate = Number.isFinite(n) && n > 0 ? n : 0;
      const minutes = round2(e.minutes);
      return { userId: e.userId, userName: null, projectId: e.projectId, projectTitle: null, minutes, amount: toCents((minutes / 60) * rate), rate, activityCount: e.count };
    });
}

/** One member's pay over the period, across projects. */
export interface PayrollMember {
  userId: string;
  userName: string | null;
  minutes: number;
  /** Σ of the member's lines, in cents. */
  amount: number;
  /** Per paid hour: amount ÷ the hours that have a rate (0 when none do). */
  rate: number;
  /** Minutes on projects without a rate (tracked, not paid). */
  unpricedMinutes: number;
  /** The member's lines, most paid first. */
  lines: PayrollLine[];
}

function byPayThenTime(a: { amount: number; minutes: number }, b: { amount: number; minutes: number }): number {
  return b.amount - a.amount || b.minutes - a.minutes;
}

/** Pay per member: the sum of their lines, so a member's amount always equals the lines listed under them. */
export function payrollByMember(lines: readonly PayrollLine[]): PayrollMember[] {
  const groups = new Map<string, PayrollLine[]>();
  for (const l of lines) groups.set(l.userId, [...(groups.get(l.userId) ?? []), l]);
  return [...groups].map(([userId, list]) => {
    const minutes = list.reduce((s, l) => s + l.minutes, 0);
    const unpricedMinutes = list.filter((l) => !(l.rate > 0)).reduce((s, l) => s + l.minutes, 0);
    const amount = sumMoney(list.map((l) => l.amount));
    return {
      userId,
      userName: list.find((l) => l.userName)?.userName ?? null,
      minutes,
      amount,
      rate: paidRate(amount, minutes - unpricedMinutes),
      unpricedMinutes,
      lines: [...list].sort(byPayThenTime),
    };
  }).sort((a, b) => byPayThenTime(a, b) || a.userId.localeCompare(b.userId));
}

/** Amount ÷ paid hours, to cents; 0 without paid hours. */
function paidRate(amount: number, paidMinutes: number): number {
  return paidMinutes > 0 ? toCents(amount / (paidMinutes / 60)) : 0;
}

/** The period's totals: amount is the cent-exact sum of the lines (it always equals the rows above it). */
export function payrollTotals(lines: readonly PayrollLine[]): { minutes: number; amount: number; unpricedMinutes: number; members: number; rate: number } {
  const minutes = lines.reduce((s, l) => s + l.minutes, 0);
  const unpricedMinutes = lines.filter((l) => !(l.rate > 0)).reduce((s, l) => s + l.minutes, 0);
  const amount = sumMoney(lines.map((l) => l.amount));
  return { minutes, amount, unpricedMinutes, members: new Set(lines.map((l) => l.userId)).size, rate: paidRate(amount, minutes - unpricedMinutes) };
}

/** projectId → labour cost (cent-exact sum of the project's pay lines). */
export function projectCosts(lines: readonly PayrollLine[]): Map<string, number> {
  const groups = new Map<string, number[]>();
  for (const l of lines) groups.set(l.projectId, [...(groups.get(l.projectId) ?? []), l.amount]);
  return new Map([...groups].map(([id, amounts]) => [id, sumMoney(amounts)]));
}

/* ----------------------------- Apps ----------------------------- */

/** An app's category, or "unclassified" when nobody has classified it (lib/apps would call it neutral). */
export type AppGroup = AppCategory | "unclassified";

export const APP_GROUPS: readonly AppGroup[] = ["productive", "neutral", "unproductive", "unclassified"];

export const APP_GROUP_LABELS: Record<AppGroup, string> = {
  productive: "Productive",
  neutral: "Neutral",
  unproductive: "Unproductive",
  unclassified: "Unclassified",
};

/** The group an app's time counts under (contract §8 names; categories from lib/apps). */
export function appGroupOf(name: string | null | undefined): AppGroup {
  return isClassifiedApp(name) ? appCategory(name) : "unclassified";
}

/** One application's use over the period, under its canonical name. */
export interface AppUsageLine {
  /** Grouping key (the canonical name, lower-cased). */
  key: string;
  name: string;
  group: AppGroup;
  /** The app's colour: the same on every page, whatever its rank. */
  color: string;
  /** Focused minutes. */
  minutes: number;
  /** null when the server doesn't report idle time per app. */
  idleMinutes: number | null;
  /** Blocks in which the app appeared (focused or running). */
  activityCount: number;
  /** Members who used it. */
  userCount: number;
}

function appLine(rawName: string | null, minutes: number, idle: number | null, activityCount: number, userCount: number): AppUsageLine {
  const name = friendlyAppName(rawName);
  return { key: appKey(rawName), name, group: appGroupOf(name), color: appColor(name), minutes, idleMinutes: idle, activityCount, userCount };
}

/**
 * GET /api/app/reporting/app-usage, grouped by canonical name — a newer server
 * already groups (displayName); an older one sends raw process names, so
 * "chrome.exe" and "Chrome.exe" are folded together here. Members of folded
 * rows can't be de-duplicated, so the larger count is kept (a lower bound).
 */
export function readAppUsage(raw: unknown): AppUsageLine[] {
  const groups = new Map<string, AppUsageLine>();
  for (const r of recordsOf(raw)) {
    const rawName = textOf(r.displayName) ?? textOf(r.appName);
    const line = appLine(rawName, minutesOf(r.trackedMinutes), optionalMinutes(r.idleMinutes), countOf(r.activityCount), countOf(r.userCount));
    const current = groups.get(line.key);
    if (!current) {
      groups.set(line.key, line);
      continue;
    }
    current.minutes += line.minutes;
    current.idleMinutes = current.idleMinutes === null && line.idleMinutes === null ? null : (current.idleMinutes ?? 0) + (line.idleMinutes ?? 0);
    current.activityCount += line.activityCount;
    current.userCount = Math.max(current.userCount, line.userCount);
  }
  return [...groups.values()].sort((a, b) => b.minutes - a.minutes || a.name.localeCompare(b.name));
}

/**
 * App usage derived from blocks, crediting time as the server does: each
 * focused window its own seconds (scaled down when a block reports more than
 * it lasted); a legacy block without per-window time credits its first
 * focused app with the whole block. Running programs count toward reach.
 */
export function appUsageFromBlocks(
  blocks: readonly {
    userId: string;
    startedAt: string;
    endedAt: string;
    activeWindows?: readonly { appName?: string | null; seconds?: number | null }[] | null;
    runningPrograms?: readonly { appName?: string | null }[] | null;
  }[],
): AppUsageLine[] {
  const minutes = new Map<string, number>();
  const names = new Map<string, string | null>();
  const counts = new Map<string, number>();
  const people = new Map<string, Set<string>>();
  for (const b of blocks) {
    const duration = blockMinutes(b);
    const active = (b.activeWindows ?? []).filter((w) => w && textOf(w.appName));
    const timed = active.filter((w) => minutesOf(w.seconds) > 0);
    const credit = (name: string, m: number) => {
      const key = appKey(name);
      minutes.set(key, (minutes.get(key) ?? 0) + m);
      if (!names.has(key)) names.set(key, name);
    };
    if (timed.length) {
      const reported = timed.reduce((s, w) => s + minutesOf(w.seconds) / 60, 0);
      const scale = reported > duration ? duration / reported : 1;
      for (const w of timed) credit(textOf(w.appName) as string, (minutesOf(w.seconds) / 60) * scale);
    } else if (active[0]) {
      credit(textOf(active[0].appName) as string, duration);
    }
    const seen = new Set<string>();
    for (const w of [...active, ...(b.runningPrograms ?? [])]) {
      const name = textOf(w?.appName);
      if (!name) continue;
      const key = appKey(name);
      if (seen.has(key)) continue;
      seen.add(key);
      if (!names.has(key)) names.set(key, name);
      counts.set(key, (counts.get(key) ?? 0) + 1);
      const set = people.get(key) ?? new Set<string>();
      set.add(b.userId);
      people.set(key, set);
    }
  }
  return [...names]
    .map(([key, name]) => appLine(name, round2(minutes.get(key) ?? 0), null, counts.get(key) ?? 0, people.get(key)?.size ?? 0))
    .sort((a, b) => b.minutes - a.minutes || a.name.localeCompare(b.name));
}

/** Minutes per group, and the total. */
export function appGroupTotals(lines: readonly AppUsageLine[]): Record<AppGroup, number> & { total: number } {
  const totals = { productive: 0, neutral: 0, unproductive: 0, unclassified: 0, total: 0 };
  for (const l of lines) {
    totals[l.group] += l.minutes;
    totals.total += l.minutes;
  }
  return totals;
}

/* ----------------------------- Attendance ----------------------------- */

/** One member's presence over the period. */
export interface AttendanceLine {
  userId: string;
  name: string | null;
  /** Days ("YYYY-MM-DD") with tracked time — weekdays and weekends. */
  presentDays: Set<string>;
  workedMinutes: number;
  idleMinutes: number | null;
  /** Minutes after local midnight of the day's first start, averaged over present days; null when not reported. */
  averageStart: number | null;
  /** …of the day's last end. */
  averageEnd: number | null;
  /** The server's count of weekend days worked (null from older servers). */
  weekendDaysReported: number | null;
}

/**
 * GET /api/app/reporting/attendance. `detailed` says the server sends the
 * contract-v2 fields (weekend days, start/finish); an older one doesn't, and
 * also leaves out members who tracked nothing (see withRoster).
 */
export function readAttendance(raw: unknown): { lines: AttendanceLine[]; detailed: boolean } {
  const rows = recordsOf(raw && typeof raw === "object" ? (raw as { rows?: unknown }).rows : null);
  const detailed = rows.some((r) => r.weekendDaysWorked !== undefined || r.averageStartMinutes !== undefined);
  return {
    detailed,
    lines: rows
      .filter((r) => typeof r.userId === "string")
      .map((r) => ({
        userId: r.userId as string,
        name: textOf(r.userName),
        presentDays: new Set(asArray<unknown>(r.presentDays).flatMap((d) => serverDayKey(d) ?? [])),
        workedMinutes: minutesOf(r.workedMinutes),
        idleMinutes: optionalMinutes(r.idleMinutes),
        averageStart: optionalMinutes(r.averageStartMinutes),
        averageEnd: optionalMinutes(r.averageEndMinutes),
        weekendDaysReported: r.weekendDaysWorked === undefined || r.weekendDaysWorked === null ? null : countOf(r.weekendDaysWorked),
      })),
  };
}

/** Add the roster members a source left out (they tracked nothing): present nowhere, absent every working day. */
export function withRoster(lines: readonly AttendanceLine[], roster: readonly { userId: string; name?: string | null }[]): AttendanceLine[] {
  const known = new Set(lines.map((l) => l.userId));
  const missing = roster
    .filter((m) => !known.has(m.userId))
    .map((m) => ({
      userId: m.userId,
      name: m.name ?? null,
      presentDays: new Set<string>(),
      workedMinutes: 0,
      idleMinutes: null,
      averageStart: null,
      averageEnd: null,
      weekendDaysReported: null,
    }));
  return [...lines, ...missing];
}

/**
 * Presence derived from blocks (the demo's attendance): a member is present on
 * a day of `timeZone` where a block STARTED; the day's first start and last
 * end give its start and finish (minutes after that day's local midnight).
 */
export function attendanceFromBlocks(blocks: readonly ReportBlock[], timeZone: string): AttendanceLine[] {
  const perUser = new Map<string, { days: Map<string, { first: number; last: number }>; minutes: number; idle: number }>();
  for (const b of blocks) {
    const start = Date.parse(b.startedAt);
    const end = Date.parse(b.endedAt);
    if (!Number.isFinite(start) || !Number.isFinite(end) || typeof b.userId !== "string") continue;
    const minutes = blockMinutes(b);
    const entry = perUser.get(b.userId) ?? { days: new Map(), minutes: 0, idle: 0 };
    entry.minutes += minutes;
    entry.idle += Math.min(minutes, b.idleMinutes === null || b.idleMinutes === undefined ? idleFromActivity(minutes, b.productivity) : minutesOf(b.idleMinutes));
    const day = zonedDayKey(start, timeZone);
    const span = entry.days.get(day);
    entry.days.set(day, { first: Math.min(span?.first ?? start, start), last: Math.max(span?.last ?? end, end) });
    perUser.set(b.userId, entry);
  }
  return [...perUser].map(([userId, e]) => {
    let starts = 0;
    let ends = 0;
    for (const [day, span] of e.days) {
      const midnight = startOfZonedDay(day, timeZone).getTime();
      starts += (span.first - midnight) / 60_000;
      ends += (span.last - midnight) / 60_000;
    }
    const n = e.days.size;
    return {
      userId,
      name: null,
      presentDays: new Set(e.days.keys()),
      workedMinutes: round2(e.minutes),
      idleMinutes: round2(e.idle),
      averageStart: n ? Math.round(starts / n) : null,
      averageEnd: n ? Math.round(ends / n) : null,
      weekendDaysReported: null,
    };
  });
}

/** One member's attendance as the report shows it. */
export interface AttendanceRow {
  userId: string;
  name: string | null;
  /** Working days (Mon–Fri) with tracked time. */
  present: number;
  /** Working days without. */
  absent: number;
  /** Saturdays and Sundays with tracked time. */
  weekendDays: number;
  workedMinutes: number;
  idleMinutes: number | null;
  averageStart: number | null;
  averageEnd: number | null;
  presentDays: Set<string>;
}

/**
 * Attendance over the period's days that have begun (`days` up to today):
 * present + absent always equals the working days shown, so the counts and
 * the day grid agree. Weekend work is counted apart (never as an absence).
 */
export function attendanceRows(lines: readonly AttendanceLine[], days: readonly string[]): AttendanceRow[] {
  const workdays = days.filter((d) => !isWeekendDay(d));
  const weekends = days.filter((d) => isWeekendDay(d));
  return lines.map((l) => {
    const present = workdays.filter((d) => l.presentDays.has(d)).length;
    return {
      userId: l.userId,
      name: l.name,
      present,
      absent: workdays.length - present,
      weekendDays: Math.max(weekends.filter((d) => l.presentDays.has(d)).length, l.weekendDaysReported ?? 0),
      workedMinutes: Math.round(l.workedMinutes),
      idleMinutes: l.idleMinutes === null ? null : Math.round(l.idleMinutes),
      averageStart: l.averageStart,
      averageEnd: l.averageEnd,
      presentDays: l.presentDays,
    };
  });
}

/** The team's typical start and finish: each member's averages weighted by the days they were present. */
export function averageClock(rows: readonly AttendanceRow[]): { start: number | null; end: number | null } {
  let startWeight = 0;
  let endWeight = 0;
  let start = 0;
  let end = 0;
  for (const r of rows) {
    const weight = r.present + r.weekendDays;
    if (weight <= 0) continue;
    if (r.averageStart !== null) {
      start += r.averageStart * weight;
      startWeight += weight;
    }
    if (r.averageEnd !== null) {
      end += r.averageEnd * weight;
      endWeight += weight;
    }
  }
  return { start: startWeight ? Math.round(start / startWeight) : null, end: endWeight ? Math.round(end / endWeight) : null };
}

/** "09:05" for minutes after midnight; "00:30 (+1)" past midnight; "—" when unknown. */
export function formatClock(minutes: number | null | undefined): string {
  if (minutes === null || minutes === undefined || !Number.isFinite(minutes) || minutes < 0) return "—";
  const total = Math.round(minutes);
  const day = Math.floor(total / 1440);
  const inDay = total % 1440;
  const clock = `${String(Math.floor(inDay / 60)).padStart(2, "0")}:${String(inDay % 60).padStart(2, "0")}`;
  return day > 0 ? `${clock} (+${day})` : clock;
}

/* ----------------------------- Devices ----------------------------- */

export type DevicePlatform = "windows" | "macos" | "linux";

/** What a device's agent reported most recently (presence), else over the period (device report). */
export interface DeviceLine {
  /** "<userId>:<deviceId>", or "<userId>:legacy" for blocks from agents that send no device id. */
  key: string;
  userId: string;
  userName: string | null;
  deviceId: string | null;
  deviceName: string | null;
  platform: DevicePlatform | null;
  osVersion: string | null;
  agentVersion: string | null;
  /** Minutes tracked on the device over the period. */
  trackedMinutes: number;
  /** UTC instant (ms) of the device's last tracked block in the period; null when none. */
  lastActivityAt: number | null;
  presence: {
    status: string;
    online: boolean;
    /** UTC instant (ms) of the last heartbeat; null when unreadable. */
    lastSeenAt: number | null;
    statusSince: number | null;
    projectTitle: string | null;
    currentApp: string | null;
  } | null;
}

/** A device is online while its last heartbeat is at most this old (contract §3). */
export const DEVICE_ONLINE_SECONDS = 150;

function platformOf(value: unknown): DevicePlatform | null {
  const p = typeof value === "string" ? value.trim().toLowerCase() : "";
  return p === "windows" || p === "macos" || p === "linux" ? p : null;
}

/** Devices are told apart per member (the server groups by member + device too). */
function deviceKey(deviceId: unknown, userId: string): string {
  const id = textOf(deviceId);
  return `${userId}:${id ? id.toLowerCase() : "legacy"}`;
}

/**
 * The period's devices (GET /api/app/reporting/devices) merged with live
 * presence (GET /api/app/presence/team) by device: what the agent reports
 * now wins over what it reported during the period, a device that is running
 * but tracked nothing in the period is still listed (unless `knownOnly`, for
 * a report narrowed to a project, which presence can't be), and "online" is
 * re-judged against `now` so a page left open ages correctly.
 */
export function mergeDevices(devices: unknown, presence: unknown, now: Date | number, options: { knownOnly?: boolean } = {}): DeviceLine[] {
  const at = typeof now === "number" ? now : now.getTime();
  const lines = new Map<string, DeviceLine>();
  for (const r of recordsOf(devices)) {
    if (typeof r.userId !== "string") continue;
    const key = deviceKey(r.deviceId, r.userId);
    const last = utcInstant(r.lastActivityAt);
    const current = lines.get(key);
    if (current) {
      current.trackedMinutes += minutesOf(r.trackedMinutes);
      if (last !== null && (current.lastActivityAt === null || last > current.lastActivityAt)) current.lastActivityAt = last;
      continue;
    }
    lines.set(key, {
      key,
      userId: r.userId,
      userName: textOf(r.userName),
      deviceId: textOf(r.deviceId),
      deviceName: textOf(r.deviceName),
      platform: platformOf(r.platform),
      osVersion: textOf(r.osVersion),
      agentVersion: textOf(r.agentVersion),
      trackedMinutes: minutesOf(r.trackedMinutes),
      lastActivityAt: last,
      presence: null,
    });
  }
  for (const p of recordsOf(presence)) {
    if (typeof p.userId !== "string" || !textOf(p.deviceId)) continue;
    const key = deviceKey(p.deviceId, p.userId);
    if (options.knownOnly && !lines.has(key)) continue;
    const seen = utcInstant(p.lastSeenAt);
    const state = {
      status: textOf(p.status) ?? "stopped",
      online: seen !== null && at - seen <= DEVICE_ONLINE_SECONDS * 1000,
      lastSeenAt: seen,
      statusSince: utcInstant(p.statusSince),
      projectTitle: textOf(p.projectTitle),
      currentApp: textOf(p.currentApp),
    };
    const line = lines.get(key) ?? {
      key,
      userId: p.userId,
      userName: null,
      deviceId: textOf(p.deviceId),
      deviceName: null,
      platform: null,
      osVersion: null,
      agentVersion: null,
      trackedMinutes: 0,
      lastActivityAt: null,
      presence: null,
    };
    // Two heartbeats for one device: keep the newer.
    if (line.presence && (line.presence.lastSeenAt ?? 0) >= (seen ?? 0)) continue;
    line.userName = textOf(p.name) ?? textOf(p.userName) ?? line.userName;
    line.deviceName = textOf(p.deviceName) ?? line.deviceName;
    line.platform = platformOf(p.platform) ?? line.platform;
    line.osVersion = textOf(p.osVersion) ?? line.osVersion;
    line.agentVersion = textOf(p.agentVersion) ?? line.agentVersion;
    line.presence = state;
    lines.set(key, line);
  }
  return [...lines.values()].sort(
    (a, b) =>
      Number(b.presence?.online ?? false) - Number(a.presence?.online ?? false) ||
      b.trackedMinutes - a.trackedMinutes ||
      (b.lastActivityAt ?? 0) - (a.lastActivityAt ?? 0) ||
      a.key.localeCompare(b.key),
  );
}

/** Compare dotted versions numerically ("0.10.0" > "0.9.3"); a missing part counts as 0, text after the numbers is ignored. */
export function compareVersions(a: string, b: string): number {
  const parts = (v: string) => (v.match(/\d+/g) ?? []).map(Number);
  const pa = parts(a);
  const pb = parts(b);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d !== 0) return d < 0 ? -1 : 1;
  }
  return 0;
}

/** The newest agent version among the devices; null when none reported one. */
export function latestAgentVersion(lines: readonly Pick<DeviceLine, "agentVersion">[]): string | null {
  let latest: string | null = null;
  for (const l of lines) {
    if (l.agentVersion && /\d/.test(l.agentVersion) && (latest === null || compareVersions(l.agentVersion, latest) > 0)) latest = l.agentVersion;
  }
  return latest;
}

/* ----------------------------- Access ----------------------------- */

/** What the viewer may see in the reports (contract §6: the server scopes data the same way). */
export interface ReportAccess {
  /** Tracker.Activities.ViewAll: reports cover every member; without it the server reports only the viewer. */
  viewAll: boolean;
  /** Tracker.Team.Manage: hourly rates, so pay and labour cost. */
  manageTeam: boolean;
}

/** The backend permissions the reports depend on (the names live-identity's PERMISSION uses too). */
export const REPORT_PERMISSIONS = {
  viewAll: "Tracker.Activities.ViewAll",
  manageTeam: "Tracker.Team.Manage",
} as const;

/** `auth.grantedPolicies` of the ABP application configuration; null when it isn't there. */
export function grantedPoliciesOf(config: unknown): Record<string, boolean> | null {
  const auth = config && typeof config === "object" ? (config as { auth?: unknown }).auth : null;
  const policies = auth && typeof auth === "object" ? (auth as { grantedPolicies?: unknown }).grantedPolicies : null;
  return policies && typeof policies === "object" && !Array.isArray(policies) ? (policies as Record<string, boolean>) : null;
}

/**
 * The viewer's report access: from their granted permissions when known, else
 * from their role (owners and admins hold both by default, members neither).
 */
export function reportAccessFor(role: Role, policies: Record<string, boolean> | null | undefined): ReportAccess {
  if (policies) {
    return {
      viewAll: policies[REPORT_PERMISSIONS.viewAll] === true,
      manageTeam: policies[REPORT_PERMISSIONS.manageTeam] === true,
    };
  }
  const manager = role === "owner" || role === "admin";
  return { viewAll: manager, manageTeam: manager };
}
