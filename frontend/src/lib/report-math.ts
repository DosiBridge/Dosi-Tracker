import type { RangeKey } from "./reports-data";

/* ============================================================================
 * REPORT MATH — pure helpers shared by every report screen.
 *
 * The reports used to carry their own copy of the real-clock range presets,
 * their own query-string building and their own percentage/money arithmetic
 * (with a divide-by-zero "NaN%" and a payroll that multiplied a member's
 * HIGHEST project rate by ALL of their hours). One definition lives here, as
 * pure functions of their inputs (the clock is a parameter), so it is
 * unit-tested without React or a backend.
 * ========================================================================== */

const PRESET_LABELS: Record<RangeKey, string> = {
  today: "Today",
  yesterday: "Yesterday",
  "7d": "Last 7 days",
  "30d": "Last 30 days",
  month: "This month",
  custom: "Custom",
};

/**
 * Bounds of a range preset relative to `now` (local calendar, like the demo
 * presets in reports-data). Demo pages pass the frozen demo clock, live pages
 * the real one — the arithmetic is identical. "custom" has no explicit dates
 * yet, so it reads as the last 7 days.
 */
export function rangeForKeyAt(key: RangeKey, now: Date): { from: Date; to: Date; label: string } {
  const to = new Date(now);
  const from = new Date(now);
  switch (key) {
    case "today":
      from.setHours(0, 0, 0, 0);
      break;
    case "yesterday":
      from.setDate(from.getDate() - 1);
      from.setHours(0, 0, 0, 0);
      to.setDate(to.getDate() - 1);
      to.setHours(23, 59, 59, 999);
      break;
    case "30d":
      from.setDate(from.getDate() - 30);
      break;
    case "month":
      from.setDate(1);
      from.setHours(0, 0, 0, 0);
      break;
    default: // "7d" and "custom"
      from.setDate(from.getDate() - 7);
      break;
  }
  return { from, to, label: PRESET_LABELS[key] };
}

/** The "no filter" value of the report dropdowns. */
export const ALL = "all";

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

/** A non-negative finite number of minutes (anything else counts as 0). */
function minutesOf(value: unknown): number {
  const n = Number(value);
  return Number.isFinite(n) && n > 0 ? n : 0;
}

/** Round to cents (also used for hours and rates shown with 2 decimals). */
function toCents(n: number): number {
  return Math.round(n * 100) / 100;
}

/** Roster name when known, otherwise a stable short label (never a demo person). */
export function memberName(userId: string, known?: { name?: string | null } | null): string {
  const name = known?.name?.trim();
  return name || `Member ${String(userId).slice(0, 8)}`;
}

/* ----------------------------- Rates & pay ----------------------------- */

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

/** Labour cost of one project: each member's hours on it × their rate on it. */
export function laborCost(
  perUser: readonly MinutesRow[],
  rates: ReadonlyMap<string, number>,
): { minutes: number; pricedMinutes: number; cost: number } {
  let minutes = 0;
  let pricedMinutes = 0;
  let cost = 0;
  for (const row of perUser) {
    const m = minutesOf(row.trackedMinutes);
    const rate = rates.get(row.userId) ?? 0;
    minutes += m;
    if (rate > 0) {
      pricedMinutes += m;
      cost += (m / 60) * rate;
    }
  }
  return { minutes, pricedMinutes, cost: toCents(cost) };
}

export interface ProjectPayrollInput {
  projectId: string;
  /** Each member's minutes on THIS project. */
  perUser: readonly MinutesRow[];
  /** Each member's hourly rate on THIS project (see ratesByMember). */
  rates: ReadonlyMap<string, number>;
}

export interface PayrollLine {
  userId: string;
  /** All tracked minutes in the range. */
  minutes: number;
  hours: number;
  /** Hours on projects where the member has a rate. */
  pricedHours: number;
  /** Σ over projects of (rate on the project × hours on the project), in cents. */
  amount: number;
  /** Effective blended rate: amount ÷ hours (0 without hours). */
  rate: number;
  /** Projects the member tracked time on. */
  projectCount: number;
}

/**
 * Pay per member = SUM over projects of (that project's hourly rate for the
 * member × the member's hours on that project). Hours on a project without a
 * rate stay in the hours but add nothing to the pay, so the blended rate is
 * honest about unpriced time. `totals` (each member's minutes across all
 * projects) is authoritative for hours; members only seen per project are
 * still listed.
 */
export function computePayroll(totals: readonly MinutesRow[], projects: readonly ProjectPayrollInput[]): PayrollLine[] {
  const acc = new Map<string, { total: number; onProjects: number; priced: number; amount: number; projects: number }>();
  const entry = (userId: string) => {
    let e = acc.get(userId);
    if (!e) {
      e = { total: 0, onProjects: 0, priced: 0, amount: 0, projects: 0 };
      acc.set(userId, e);
    }
    return e;
  };

  for (const t of totals) entry(t.userId).total += minutesOf(t.trackedMinutes);
  for (const p of projects) {
    for (const row of p.perUser) {
      const m = minutesOf(row.trackedMinutes);
      if (m === 0) continue;
      const e = entry(row.userId);
      const rate = p.rates.get(row.userId) ?? 0;
      e.onProjects += m;
      e.projects += 1;
      if (rate > 0) {
        e.priced += m;
        e.amount += (m / 60) * rate;
      }
    }
  }

  return [...acc.entries()].map(([userId, e]) => {
    const minutes = Math.max(e.total, e.onProjects);
    const hours = minutes / 60;
    return {
      userId,
      minutes,
      hours: toCents(hours),
      pricedHours: toCents(e.priced / 60),
      amount: toCents(e.amount),
      rate: hours > 0 ? toCents(e.amount / hours) : 0,
      projectCount: e.projects,
    };
  });
}

/* ----------------------------- Activity ----------------------------- */

/**
 * Split tracked minutes by the backend's duration-weighted activity score:
 * the active ("productive") share and everything else. The backend does not
 * classify time as neutral/unproductive or measure idle time, so nothing
 * beyond these two numbers is derived.
 */
export function activitySplit(trackedMinutes: number, averageProductivity: number): { productive: number; other: number; focus: number } {
  const tracked = Math.round(minutesOf(trackedMinutes));
  const score = Number(averageProductivity);
  const pct = Number.isFinite(score) ? Math.min(100, Math.max(0, score)) : 0;
  const productive = Math.round((tracked * pct) / 100);
  const other = Math.max(tracked - productive, 0);
  return { productive, other, focus: safePercent(productive, productive + other) };
}

/** One member-day row (GET /api/app/reporting/user-daily-series). */
export interface UserDayRow {
  userId: string;
  date: string;
}

/** userId → number of distinct UTC days with tracked activity. */
export function activeDaysByUser(rows: readonly UserDayRow[]): Map<string, number> {
  const days = new Map<string, Set<string>>();
  for (const r of rows) {
    if (!r || typeof r.userId !== "string" || typeof r.date !== "string") continue;
    const set = days.get(r.userId) ?? new Set<string>();
    set.add(r.date.slice(0, 10));
    days.set(r.userId, set);
  }
  return new Map([...days].map(([id, set]) => [id, set.size]));
}
