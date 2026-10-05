import type { Activity, WindowInfo } from "./types";
import { END_REASON_LABELS, normalizeActivityNote, platformLabel, type ActivityEndReason, type AgentPlatform } from "./api-contract";
import { OTHER_APPS_NAME, appColor, appKey, friendlyAppName } from "./apps";
import { normalizeUtcIso, parseTimeline, parseWindowList } from "./live-dataset";
import { formatUtcOffset, isValidTimeZone, normalizeTimeZone, startOfZonedDay, utcOffsetMinutes } from "./time-zone";
import {
  aggregateTracked,
  apportionPercent,
  blockIdleMinutes,
  optionalPercent,
  trackedRowFromActivity,
  type MemberDayAggregate,
  type MemberProjectAggregate,
  type ReportAggregates,
  type TrackedRow,
} from "./work-report";

/* ============================================================================
 * WORK ANALYTICS — pure derivations behind the analytics report's charts.
 *
 * Two sources feed it. The period's TOTALS — hours, activity, idle time,
 * sessions, days, members, projects, who works on what, the hour-of-week
 * rhythm — come from the reporting endpoints (in demo mode: the same figures
 * computed from the seeded blocks by the backend's rules, see work-report.ts).
 * The DISTRIBUTIONS — focus stretches, activity levels, apps, a day's timeline
 * — need the tracked blocks themselves, which a live session pages in up to
 * MAX_BLOCKS, newest first: past that, those charts are a sample and say so.
 *
 * Time is read in the reporting zone (contract §6): a block belongs to the
 * day it STARTED on, as the backend books it, while hour-of-day charts split
 * blocks at the zone's hour boundaries, as /reporting/hourly does. Start and
 * finish times are read on each member's own clock (the offset their agent
 * reported), so a Dhaka morning reads 09:00 to a manager in London.
 *
 * Activity that was not measured is null: it is left out of every average,
 * histogram and line — never counted as 0 %.
 *
 * Pure functions of their inputs (no clock, React or network), so they are
 * unit-tested directly.
 * ========================================================================== */

const MINUTE_MS = 60_000;
const HOUR_MS = 3_600_000;
const DAY_MS = 86_400_000;
/** Zone offsets only change on quarter-hour UTC instants, so one is read per 15-minute slot. */
const SLOT_MS = 15 * MINUTE_MS;

/** Blocks read per request (ABP's largest page). */
export const BLOCK_PAGE = 1000;
/** Upper bound on blocks loaded for one period's distributions (the newest are kept). */
export const MAX_BLOCKS = 5000;

const WEEKDAY_NAMES = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] as const;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"] as const;

/** Monday-first weekday names, matching {@link ZoneCalendar.weekday}. */
export const WEEKDAYS: readonly string[] = WEEKDAY_NAMES;

/* ----------------------------- Zone calendar ----------------------------- */

/** Calendar readings of instants in one IANA zone (fast: the zone's offset is cached per quarter hour). */
export interface ZoneCalendar {
  readonly timeZone: string;
  /** The zone's UTC offset at an instant, in minutes (+360 for Dhaka). */
  offsetAt(ms: number): number;
  /** The calendar day ("YYYY-MM-DD") an instant falls on. */
  dayKey(ms: number): string;
  /** Minutes after local midnight, 0 ≤ m < 1440. */
  minuteOfDay(ms: number): number;
  /** Monday = 0 … Sunday = 6. */
  weekday(ms: number): number;
  /** The instant a day begins (exact across DST). */
  dayStart(day: string): number;
  /** The start of the local hour holding `ms`. */
  hourFloor(ms: number): number;
  /** The first local hour boundary at or after `ms`. */
  hourCeil(ms: number): number;
}

const calendars = new Map<string, ZoneCalendar>();

/** The calendar of a zone; an unknown zone reads as UTC, as the server does. */
export function zoneCalendar(timeZone: string): ZoneCalendar {
  const zone = isValidTimeZone(timeZone) ? timeZone : "UTC";
  const cached = calendars.get(zone);
  if (cached) return cached;
  const offsets = new Map<number, number>();
  const offsetAt = (ms: number): number => {
    const slot = Math.floor(ms / SLOT_MS);
    let offset = offsets.get(slot);
    if (offset === undefined) {
      if (offsets.size > 200_000) offsets.clear();
      offset = utcOffsetMinutes(slot * SLOT_MS, zone);
      offsets.set(slot, offset);
    }
    return offset;
  };
  const local = (ms: number) => ms + offsetAt(ms) * MINUTE_MS;
  const sinceMidnight = (localMs: number) => ((localMs % DAY_MS) + DAY_MS) % DAY_MS;
  const calendar: ZoneCalendar = {
    timeZone: zone,
    offsetAt,
    dayKey: (ms) => new Date(local(ms)).toISOString().slice(0, 10),
    minuteOfDay: (ms) => sinceMidnight(local(ms)) / MINUTE_MS,
    weekday: (ms) => (new Date(local(ms)).getUTCDay() + 6) % 7,
    dayStart: (day) => startOfZonedDay(day, zone).getTime(),
    hourFloor: (ms) => {
      const l = local(ms);
      return ms - (((l % HOUR_MS) + HOUR_MS) % HOUR_MS);
    },
    hourCeil: (ms) => {
      const l = local(ms);
      const into = ((l % HOUR_MS) + HOUR_MS) % HOUR_MS;
      return into === 0 ? ms : ms + (HOUR_MS - into);
    },
  };
  calendars.set(zone, calendar);
  return calendar;
}

/* ----------------------------- Labels ----------------------------- */

function keyDate(key: string): Date {
  return new Date(Date.UTC(Number(key.slice(0, 4)), Number(key.slice(5, 7)) - 1, Number(key.slice(8, 10))));
}

/** Monday = 0 … Sunday = 6 of a day key. */
export function weekdayOfKey(key: string): number {
  return (keyDate(key).getUTCDay() + 6) % 7;
}

/** "Mon 28" (short) or "28 Sep" (dense). */
export function dayLabel(key: string, dense = false): string {
  const d = keyDate(key);
  return dense ? `${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]}` : `${WEEKDAY_NAMES[weekdayOfKey(key)]} ${d.getUTCDate()}`;
}

/** "Mon, 28 Sep". */
export function longDayLabel(key: string): string {
  const d = keyDate(key);
  return `${WEEKDAY_NAMES[weekdayOfKey(key)]}, ${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]}`;
}

/** "09:30" for minutes after midnight (1440 reads "24:00"). */
export function clockLabel(minutesOfDay: number): string {
  const m = Math.round(Math.min(1440, Math.max(0, minutesOfDay)));
  return `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
}

/** "09:00–10:00". */
export function hourRangeLabel(hour: number): string {
  return `${clockLabel(hour * 60)}–${clockLabel((hour + 1) * 60)}`;
}

/* ----------------------------- Blocks ----------------------------- */

export interface AppTime {
  /** The app as the agent named it ("chrome.exe", "Google Chrome"); see lib/apps for its display name. */
  name: string;
  seconds: number;
}

/** One tracked block, sanitised. Times are epoch milliseconds. */
export interface Block extends TrackedRow {
  id: string;
  apps: AppTime[];
  /** The task note, as the server stores it; null without one. */
  note: string | null;
  /** Why the block ended (newer agents); null when unknown. */
  endReason: ActivityEndReason | null;
  /** The device's UTC offset at the block's start, in minutes, when its agent reported it. */
  utcOffset: number | null;
  /** The device's IANA zone, when reported and known to this browser. */
  timeZone: string | null;
  platform: AgentPlatform | null;
  deviceId: string | null;
  deviceName: string | null;
  osVersion: string | null;
  agentVersion: string | null;
}

function finite(n: unknown): number {
  const v = typeof n === "number" ? n : typeof n === "string" && n.trim() !== "" ? Number(n) : Number.NaN;
  return Number.isFinite(v) ? v : 0;
}

function shortText(value: unknown, max: number): string | null {
  if (typeof value !== "string") return null;
  const text = value.trim();
  return text ? text.slice(0, max) : null;
}

function endReasonOf(value: unknown): ActivityEndReason | null {
  return typeof value === "string" && Object.prototype.hasOwnProperty.call(END_REASON_LABELS, value) ? (value as ActivityEndReason) : null;
}

function platformOf(value: unknown): AgentPlatform | null {
  const p = typeof value === "string" ? value.trim().toLowerCase() : "";
  return p === "windows" || p === "macos" ? p : null;
}

/** A UTC offset in minutes within the contract's −840…840, or null. */
function offsetOf(value: unknown): number | null {
  const n = typeof value === "number" ? value : typeof value === "string" && value.trim() !== "" ? Number(value) : Number.NaN;
  if (!Number.isFinite(n)) return null;
  const minutes = Math.round(n);
  return minutes >= -840 && minutes <= 840 ? minutes : null;
}

function appsOf(windows: readonly WindowInfo[] | undefined): AppTime[] {
  const apps: AppTime[] = [];
  for (const w of Array.isArray(windows) ? windows : []) {
    const seconds = finite(w?.seconds);
    const name = typeof w?.appName === "string" ? w.appName.trim() : "";
    if (name && seconds > 0) apps.push({ name, seconds });
  }
  return apps;
}

/** The fields a block may carry beyond what lib/types' Activity declares (newer datasets fill them in). */
function extrasOf(source: Record<string, unknown>) {
  return {
    endReason: endReasonOf(source.endReason),
    utcOffset: offsetOf(source.utcOffsetMinutes),
    timeZone: normalizeTimeZone(source.timeZone),
    platform: platformOf(source.platform),
    deviceId: shortText(source.deviceId, 64),
    deviceName: shortText(source.deviceName, 128),
    osVersion: shortText(source.osVersion, 64),
    agentVersion: shortText(source.agentVersion, 32),
  };
}

/**
 * Demo and tenant rows as blocks, oldest first. Rows with unreadable or
 * non-positive times are dropped (they carry no time to analyse); scores are
 * clamped to 0–100, a missing score is "not measured", and counts are
 * non-negative. Idle time follows work-report's blockIdleMinutes.
 */
export function toBlocks(list: readonly Activity[]): Block[] {
  const out: Block[] = [];
  for (const a of list) {
    const row = trackedRowFromActivity(a);
    if (!row) continue;
    const source = a as unknown as Record<string, unknown>;
    // A row that carries its own `note` (null without one) keeps `description` as a headline only.
    const note = Object.prototype.hasOwnProperty.call(source, "note")
      ? normalizeActivityNote(typeof source.note === "string" ? source.note : null)
      : normalizeActivityNote(a.description);
    out.push({
      ...row,
      id: String(a.id ?? ""),
      apps: appsOf(a.activeWindows),
      note,
      ...extrasOf(source),
    });
  }
  return out.sort((x, y) => x.start - y.start);
}

/**
 * GET /api/app/activity items (ActivityDto) as blocks, oldest first, read
 * field by field: a null activity % stays "not measured", the device and its
 * clock come along, and the same block read twice (offset paging over a list
 * that grows while it is read) counts once. A legacy block reports no
 * per-window time: its focused (first) window is credited with the whole
 * block, as the backend's app-usage report does.
 */
export function blocksFromApi(rows: readonly unknown[]): Block[] {
  const out: Block[] = [];
  const seen = new Set<string>();
  for (const raw of Array.isArray(rows) ? rows : []) {
    if (!raw || typeof raw !== "object") continue;
    const dto = raw as Record<string, unknown>;
    const id = typeof dto.id === "string" ? dto.id.trim() : typeof dto.id === "number" ? String(dto.id) : "";
    if (!id || seen.has(id)) continue;
    const startIso = normalizeUtcIso(dto.startedAt);
    const endIso = normalizeUtcIso(dto.endedAt);
    if (!startIso || !endIso) continue;
    const start = Date.parse(startIso);
    const end = Date.parse(endIso);
    if (!(end > start)) continue;
    seen.add(id);
    const minutes = (end - start) / MINUTE_MS;
    const activity = optionalPercent(dto.productivity);
    const windows = parseWindowList(dto.activeWindowsJson);
    if (windows.length > 0 && windows.every((w) => w.seconds === 0)) windows[0] = { ...windows[0], seconds: minutes * 60 };
    out.push({
      id,
      userId: String(dto.userId ?? ""),
      projectId: String(dto.projectId ?? ""),
      start,
      end,
      minutes,
      activity,
      idleMinutes: blockIdleMinutes({ minutes, idleSeconds: dto.idleSeconds, timeline: parseTimeline(dto.timelineJson), activity }),
      keys: Math.max(0, finite(dto.keyboardHits)),
      clicks: Math.max(0, finite(dto.mouseClicks)),
      apps: appsOf(windows),
      note: normalizeActivityNote(typeof dto.description === "string" ? dto.description : null),
      ...extrasOf(dto),
    });
  }
  return out.sort((x, y) => x.start - y.start);
}

/** The blocks that STARTED in [from, to) — the backend's rule for which blocks a period holds. */
export function blocksStartingIn(blocks: readonly Block[], from: number, to: number): Block[] {
  return blocks.filter((b) => b.start >= from && b.start < to);
}

/** The reporting aggregates of a set of blocks (demo mode), each booked to the day it started on in the zone. */
export function aggregateBlocks(blocks: readonly Block[], calendar: ZoneCalendar): ReportAggregates {
  return aggregateTracked(blocks, calendar.dayKey);
}

/**
 * Whether a set of loaded blocks covers `day` completely: when a period has
 * more blocks than were loaded (newest first), its oldest days are missing.
 */
export function coversDay(blocks: readonly Block[], truncated: boolean, day: string, calendar: ZoneCalendar): boolean {
  if (!truncated) return true;
  if (!blocks.length) return false;
  return day > calendar.dayKey(blocks[0].start);
}

/* ----------------------------- Hours ----------------------------- */

export interface HourPiece {
  day: string;
  /** Monday = 0 … Sunday = 6. */
  weekday: number;
  hour: number;
  /** Minutes of the block inside this local hour. */
  minutes: number;
  /** Minutes after local midnight where the piece starts / ends. */
  from: number;
  to: number;
}

/**
 * Split [start, end) into pieces that never cross a local hour boundary of
 * the zone (so never a day boundary either). A DST change inside an hour ends
 * the piece there.
 */
export function hourPieces(start: number, end: number, calendar: ZoneCalendar): HourPiece[] {
  const pieces: HourPiece[] = [];
  let cursor = start;
  let guard = 0;
  while (cursor < end && guard++ < 24 * 400) {
    const offset = calendar.offsetAt(cursor) * MINUTE_MS;
    const local = cursor + offset;
    let next = (Math.floor(local / HOUR_MS) + 1) * HOUR_MS - offset;
    for (let t = (Math.floor(cursor / SLOT_MS) + 1) * SLOT_MS; t < next; t += SLOT_MS) {
      if (calendar.offsetAt(t) * MINUTE_MS !== offset) {
        next = t;
        break;
      }
    }
    const pieceEnd = Math.min(next, end);
    const midnight = local - (((local % DAY_MS) + DAY_MS) % DAY_MS);
    const date = new Date(local);
    pieces.push({
      day: date.toISOString().slice(0, 10),
      weekday: (date.getUTCDay() + 6) % 7,
      hour: date.getUTCHours(),
      minutes: (pieceEnd - cursor) / MINUTE_MS,
      from: (local - midnight) / MINUTE_MS,
      to: (pieceEnd + offset - midnight) / MINUTE_MS,
    });
    cursor = pieceEnd;
  }
  return pieces;
}

/** Time in one weekday × hour slot of the reporting zone. */
export interface HourCell {
  /** Monday = 0 … Sunday = 6. */
  weekday: number;
  hour: number;
  minutes: number;
}

/**
 * GET /api/app/reporting/hourly: tracked minutes per weekday (0 = Sunday on
 * the wire) and hour of the requested zone. Rows out of range are dropped.
 */
export function parseHourly(payload: unknown): HourCell[] {
  if (!Array.isArray(payload)) return [];
  const cells: HourCell[] = [];
  for (const raw of payload) {
    if (!raw || typeof raw !== "object") continue;
    const r = raw as Record<string, unknown>;
    const day = Number(r.dayOfWeek);
    const hour = Number(r.hour);
    const minutes = finite(r.trackedMinutes);
    if (!Number.isInteger(day) || day < 0 || day > 6 || !Number.isInteger(hour) || hour < 0 || hour > 23 || minutes <= 0) continue;
    cells.push({ weekday: (day + 6) % 7, hour, minutes });
  }
  return cells;
}

/** The same cells from blocks (demo mode, or a server without /reporting/hourly): blocks split at hour boundaries. */
export function hourCellsFromBlocks(blocks: readonly Block[], calendar: ZoneCalendar): HourCell[] {
  const grid = new Map<number, number>();
  for (const b of blocks) {
    for (const p of hourPieces(b.start, b.end, calendar)) {
      const key = p.weekday * 24 + p.hour;
      grid.set(key, (grid.get(key) ?? 0) + p.minutes);
    }
  }
  return [...grid.entries()].map(([key, minutes]) => ({ weekday: Math.floor(key / 24), hour: key % 24, minutes }));
}

/** Minutes worked per weekday (rows, Monday first) × hour (columns). */
export function rhythmMatrix(cells: readonly HourCell[]): number[][] {
  const grid = Array.from({ length: 7 }, () => Array<number>(24).fill(0));
  for (const c of cells) grid[c.weekday][c.hour] += c.minutes;
  return grid;
}

export interface HourStat {
  hour: number;
  minutes: number;
  /** Activity of the measured minutes, 0–100; null when none were measured. */
  activity: number | null;
  keys: number;
  clicks: number;
}

/** Minutes per hour of day, from the weekday × hour cells (no activity: the cells don't carry it). */
export function hourTotals(cells: readonly HourCell[]): HourStat[] {
  const totals = Array.from({ length: 24 }, (_, hour) => ({ hour, minutes: 0, activity: null as number | null, keys: 0, clicks: 0 }));
  for (const c of cells) totals[c.hour].minutes += c.minutes;
  return totals;
}

/** Totals per hour of day of the reporting zone, from blocks: time, measured activity and input. */
export function hourProfile(blocks: readonly Block[], calendar: ZoneCalendar): HourStat[] {
  const acc = Array.from({ length: 24 }, (_, hour) => ({ hour, minutes: 0, weighted: 0, measured: 0, keys: 0, clicks: 0 }));
  for (const b of blocks) {
    for (const p of hourPieces(b.start, b.end, calendar)) {
      const e = acc[p.hour];
      const share = b.minutes > 0 ? p.minutes / b.minutes : 0;
      e.minutes += p.minutes;
      if (b.activity !== null) {
        e.weighted += p.minutes * b.activity;
        e.measured += p.minutes;
      }
      e.keys += b.keys * share;
      e.clicks += b.clicks * share;
    }
  }
  return acc.map((e) => ({
    hour: e.hour,
    minutes: e.minutes,
    activity: e.measured > 0 ? Math.round(e.weighted / e.measured) : null,
    keys: Math.round(e.keys),
    clicks: Math.round(e.clicks),
  }));
}

/** The hour with the most time (earliest on a tie); null without time. */
export function peakHour(profile: readonly { hour: number; minutes: number }[]): number | null {
  let best: { hour: number; minutes: number } | null = null;
  for (const h of profile) if (h.minutes > 0 && (!best || h.minutes > best.minutes)) best = h;
  return best ? best.hour : null;
}

/** The Monday-first weekday with the most time (earliest on a tie); null without time. */
export function busiestWeekday(cells: readonly HourCell[]): number | null {
  const totals = rhythmMatrix(cells).map((row) => row.reduce((s, v) => s + v, 0));
  let best: number | null = null;
  totals.forEach((v, i) => {
    if (v > 0 && (best === null || v > totals[best])) best = i;
  });
  return best;
}

/* ----------------------------- Daily ----------------------------- */

export interface DayStat {
  day: string;
  label: string;
  weekend: boolean;
  /** Minutes worked (all members). */
  minutes: number;
  /** Activity of the measured minutes, 0–100; null when none were measured. */
  activity: number | null;
  /** Minutes without input; null when unknown. */
  idleMinutes: number | null;
  /** Blocks that started this day. */
  sessions: number;
  keys: number;
  clicks: number;
  /** Distinct members with time this day. */
  members: number;
}

/** One row per day of the period from the aggregates; days without a row read as empty. */
export function dayStats(agg: ReportAggregates, days: readonly string[]): DayStat[] {
  const byDay = new Map(agg.daily.map((d) => [d.day, d]));
  const members = new Map<string, Set<string>>();
  for (const md of agg.memberDays) {
    if (md.minutes <= 0) continue;
    const set = members.get(md.day) ?? new Set<string>();
    set.add(md.userId);
    members.set(md.day, set);
  }
  return days.map((day) => {
    const d = byDay.get(day);
    const minutes = d?.minutes ?? 0;
    return {
      day,
      label: dayLabel(day),
      weekend: weekdayOfKey(day) >= 5,
      minutes,
      activity: minutes > 0 ? (d?.activity ?? null) : null,
      idleMinutes: d ? d.idleMinutes : null,
      sessions: d?.sessions ?? 0,
      keys: d?.keys ?? 0,
      clicks: d?.clicks ?? 0,
      members: members.get(day)?.size ?? 0,
    };
  });
}

/**
 * Trailing mean over up to `window` values. `seed` holds the values just
 * before the first (oldest first), so the first days already average a full
 * window instead of restarting with every period.
 */
export function movingAverage(values: readonly number[], window = 7, seed: readonly number[] = []): number[] {
  const size = Math.max(1, Math.floor(window));
  const all = [...seed, ...values];
  return values.map((_, i) => {
    const at = seed.length + i;
    const slice = all.slice(Math.max(0, at - size + 1), at + 1);
    return slice.reduce((s, v) => s + v, 0) / slice.length;
  });
}

/** Running totals. */
export function cumulative(values: readonly number[]): number[] {
  let sum = 0;
  return values.map((v) => (sum += v));
}

/**
 * The pace line: the cumulative target if every weekday met `perWeekday`
 * minutes (weekends add nothing).
 */
export function paceTarget(days: readonly { weekend: boolean }[], perWeekday: number): number[] {
  return cumulative(days.map((d) => (d.weekend ? 0 : Math.max(0, perWeekday))));
}

/* ----------------------------- Local clocks ----------------------------- */

/** How a member's own clock reads an instant. */
export interface LocalReading {
  /** Minutes after the member's local midnight, 0 ≤ m < 1440. */
  minuteOfDay: number;
  /** The member's UTC offset at that instant, in minutes. */
  offset: number;
  /** No zone is known for the member: the reporting zone stands in. */
  estimated: boolean;
}

export type MemberClock = (userId: string, at: number) => LocalReading;

/**
 * Each member's local clock: the UTC offset their agent reported on the
 * block nearest before the instant (else their first block); else the zone
 * their agent reported; else `zoneOf` (a roster's time zone, e.g. the demo
 * people's); else the reporting zone, flagged as an estimate.
 */
export function memberClock(blocks: readonly Block[], fallback: ZoneCalendar, zoneOf?: (userId: string) => string | null | undefined): MemberClock {
  const offsets = new Map<string, { start: number; offset: number }[]>();
  const zones = new Map<string, string>();
  for (const b of [...blocks].sort((x, y) => x.start - y.start)) {
    if (b.utcOffset !== null) {
      const list = offsets.get(b.userId) ?? [];
      list.push({ start: b.start, offset: b.utcOffset });
      offsets.set(b.userId, list);
    }
    if (b.timeZone) zones.set(b.userId, b.timeZone);
  }
  const reading = (at: number, offset: number, estimated: boolean): LocalReading => ({
    minuteOfDay: ((((at + offset * MINUTE_MS) % DAY_MS) + DAY_MS) % DAY_MS) / MINUTE_MS,
    offset,
    estimated,
  });
  return (userId, at) => {
    const list = offsets.get(userId);
    if (list?.length) {
      let lo = 0;
      let hi = list.length - 1;
      let found = 0;
      while (lo <= hi) {
        const mid = (lo + hi) >> 1;
        if (list[mid].start <= at) {
          found = mid;
          lo = mid + 1;
        } else hi = mid - 1;
      }
      return reading(at, list[found].offset, false);
    }
    const zone = zones.get(userId) ?? normalizeTimeZone(zoneOf?.(userId));
    if (zone) return reading(at, zoneCalendar(zone).offsetAt(at), false);
    return reading(at, fallback.offsetAt(at), true);
  };
}

export interface WorkWindow {
  day: string;
  label: string;
  weekend: boolean;
  /**
   * The earliest start and latest finish of the day, in minutes after the
   * members' own local midnight (each member on their own clock); null on days
   * without time. A finish past local midnight reads 1440.
   */
  start: number | null;
  end: number | null;
  minutes: number;
  /** Some member's clock had to be estimated (no zone reported). */
  estimated: boolean;
}

/** A member-day's start and finish on the member's own clock; null when the day's bounds are unknown. */
export function localSpan(md: MemberDayAggregate, clock: MemberClock): { start: number; end: number; estimated: boolean } | null {
  if (md.firstStart === null || md.lastEnd === null || md.lastEnd <= md.firstStart) return null;
  const first = clock(md.userId, md.firstStart);
  const span = (md.lastEnd - md.firstStart) / MINUTE_MS;
  return { start: first.minuteOfDay, end: Math.min(1440, first.minuteOfDay + span), estimated: first.estimated };
}

/**
 * A member's first start and last finish of a day, on their own clock:
 * "08:58–17:42", or "08:58–17:42 local (UTC−7)" when their clock isn't the
 * report's. Null when the day's bounds are unknown.
 */
export function localSpanLabel(clock: MemberClock, calendar: ZoneCalendar, userId: string, firstStart: number | null, lastEnd: number | null): string | null {
  if (firstStart === null || lastEnd === null || lastEnd <= firstStart) return null;
  const first = clock(userId, firstStart);
  const end = Math.min(1440, first.minuteOfDay + (lastEnd - firstStart) / MINUTE_MS);
  const span = `${clockLabel(first.minuteOfDay)}–${clockLabel(end)}`;
  return first.estimated || first.offset === calendar.offsetAt(firstStart) ? span : `${span} local (${formatUtcOffset(first.offset)})`;
}

/**
 * Fill in member-days whose first start / last end the source didn't send (an
 * older server) from the loaded blocks of that member and day.
 */
export function withBlockBounds(memberDays: readonly MemberDayAggregate[], blocks: readonly Block[], calendar: ZoneCalendar): MemberDayAggregate[] {
  if (memberDays.every((d) => d.firstStart !== null && d.lastEnd !== null)) return [...memberDays];
  const bounds = new Map<string, { first: number; last: number }>();
  for (const b of blocks) {
    const key = `${b.userId}\u0000${calendar.dayKey(b.start)}`;
    const e = bounds.get(key);
    if (!e) bounds.set(key, { first: b.start, last: b.end });
    else {
      e.first = Math.min(e.first, b.start);
      e.last = Math.max(e.last, b.end);
    }
  }
  return memberDays.map((d) => {
    if (d.firstStart !== null && d.lastEnd !== null) return d;
    const e = bounds.get(`${d.userId}\u0000${d.day}`);
    return e ? { ...d, firstStart: d.firstStart ?? e.first, lastEnd: d.lastEnd ?? e.last } : d;
  });
}

/** Each day's working window — the earliest start to the latest finish, every member on their own clock. */
export function workWindows(memberDays: readonly MemberDayAggregate[], days: readonly string[], clock: MemberClock): WorkWindow[] {
  const acc = new Map<string, { start: number; end: number; minutes: number; estimated: boolean }>();
  for (const md of memberDays) {
    const span = localSpan(md, clock);
    if (!span || md.minutes <= 0) continue;
    const e = acc.get(md.day);
    if (!e) acc.set(md.day, { start: span.start, end: span.end, minutes: md.minutes, estimated: span.estimated });
    else {
      e.start = Math.min(e.start, span.start);
      e.end = Math.max(e.end, span.end);
      e.minutes += md.minutes;
      e.estimated = e.estimated || span.estimated;
    }
  }
  return days.map((day) => {
    const e = acc.get(day);
    return {
      day,
      label: dayLabel(day),
      weekend: weekdayOfKey(day) >= 5,
      start: e ? e.start : null,
      end: e ? e.end : null,
      minutes: e ? e.minutes : 0,
      estimated: e?.estimated ?? false,
    };
  });
}

/** Mean of the non-null values, or null. */
export function meanOf(values: readonly (number | null)[]): number | null {
  const v = values.filter((x): x is number => x !== null && Number.isFinite(x));
  return v.length ? v.reduce((s, x) => s + x, 0) / v.length : null;
}

/* ----------------------------- Focus ----------------------------- */

export interface Stretch {
  userId: string;
  start: number;
  end: number;
  /** Tracked minutes inside the stretch (gaps between merged blocks excluded). */
  minutes: number;
  blocks: number;
}

/**
 * Continuous stretches of work per member: consecutive blocks separated by at
 * most `maxGapMinutes` are one stretch (the agent cuts a new block every
 * interval, so back-to-back blocks are one sitting).
 */
export function focusStretches(blocks: readonly Block[], maxGapMinutes = 5): Stretch[] {
  const byUser = new Map<string, Block[]>();
  for (const b of blocks) {
    const list = byUser.get(b.userId) ?? [];
    list.push(b);
    byUser.set(b.userId, list);
  }
  const out: Stretch[] = [];
  const gap = Math.max(0, maxGapMinutes) * MINUTE_MS;
  for (const [userId, list] of byUser) {
    list.sort((a, b) => a.start - b.start);
    let current: Stretch | null = null;
    for (const b of list) {
      if (current && b.start - current.end <= gap) {
        current.end = Math.max(current.end, b.end);
        current.minutes += b.minutes;
        current.blocks += 1;
      } else {
        if (current) out.push(current);
        current = { userId, start: b.start, end: b.end, minutes: b.minutes, blocks: 1 };
      }
    }
    if (current) out.push(current);
  }
  return out.sort((a, b) => a.start - b.start);
}

export const STRETCH_BUCKETS: readonly { label: string; max: number }[] = [
  { label: "Under 30m", max: 30 },
  { label: "30–60m", max: 60 },
  { label: "1–2h", max: 120 },
  { label: "2–4h", max: 240 },
  { label: "4h+", max: Number.POSITIVE_INFINITY },
];

/** How many stretches (and how much time) fall in each length bucket. */
export function stretchHistogram(stretches: readonly Stretch[]): { label: string; count: number; minutes: number }[] {
  const rows = STRETCH_BUCKETS.map((b) => ({ label: b.label, count: 0, minutes: 0 }));
  for (const s of stretches) {
    const i = STRETCH_BUCKETS.findIndex((b) => s.minutes < b.max);
    const row = rows[i < 0 ? rows.length - 1 : i];
    row.count += 1;
    row.minutes += s.minutes;
  }
  return rows;
}

/* ----------------------------- Activity ----------------------------- */

/** Tracked minutes by measured activity, in ten 10-point bins (the last includes 100). Unmeasured blocks are left out. */
export function activityHistogram(blocks: readonly Block[]): { label: string; from: number; minutes: number; blocks: number }[] {
  const rows = Array.from({ length: 10 }, (_, i) => ({
    label: i === 9 ? "90–100%" : `${i * 10}–${i * 10 + 9}%`,
    from: i * 10,
    minutes: 0,
    blocks: 0,
  }));
  for (const b of blocks) {
    if (b.activity === null) continue;
    const i = Math.min(9, Math.floor(b.activity / 10));
    rows[i].minutes += b.minutes;
    rows[i].blocks += 1;
  }
  return rows;
}

/** Minutes of blocks whose activity was not measured. */
export function unmeasuredMinutes(blocks: readonly Block[]): number {
  return blocks.reduce((s, b) => s + (b.activity === null ? b.minutes : 0), 0);
}

/* ----------------------------- Members ----------------------------- */

export interface MemberStat {
  userId: string;
  /** A name the source knows (the server's); the roster's name wins when there is one. */
  name: string | null;
  minutes: number;
  activity: number | null;
  idleMinutes: number | null;
  sessions: number;
  daysActive: number;
  /** Minutes on each day of the period (for a sparkline). */
  daily: number[];
  /** Mean / longest stretch length in minutes; null when none of their blocks were loaded. */
  avgStretch: number | null;
  longestStretch: number | null;
  keys: number;
  clicks: number;
  /** Key presses + clicks per tracked hour of their loaded blocks; null without any. */
  inputPerHour: number | null;
  /**
   * How steady the member's daily hours are, 0–100 (100 = the same every
   * worked day); null with fewer than two worked days.
   */
  consistency: number | null;
  /** Average first start / last finish on worked days, minutes after the member's local midnight. */
  typicalStart: number | null;
  typicalEnd: number | null;
  /** No zone is known for the member, so `typicalStart`/`typicalEnd` are read in the reporting zone. */
  localEstimated: boolean;
}

/**
 * Per-member figures, most time first: hours, activity, idle time, sessions
 * and days from the aggregates (exact), focus stretches and input from the
 * loaded blocks, start and finish on the member's own clock.
 */
export function memberStats(agg: ReportAggregates, blocks: readonly Block[], days: readonly string[], clock: MemberClock): MemberStat[] {
  const own = new Map<string, Stretch[]>();
  for (const s of focusStretches(blocks)) {
    const list = own.get(s.userId);
    if (list) list.push(s);
    else own.set(s.userId, [s]);
  }
  const input = new Map<string, { keys: number; clicks: number; minutes: number }>();
  for (const b of blocks) {
    const e = input.get(b.userId) ?? { keys: 0, clicks: 0, minutes: 0 };
    e.keys += b.keys;
    e.clicks += b.clicks;
    e.minutes += b.minutes;
    input.set(b.userId, e);
  }
  const dayIndex = new Map(days.map((d, i) => [d, i]));
  const perDay = new Map<string, MemberDayAggregate[]>();
  for (const md of agg.memberDays) {
    const list = perDay.get(md.userId);
    if (list) list.push(md);
    else perDay.set(md.userId, [md]);
  }

  return agg.members
    .filter((m) => m.minutes > 0)
    .map((m) => {
      const mine = perDay.get(m.userId) ?? [];
      const daily = days.map(() => 0);
      for (const md of mine) {
        const i = dayIndex.get(md.day);
        if (i !== undefined) daily[i] += md.minutes;
      }
      const worked = mine.filter((md) => md.minutes > 0);
      const workedMinutes = worked.map((md) => md.minutes);
      let consistency: number | null = null;
      if (workedMinutes.length >= 2) {
        const mean = workedMinutes.reduce((s, v) => s + v, 0) / workedMinutes.length;
        const sd = Math.sqrt(workedMinutes.reduce((s, v) => s + (v - mean) ** 2, 0) / workedMinutes.length);
        consistency = mean > 0 ? Math.round(100 * Math.max(0, 1 - sd / mean)) : null;
      }
      const spans = worked.map((md) => localSpan(md, clock)).filter((s): s is NonNullable<typeof s> => s !== null);
      const sits = own.get(m.userId) ?? [];
      const io = input.get(m.userId);
      return {
        userId: m.userId,
        name: m.name,
        minutes: m.minutes,
        activity: m.activity === null ? null : Math.round(m.activity),
        idleMinutes: m.idleMinutes,
        sessions: m.sessions,
        daysActive: worked.length,
        daily,
        avgStretch: sits.length ? sits.reduce((s, x) => s + x.minutes, 0) / sits.length : null,
        longestStretch: sits.length ? sits.reduce((mx, x) => Math.max(mx, x.minutes), 0) : null,
        keys: Math.round(io?.keys ?? 0),
        clicks: Math.round(io?.clicks ?? 0),
        inputPerHour: io && io.minutes > 0 ? Math.round(((io.keys + io.clicks) / io.minutes) * 60) : null,
        consistency,
        typicalStart: meanOf(spans.map((s) => s.start)),
        typicalEnd: meanOf(spans.map((s) => s.end)),
        localEstimated: spans.some((s) => s.estimated),
      };
    })
    .sort((a, b) => b.minutes - a.minutes || a.userId.localeCompare(b.userId));
}

export const RADAR_DIMENSIONS = ["Hours", "Activity", "Days active", "Focus", "Input", "Consistency"] as const;

/**
 * Each member's profile on comparable 0–100 scales: hours, focus and input
 * relative to the team's highest; activity and consistency as they are; days
 * active as a share of the period. A figure that wasn't measured plots at 0.
 */
export function radarRows(stats: readonly MemberStat[], dayCount: number): Record<string, number | string>[] {
  const max = (pick: (s: MemberStat) => number) => Math.max(1e-9, ...stats.map(pick));
  const hours = max((s) => s.minutes);
  const focus = max((s) => s.avgStretch ?? 0);
  const input = max((s) => s.inputPerHour ?? 0);
  const days = Math.max(1, dayCount);
  const scale = (v: number) => Math.round(Math.min(100, Math.max(0, v)));
  const value = (s: MemberStat, dim: (typeof RADAR_DIMENSIONS)[number]): number => {
    switch (dim) {
      case "Hours":
        return scale((s.minutes / hours) * 100);
      case "Activity":
        return scale(s.activity ?? 0);
      case "Days active":
        return scale((s.daysActive / days) * 100);
      case "Focus":
        return scale(((s.avgStretch ?? 0) / focus) * 100);
      case "Input":
        return scale(((s.inputPerHour ?? 0) / input) * 100);
      default:
        return scale(s.consistency ?? 0);
    }
  };
  return RADAR_DIMENSIONS.map((dimension) => {
    const row: Record<string, number | string> = { dimension };
    for (const s of stats) row[s.userId] = value(s, dimension);
    return row;
  });
}

/* ----------------------------- Projects & apps ----------------------------- */

export const OTHER = "__other__";

/**
 * Hours per day split by project, for a stacked chart: the `top` projects
 * keep their id as the key, the rest are summed under {@link OTHER}. Each
 * block counts toward the day it started on.
 */
export function projectDays(
  blocks: readonly Block[],
  days: readonly string[],
  top: readonly string[],
  calendar: ZoneCalendar,
): Record<string, number | string>[] {
  const keep = new Set(top);
  const rows = new Map<string, Record<string, number | string>>();
  for (const d of days) {
    const row: Record<string, number | string> = { day: d, label: dayLabel(d) };
    for (const id of top) row[id] = 0;
    row[OTHER] = 0;
    rows.set(d, row);
  }
  for (const b of blocks) {
    const row = rows.get(calendar.dayKey(b.start));
    if (!row) continue;
    const key = keep.has(b.projectId) ? b.projectId : OTHER;
    row[key] = Math.round(((row[key] as number) + b.minutes / 60) * 100) / 100;
  }
  return days.map((d) => rows.get(d)!);
}

/** Minutes per member per project, from blocks (when the server sent no such breakdown). */
export function memberProjectsFromBlocks(blocks: readonly Block[]): MemberProjectAggregate[] {
  const acc = new Map<string, MemberProjectAggregate>();
  for (const b of blocks) {
    const key = `${b.userId}\u0000${b.projectId}`;
    const e = acc.get(key) ?? { userId: b.userId, projectId: b.projectId, minutes: 0 };
    e.minutes += b.minutes;
    acc.set(key, e);
  }
  return [...acc.values()];
}

export interface FlowNode {
  name: string;
  kind: "member" | "project";
  id: string;
}

export interface FlowData {
  nodes: FlowNode[];
  links: { source: number; target: number; value: number }[];
}

/**
 * Who works on what, for a member → project flow diagram. Values are hours.
 * Beyond `maxMembers` / `maxProjects` the smallest are grouped as "Others".
 */
export function memberProjectFlow(
  pairs: readonly MemberProjectAggregate[],
  memberName: (id: string) => string,
  projectName: (id: string) => string,
  maxMembers = 8,
  maxProjects = 6,
): FlowData {
  const valid = pairs.filter((p) => p && p.minutes > 0 && p.userId && p.projectId);
  const memberTotals = new Map<string, number>();
  const projectTotals = new Map<string, number>();
  for (const p of valid) {
    memberTotals.set(p.userId, (memberTotals.get(p.userId) ?? 0) + p.minutes);
    projectTotals.set(p.projectId, (projectTotals.get(p.projectId) ?? 0) + p.minutes);
  }
  const ranked = (m: Map<string, number>) => [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([id]) => id);
  const members = ranked(memberTotals);
  const projects = ranked(projectTotals);
  const keptMembers = new Set(members.slice(0, members.length > maxMembers ? maxMembers - 1 : maxMembers));
  const keptProjects = new Set(projects.slice(0, projects.length > maxProjects ? maxProjects - 1 : maxProjects));

  const nodes: FlowNode[] = [];
  const index = new Map<string, number>();
  const node = (key: string, name: string, kind: FlowNode["kind"], id: string) => {
    if (!index.has(key)) {
      index.set(key, nodes.length);
      nodes.push({ name, kind, id });
    }
    return index.get(key)!;
  };
  for (const id of members) {
    if (keptMembers.has(id)) node(`m:${id}`, memberName(id), "member", id);
  }
  if (members.some((id) => !keptMembers.has(id))) node(`m:${OTHER}`, "Other members", "member", OTHER);
  for (const id of projects) {
    if (keptProjects.has(id)) node(`p:${id}`, projectName(id), "project", id);
  }
  if (projects.some((id) => !keptProjects.has(id))) node(`p:${OTHER}`, "Other projects", "project", OTHER);

  const values = new Map<string, number>();
  for (const p of valid) {
    const m = index.get(`m:${keptMembers.has(p.userId) ? p.userId : OTHER}`)!;
    const t = index.get(`p:${keptProjects.has(p.projectId) ? p.projectId : OTHER}`)!;
    const key = `${m}>${t}`;
    values.set(key, (values.get(key) ?? 0) + p.minutes);
  }
  const links = [...values.entries()]
    .map(([key, minutes]) => {
      const [source, target] = key.split(">").map(Number);
      return { source, target, value: Math.round((minutes / 60) * 100) / 100 };
    })
    .filter((l) => l.value > 0)
    .sort((a, b) => a.source - b.source || b.value - a.value);
  return { nodes, links };
}

/** Minutes of focused time per app, by its canonical name (lib/apps). */
export interface AppMinutes {
  name: string;
  minutes: number;
}

/** Focused time per app from blocks, every spelling of one app merged under its canonical name. */
export function appUsageFromBlocks(blocks: readonly Block[]): AppMinutes[] {
  const by = new Map<string, AppMinutes>();
  for (const b of blocks) {
    for (const a of b.apps) {
      const key = appKey(a.name);
      if (!key) continue;
      const e = by.get(key) ?? { name: friendlyAppName(a.name), minutes: 0 };
      e.minutes += a.seconds / 60;
      by.set(key, e);
    }
  }
  return [...by.values()];
}

export interface AppStat {
  /** Display name: the canonical name, or "Other apps". */
  app: string;
  minutes: number;
  /** Whole-percent share of the focused time; the shares add up to 100. */
  share: number;
  /** The app's colour on every page (lib/apps); slate for "Other apps". */
  color: string;
}

/** The most-used apps, most first; the rest after `top` is summed as "Other apps". */
export function appStats(rows: readonly AppMinutes[], top = 8): AppStat[] {
  const merged = new Map<string, AppMinutes>();
  for (const r of rows) {
    const key = appKey(r.name);
    if (!key || !(r.minutes > 0)) continue;
    const e = merged.get(key) ?? { name: friendlyAppName(r.name), minutes: 0 };
    e.minutes += r.minutes;
    merged.set(key, e);
  }
  const all = [...merged.values()].sort((a, b) => b.minutes - a.minutes || a.name.localeCompare(b.name));
  const head = all.slice(0, top);
  const rest = all.slice(top).reduce((s, a) => s + a.minutes, 0);
  const list = rest > 0 ? [...head, { name: OTHER_APPS_NAME, minutes: rest }] : head;
  const shares = apportionPercent(list.map((a) => a.minutes));
  return list.map((a, i) => ({ app: a.name, minutes: a.minutes, share: shares[i], color: appColor(a.name) }));
}

/* ----------------------------- Day timeline ----------------------------- */

export interface TimelineSegment {
  start: number;
  end: number;
  projectId: string;
  activity: number | null;
  idleMinutes: number | null;
  note: string | null;
}

export interface TimelineGap {
  start: number;
  end: number;
  /** Why the block before the gap ended; null when its agent didn't say. */
  reason: ActivityEndReason | null;
}

export interface TimelineLane {
  userId: string;
  segments: TimelineSegment[];
  gaps: TimelineGap[];
  /** The first start and last finish (epoch ms). */
  first: number;
  last: number;
  minutes: number;
}

export interface DayTimeline {
  day: string;
  lanes: TimelineLane[];
  /** The axis (epoch ms): whole hours of the reporting zone around the day's work. */
  from: number;
  to: number;
}

/**
 * One day's tracked time per member, as segments on a clock — the blocks
 * that started that day in the reporting zone — with the gaps between them
 * and the reason the block before each gap ended (break, idle, screen lock,
 * sleep…). Gaps shorter than `minGapMinutes` are seams, not gaps. Lanes are
 * ordered by who started first.
 */
export function dayTimeline(blocks: readonly Block[], day: string, calendar: ZoneCalendar, minGapMinutes = 1): DayTimeline {
  const byUser = new Map<string, Block[]>();
  for (const b of blocks) {
    if (calendar.dayKey(b.start) !== day) continue;
    const list = byUser.get(b.userId) ?? [];
    list.push(b);
    byUser.set(b.userId, list);
  }
  const minGap = Math.max(0, minGapMinutes) * MINUTE_MS;
  const lanes: TimelineLane[] = [];
  for (const [userId, list] of byUser) {
    list.sort((a, b) => a.start - b.start || a.end - b.end);
    const gaps: TimelineGap[] = [];
    let reach = list[0].end;
    let reason = list[0].endReason;
    for (const b of list.slice(1)) {
      if (b.start - reach >= minGap) gaps.push({ start: reach, end: b.start, reason });
      if (b.end >= reach) {
        reach = b.end;
        reason = b.endReason;
      }
    }
    lanes.push({
      userId,
      segments: list.map((b) => ({
        start: b.start,
        end: b.end,
        projectId: b.projectId,
        activity: b.activity,
        idleMinutes: b.idleMinutes,
        note: b.note,
      })),
      gaps,
      first: list[0].start,
      last: Math.max(...list.map((b) => b.end)),
      minutes: list.reduce((s, b) => s + b.minutes, 0),
    });
  }
  lanes.sort((a, b) => a.first - b.first || a.userId.localeCompare(b.userId));
  if (!lanes.length) {
    const start = calendar.dayStart(day);
    return { day, lanes, from: start, to: start + DAY_MS };
  }
  const first = Math.min(...lanes.map((l) => l.first));
  const last = Math.max(...lanes.map((l) => l.last));
  const from = calendar.hourFloor(first);
  const to = Math.max(calendar.hourCeil(last), from + HOUR_MS);
  return { day, lanes, from, to };
}

export type GapKind = "break" | "idle" | "lock" | "sleep" | "off" | "untracked";

/** How a gap reads: a short label for the bar, a full one for tooltips and screen readers, and its look. */
export function gapKind(reason: ActivityEndReason | null): { kind: GapKind; label: string; description: string } {
  switch (reason) {
    case "break":
      return { kind: "break", label: "Break", description: "On a break" };
    case "idle":
      return { kind: "idle", label: "Idle", description: "Idle — away from the keyboard and mouse" };
    case "lock":
      return { kind: "lock", label: "Locked", description: "Screen locked" };
    case "sleep":
      return { kind: "sleep", label: "Asleep", description: "Computer asleep" };
    case "pause":
    case "quit":
    case "signout":
    case "shutdown":
      return { kind: "off", label: reason === "pause" ? "Paused" : "Off", description: END_REASON_LABELS[reason] };
    default:
      return { kind: "untracked", label: "Not tracked", description: "Not tracked" };
  }
}

/* ----------------------------- Devices ----------------------------- */

export interface DeviceRow {
  userId: string;
  userName: string | null;
  deviceId: string | null;
  deviceName: string | null;
  platform: AgentPlatform | null;
  osVersion: string | null;
  agentVersion: string | null;
  /** Epoch ms of the device's last block in the period; null when unknown. */
  lastActivityAt: number | null;
  minutes: number;
}

/** GET /api/app/reporting/devices: one row per member and device. */
export function parseDevices(payload: unknown): DeviceRow[] {
  if (!Array.isArray(payload)) return [];
  const out: DeviceRow[] = [];
  for (const raw of payload) {
    if (!raw || typeof raw !== "object") continue;
    const r = raw as Record<string, unknown>;
    const userId = typeof r.userId === "string" && r.userId ? r.userId : null;
    const minutes = finite(r.trackedMinutes);
    if (!userId || minutes <= 0) continue;
    const last = normalizeUtcIso(r.lastActivityAt);
    out.push({
      userId,
      userName: shortText(r.userName, 256),
      deviceId: shortText(r.deviceId, 64),
      deviceName: shortText(r.deviceName, 128),
      platform: platformOf(r.platform),
      osVersion: shortText(r.osVersion, 64),
      agentVersion: shortText(r.agentVersion, 32),
      lastActivityAt: last ? Date.parse(last) : null,
      minutes,
    });
  }
  return out;
}

/** The same rows from blocks that carry device details (a demo dataset or a server without /reporting/devices). */
export function devicesFromBlocks(blocks: readonly Block[]): DeviceRow[] {
  const by = new Map<string, DeviceRow>();
  for (const b of blocks) {
    if (!b.platform && !b.deviceId && !b.deviceName && !b.agentVersion) continue;
    const key = `${b.userId}\u0000${b.deviceId ?? b.deviceName ?? b.platform ?? ""}`;
    const e = by.get(key) ?? {
      userId: b.userId,
      userName: null,
      deviceId: b.deviceId,
      deviceName: b.deviceName,
      platform: b.platform,
      osVersion: b.osVersion,
      agentVersion: b.agentVersion,
      lastActivityAt: null,
      minutes: 0,
    };
    e.minutes += b.minutes;
    if (e.lastActivityAt === null || b.end > e.lastActivityAt) {
      e.lastActivityAt = b.end;
      e.osVersion = b.osVersion ?? e.osVersion;
      e.agentVersion = b.agentVersion ?? e.agentVersion;
      e.platform = b.platform ?? e.platform;
    }
    by.set(key, e);
  }
  return [...by.values()];
}

export interface PlatformShare {
  /** null: the agent didn't say (an older agent). */
  platform: AgentPlatform | null;
  label: string;
  devices: number;
  members: number;
  minutes: number;
  /** Whole-percent share of the tracked time; the shares add up to 100. */
  share: number;
}

/** Tracked time and devices per platform: Windows, macOS, then devices that didn't report one. */
export function platformMix(rows: readonly DeviceRow[]): PlatformShare[] {
  const order: (AgentPlatform | null)[] = ["windows", "macos", null];
  const groups = order.map((platform) => {
    const mine = rows.filter((r) => r.platform === platform);
    return {
      platform,
      label: platformLabel(platform) ?? "Not reported",
      devices: mine.length,
      members: new Set(mine.map((r) => r.userId)).size,
      minutes: mine.reduce((s, r) => s + r.minutes, 0),
    };
  });
  const present = groups.filter((g) => g.devices > 0 && g.minutes > 0);
  const shares = apportionPercent(present.map((g) => g.minutes));
  return present.map((g, i) => ({ ...g, share: shares[i] }));
}

function compareVersions(a: string, b: string): number {
  const pa = a.split(/[.+-]/).map((x) => Number.parseInt(x, 10));
  const pb = b.split(/[.+-]/).map((x) => Number.parseInt(x, 10));
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const x = Number.isFinite(pa[i]) ? pa[i] : 0;
    const y = Number.isFinite(pb[i]) ? pb[i] : 0;
    if (x !== y) return x - y;
  }
  return a.localeCompare(b);
}

/** Devices per agent version, newest version first; devices that reported none last. */
export function agentVersions(rows: readonly DeviceRow[]): { version: string | null; devices: number }[] {
  const by = new Map<string | null, number>();
  for (const r of rows) by.set(r.agentVersion, (by.get(r.agentVersion) ?? 0) + 1);
  return [...by.entries()]
    .map(([version, devices]) => ({ version, devices }))
    .sort((a, b) => (a.version === null ? 1 : 0) - (b.version === null ? 1 : 0) || (a.version && b.version ? compareVersions(b.version, a.version) : 0));
}

/* ----------------------------- Layouts ----------------------------- */

export interface Tile<T> {
  item: T;
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * Squarified treemap: tiles with areas proportional to `value`, laid out in
 * strips along the shorter side so each stays close to square. Largest first;
 * non-positive values get no tile.
 */
export function squarify<T>(items: readonly T[], value: (item: T) => number, width: number, height: number): Tile<T>[] {
  const positive = items.map((item) => ({ item, v: Math.max(0, value(item)) })).filter((n) => n.v > 0);
  const total = positive.reduce((s, n) => s + n.v, 0);
  if (total <= 0 || width <= 0 || height <= 0) return [];
  const scale = (width * height) / total;
  const nodes = positive.sort((a, b) => b.v - a.v).map((n) => ({ item: n.item, area: n.v * scale }));
  const out: Tile<T>[] = [];
  let x = 0;
  let y = 0;
  let w = width;
  let h = height;
  const worst = (row: { area: number }[], side: number) => {
    const s = row.reduce((t, n) => t + n.area, 0);
    const max = Math.max(...row.map((n) => n.area));
    const min = Math.min(...row.map((n) => n.area));
    return Math.max((side * side * max) / (s * s), (s * s) / (side * side * min));
  };
  const place = (row: { item: T; area: number }[]) => {
    const s = row.reduce((t, n) => t + n.area, 0);
    if (w >= h) {
      const stripW = s / h;
      let cy = y;
      for (const n of row) {
        const nh = n.area / stripW;
        out.push({ item: n.item, x, y: cy, w: stripW, h: nh });
        cy += nh;
      }
      x += stripW;
      w -= stripW;
    } else {
      const stripH = s / w;
      let cx = x;
      for (const n of row) {
        const nw = n.area / stripH;
        out.push({ item: n.item, x: cx, y, w: nw, h: stripH });
        cx += nw;
      }
      y += stripH;
      h -= stripH;
    }
  };
  let row: { item: T; area: number }[] = [];
  for (const node of nodes) {
    const side = Math.min(w, h);
    if (row.length === 0 || worst([...row, node], side) <= worst(row, side)) {
      row.push(node);
    } else {
      place(row);
      row = [node];
    }
  }
  if (row.length) place(row);
  return out;
}

export interface PlacedNode {
  index: number;
  node: FlowNode;
  /** Hours through the node. */
  value: number;
  x: number;
  y: number;
  height: number;
}

export interface PlacedLink {
  source: number;
  target: number;
  value: number;
  /** Centre line of the band where it leaves the member and enters the project. */
  sy: number;
  ty: number;
  thickness: number;
  x0: number;
  x1: number;
}

/**
 * Two-column flow layout (members left, projects right) for
 * {@link memberProjectFlow}: node heights proportional to their hours, bands
 * stacked inside each node in the order of the node they connect to, so the
 * ribbons cross as little as possible.
 */
export function layoutFlow(
  flow: FlowData,
  width: number,
  height: number,
  options: { nodeWidth?: number; gap?: number; left?: number; right?: number } = {},
): { nodes: PlacedNode[]; links: PlacedLink[] } {
  const nodeWidth = options.nodeWidth ?? 12;
  const gap = options.gap ?? 14;
  const leftX = options.left ?? 0;
  const rightX = width - (options.right ?? 0) - nodeWidth;
  const value = flow.nodes.map(() => 0);
  for (const l of flow.links) {
    value[l.source] += l.value;
    value[l.target] += l.value;
  }
  const columns = [
    flow.nodes.map((n, i) => ({ n, i })).filter(({ n }) => n.kind === "member"),
    flow.nodes.map((n, i) => ({ n, i })).filter(({ n }) => n.kind === "project"),
  ];
  const total = flow.links.reduce((s, l) => s + l.value, 0);
  const longest = Math.max(1, ...columns.map((c) => c.length));
  const k = total > 0 ? Math.max(0, height - gap * (longest - 1)) / total : 0;

  const placed: PlacedNode[] = new Array(flow.nodes.length);
  columns.forEach((column, side) => {
    const used = column.reduce((s, { i }) => s + Math.max(2, value[i] * k), 0) + gap * Math.max(0, column.length - 1);
    let y = Math.max(0, (height - used) / 2);
    for (const { n, i } of column) {
      const h = Math.max(2, value[i] * k);
      placed[i] = { index: i, node: n, value: value[i], x: side === 0 ? leftX : rightX, y, height: h };
      y += h + gap;
    }
  });

  const sourceOffset = flow.nodes.map(() => 0);
  const targetOffset = flow.nodes.map(() => 0);
  const centre = (i: number) => placed[i].y + placed[i].height / 2;
  const bySource = [...flow.links].sort((a, b) => a.source - b.source || centre(a.target) - centre(b.target));
  const sy = new Map<(typeof flow.links)[number], number>();
  for (const l of bySource) {
    const t = l.value * k;
    sy.set(l, placed[l.source].y + sourceOffset[l.source] + t / 2);
    sourceOffset[l.source] += t;
  }
  const byTarget = [...flow.links].sort((a, b) => a.target - b.target || centre(a.source) - centre(b.source));
  const ty = new Map<(typeof flow.links)[number], number>();
  for (const l of byTarget) {
    const t = l.value * k;
    ty.set(l, placed[l.target].y + targetOffset[l.target] + t / 2);
    targetOffset[l.target] += t;
  }
  const links = flow.links.map((l) => ({
    source: l.source,
    target: l.target,
    value: l.value,
    sy: sy.get(l) ?? 0,
    ty: ty.get(l) ?? 0,
    thickness: Math.max(1, l.value * k),
    x0: leftX + nodeWidth,
    x1: rightX,
  }));
  return { nodes: placed, links };
}

/** A point with a label to place next to it (pixels; the label's box is `width` × `height`). */
export interface LabelPoint {
  id: string;
  x: number;
  y: number;
  /** The mark's radius. */
  r: number;
  width: number;
  height: number;
}

/** Where a label goes: the centre of its box. */
export interface LabelPlacement {
  x: number;
  y: number;
}

/** The rectangle labels must stay inside (pixels). */
export interface LabelArea {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

/**
 * Labels that never overlap: each point, in the order given (most important
 * first), takes the first spot around its mark — above, below, right, left,
 * then the diagonals — whose box stays inside `area` and clear of every label
 * placed so far and of every other mark. A label with no free spot is left
 * out (its point stays reachable through the tooltip and the table).
 */
export function placeLabels(points: readonly LabelPoint[], area: LabelArea, gap = 3): Map<string, LabelPlacement> {
  const placed = new Map<string, LabelPlacement>();
  const boxes: { x0: number; y0: number; x1: number; y1: number }[] = [];
  const hitsMark = (box: { x0: number; y0: number; x1: number; y1: number }, self: LabelPoint) =>
    points.some((p) => {
      if (p === self) return false;
      const nx = Math.max(box.x0, Math.min(p.x, box.x1));
      const ny = Math.max(box.y0, Math.min(p.y, box.y1));
      return (p.x - nx) ** 2 + (p.y - ny) ** 2 < p.r ** 2;
    });
  for (const p of points) {
    const w = p.width;
    const h = p.height;
    const d = p.r + gap;
    const diagonal = d * Math.SQRT1_2;
    const spots: [number, number][] = [
      [p.x, p.y - d - h / 2],
      [p.x, p.y + d + h / 2],
      [p.x + d + w / 2, p.y],
      [p.x - d - w / 2, p.y],
      [p.x + diagonal + w / 2, p.y - diagonal - h / 2],
      [p.x - diagonal - w / 2, p.y - diagonal - h / 2],
      [p.x + diagonal + w / 2, p.y + diagonal + h / 2],
      [p.x - diagonal - w / 2, p.y + diagonal + h / 2],
    ];
    for (const [cx, cy] of spots) {
      const box = { x0: cx - w / 2, y0: cy - h / 2, x1: cx + w / 2, y1: cy + h / 2 };
      if (box.x0 < area.x0 || box.y0 < area.y0 || box.x1 > area.x1 || box.y1 > area.y1) continue;
      if (boxes.some((b) => box.x0 < b.x1 && b.x0 < box.x1 && box.y0 < b.y1 && b.y0 < box.y1)) continue;
      if (hitsMark(box, p)) continue;
      boxes.push(box);
      placed.set(p.id, { x: cx, y: cy });
      break;
    }
  }
  return placed;
}

/* ----------------------------- Summary ----------------------------- */

export interface AnalyticsSummary {
  totalMinutes: number;
  /** Activity of the measured minutes; null when nothing was measured. */
  activity: number | null;
  /** Minutes without input; null when the source doesn't measure idle time. */
  idleMinutes: number | null;
  sessions: number;
  members: number;
  daysWorked: number;
  /** Mean / longest continuous stretch (minutes) of the loaded blocks; null without any. */
  avgStretch: number | null;
  longestStretch: number | null;
  /** Key presses + clicks per tracked hour; null without time. */
  inputPerHour: number | null;
  peakHour: number | null;
  /** Monday-first index of the weekday with the most time; null without time. */
  busiestWeekday: number | null;
}

/** The period rolled up: totals from the aggregates, stretches from the loaded blocks, rhythm from the hour cells. */
export function summarize(agg: ReportAggregates, days: readonly string[], blocks: readonly Block[], cells: readonly HourCell[]): AnalyticsSummary {
  const inPeriod = new Set(days);
  const stretches = focusStretches(blocks);
  const { totals } = agg;
  return {
    totalMinutes: totals.minutes,
    activity: totals.activity === null ? null : Math.round(totals.activity),
    idleMinutes: totals.idleMinutes,
    sessions: totals.sessions,
    members: agg.members.filter((m) => m.minutes > 0).length,
    daysWorked: agg.daily.filter((d) => inPeriod.has(d.day) && d.minutes > 0).length,
    avgStretch: stretches.length ? stretches.reduce((s, x) => s + x.minutes, 0) / stretches.length : null,
    longestStretch: stretches.length ? stretches.reduce((m, x) => Math.max(m, x.minutes), 0) : null,
    inputPerHour: totals.minutes > 0 ? Math.round(((totals.keys + totals.clicks) / totals.minutes) * 60) : null,
    peakHour: peakHour(hourTotals(cells)),
    busiestWeekday: busiestWeekday(cells),
  };
}
