import type { Activity, User } from "./types";
import { addZonedDays, formatDayRange, zonedDayKey, zonedDayRange, zonedPeriod } from "./time-zone";

/* ============================================================================
 * Timesheet grid — one row PER MEMBER, the seven days (Mon–Sun) of a week of
 * the reporting zone per row.
 *
 * Live, the minutes come from GET /api/app/reporting/user-daily-series asked
 * for that zone (contract §6: each row's `date` is the local day); the demo
 * derives the same grid from its activity rows, each block on the day it
 * started. Pure, so the bucketing (dates the backend sends without a zone
 * suffix, rows outside the week, members missing from the roster) is
 * unit-tested.
 * ========================================================================== */

/** One member-day from the backend. */
export interface UserDailyPoint {
  userId: string;
  userName?: string | null;
  date: string;
  trackedMinutes?: number | null;
  activityCount?: number | null;
}

export type GridMember = Pick<User, "id" | "name" | "designation" | "status">;

export interface WeekMatrix {
  members: GridMember[];
  /** Minutes per member (row) per day (column). */
  matrix: number[][];
}

/** A Monday–Sunday week of the reporting zone. */
export interface TimesheetWeek {
  /** The seven days, "YYYY-MM-DD", Monday first. */
  days: string[];
  /** Query bounds: Monday's start … the start of the next Monday. */
  from: Date;
  to: Date;
  /** Today in the zone: days after it haven't happened yet. */
  today: string;
  /** "29 Sep – 5 Oct 2026" */
  span: string;
}

/** The week `offset` weeks back from the one containing `now` (0 = this week; later weeks don't exist yet). */
export function timesheetWeek(now: Date, timeZone: string, offset = 0): TimesheetWeek {
  const monday = zonedPeriod("week", now, timeZone, Math.min(0, offset)).fromKey;
  const sunday = addZonedDays(monday, 6);
  const range = zonedDayRange(monday, sunday, timeZone);
  return { days: range.days, from: range.from, to: range.to, today: zonedDayKey(now, timeZone), span: formatDayRange(monday, sunday) };
}

function emptyMatrix(roster: GridMember[], days: string[]): { members: GridMember[]; matrix: number[][]; index: Map<string, number> } {
  const members: GridMember[] = [...roster];
  return {
    members,
    matrix: members.map(() => Array<number>(days.length).fill(0)),
    index: new Map<string, number>(members.map((m, i) => [m.id, i])),
  };
}

/**
 * Build the week matrix from the backend's member-days. `roster` fixes the
 * row order (and gives members with no time a row of zeros); anyone present
 * only in the series is appended, named by the backend's userName. Rows on
 * other days are ignored; a member-day reported twice is summed.
 */
export function buildWeekMatrix(points: UserDailyPoint[], days: string[], roster: GridMember[]): WeekMatrix {
  const { members, matrix, index } = emptyMatrix(roster, days);
  const column = new Map(days.map((d, i) => [d, i]));

  for (const p of Array.isArray(points) ? points : []) {
    if (!p || typeof p.userId !== "string") continue;
    // The calendar date only: the backend writes the local day without a zone ("2026-07-14T00:00:00").
    const di = column.get(String(p.date).slice(0, 10));
    if (di === undefined) continue;

    let row = index.get(p.userId);
    if (row === undefined) {
      row = members.length;
      index.set(p.userId, row);
      members.push({ id: p.userId, name: (p.userName ?? "").trim() || "Member", designation: "Member", status: "offline" });
      matrix.push(Array<number>(days.length).fill(0));
    }
    const minutes = Number(p.trackedMinutes ?? 0);
    matrix[row][di] += Number.isFinite(minutes) ? Math.max(0, Math.round(minutes)) : 0;
  }
  return { members, matrix };
}

/**
 * The same grid from activity rows (the demo): each block counts its whole
 * length on the day it started in `timeZone`, for roster members only.
 */
export function weekMatrixFromActivities(
  activities: Activity[],
  days: string[],
  roster: GridMember[],
  timeZone: string,
): WeekMatrix {
  const { members, index } = emptyMatrix(roster, days);
  const column = new Map(days.map((d, i) => [d, i]));
  const exact = members.map(() => Array<number>(days.length).fill(0));
  for (const a of activities) {
    const row = index.get(a.userId);
    const start = Date.parse(a.startedAt);
    const end = Date.parse(a.endedAt);
    if (row === undefined || !Number.isFinite(start) || !Number.isFinite(end) || end <= start) continue;
    const di = column.get(zonedDayKey(start, timeZone));
    if (di === undefined) continue;
    exact[row][di] += (end - start) / 60_000;
  }
  return { members, matrix: exact.map((cells) => cells.map((m) => Math.round(m))) };
}

/** A cell's heat step: 0 nothing, then under 2 h, under 4 h, under 6 h, and 6 h or more. */
export function heatStep(minutes: number): 0 | 1 | 2 | 3 | 4 {
  if (!(minutes > 0)) return 0;
  if (minutes < 120) return 1;
  if (minutes < 240) return 2;
  if (minutes < 360) return 3;
  return 4;
}

/** The average over the days that have tracked time (a week of 3 worked days averages over 3, not 7). */
export function averagePerWorkedDay(dayTotals: number[]): { minutes: number; days: number } {
  const worked = dayTotals.filter((m) => m > 0);
  const total = worked.reduce((s, m) => s + m, 0);
  return { minutes: worked.length ? Math.round(total / worked.length) : 0, days: worked.length };
}
