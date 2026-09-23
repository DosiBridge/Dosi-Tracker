import type { Activity } from "./types";

/* ============================================================================
 * THE canonical derivation of tracked time.
 *
 * Every surface that shows "how long did someone work" — dashboard KPIs, /team,
 * /timesheet, /projects and every report (time & activity, weekly, payroll,
 * projects, apps, productivity) — MUST call these functions instead of
 * re-deriving the number locally. Historically each surface rolled its own
 * arithmetic (one counted rows × `project.intervalMinutes`, another summed a
 * hardcoded per-user field) and the same metric differed by up to 19x between
 * pages, with payroll — the number an owner actually pays against — lowest.
 * A single definition is the only thing that keeps them honest.
 *
 * These are pure functions over `Activity[]`: no React, no module state, no
 * imports from components/hooks (`npm run test:arch` enforces the layering).
 *
 * DEFINITIONS, fixed here and nowhere else:
 *  - An activity accounts for the WALL-CLOCK time it spans, `endedAt` minus
 *    `startedAt`. Not its project's screenshot interval — that is a capture
 *    cadence, not a duration.
 *  - An activity belongs to the period that contains its `endedAt`. Work is
 *    booked when it lands, which is also how the day buckets and the range
 *    filters in `reports-data.ts` group rows.
 *  - "Productivity" over a set of rows is DURATION-WEIGHTED: a 90-minute block
 *    at 50% must not count the same as a 5-minute block at 95%.
 *  - Calendar days are UTC, so a bucket never shifts with the viewer's or the
 *    test runner's timezone.
 * ========================================================================== */

/** Minutes a single activity accounts for. Malformed or reversed rows are 0. */
export function durationMinutes(activity: Activity): number {
  const start = Date.parse(activity.startedAt);
  const end = Date.parse(activity.endedAt);
  if (!Number.isFinite(start) || !Number.isFinite(end)) return 0;
  return Math.max(0, (end - start) / 60_000);
}

/** Total minutes across a set of activities. */
export function totalMinutes(activities: Activity[]): number {
  return activities.reduce((sum, a) => sum + durationMinutes(a), 0);
}

/** The UTC calendar day an activity is booked to ("2026-07-14"). */
export function dayKey(activity: Activity): string {
  return activity.endedAt.slice(0, 10);
}

/** Does this activity land inside [from, to] (both ends inclusive)? */
export function inRange(activity: Activity, from: Date, to: Date): boolean {
  const at = Date.parse(activity.endedAt);
  return Number.isFinite(at) && at >= from.getTime() && at <= to.getTime();
}

/** Minutes booked inside [from, to]. */
export function minutesInRange(activities: Activity[], from: Date, to: Date): number {
  return totalMinutes(activities.filter((a) => inRange(a, from, to)));
}

/** Minutes booked by one member, optionally narrowed to a period. */
export function minutesForUser(
  activities: Activity[],
  userId: string,
  from?: Date,
  to?: Date,
): number {
  return totalMinutes(
    activities.filter(
      (a) => a.userId === userId && (from === undefined || to === undefined || inRange(a, from, to)),
    ),
  );
}

/** Minutes booked to one project, optionally narrowed to a period. */
export function minutesForProject(
  activities: Activity[],
  projectId: string,
  from?: Date,
  to?: Date,
): number {
  return totalMinutes(
    activities.filter(
      (a) =>
        a.projectId === projectId && (from === undefined || to === undefined || inRange(a, from, to)),
    ),
  );
}

/** The rows booked to one UTC calendar day, optionally for a single member. */
export function activitiesOnDay(activities: Activity[], day: string, userId?: string): Activity[] {
  return activities.filter(
    (a) => dayKey(a) === day && (userId === undefined || a.userId === userId),
  );
}

/** Minutes booked on one UTC calendar day, optionally for a single member. */
export function minutesOnDay(activities: Activity[], day: string, userId?: string): number {
  return totalMinutes(activitiesOnDay(activities, day, userId));
}

/**
 * Duration-weighted mean productivity (0-100, rounded). Empty input — and input
 * whose rows all have zero duration — scores 0: there is no work to rate.
 */
export function averageProductivity(activities: Activity[]): number {
  let weighted = 0;
  let minutes = 0;
  for (const a of activities) {
    const d = durationMinutes(a);
    weighted += d * a.productivity;
    minutes += d;
  }
  return minutes > 0 ? Math.round(weighted / minutes) : 0;
}

/** How many distinct UTC days a member logged any time on. */
export function distinctActiveDays(activities: Activity[], userId: string): number {
  const days = new Set<string>();
  for (const a of activities) {
    if (a.userId === userId && durationMinutes(a) > 0) days.add(dayKey(a));
  }
  return days.size;
}

/** Minutes per member — one pass, for tables that need every row at once. */
export function minutesByUser(activities: Activity[]): Map<string, number> {
  const map = new Map<string, number>();
  for (const a of activities) {
    map.set(a.userId, (map.get(a.userId) ?? 0) + durationMinutes(a));
  }
  return map;
}

/** Minutes per project — one pass. */
export function minutesByProject(activities: Activity[]): Map<string, number> {
  const map = new Map<string, number>();
  for (const a of activities) {
    map.set(a.projectId, (map.get(a.projectId) ?? 0) + durationMinutes(a));
  }
  return map;
}

/* ============================================================================
 * Reconciling a generated day against its canonical totals
 *
 * Generated timelines (see monitor-data.ts) invent per-segment texture — a
 * meeting is less focused than a deep-work block, some apps are unproductive.
 * That texture is worth keeping, but it must not change what the DAY totals to,
 * or the same person reads differently on /monitor than on /team. These two
 * helpers pin the totals while leaving the shape intact. They live here, with
 * the other definitions, rather than beside the generator.
 * ========================================================================== */

/** The minimal shape these helpers need — a timeline segment. */
export interface ReconcilableSegment {
  minutes: number;
  endMin: number;
  productivity: number;
  /** Only segments the caller counts as tracked time participate. */
  counts: boolean;
}

/**
 * Reduce total counted minutes to exactly `target` by trimming from the end.
 * Mutates in place and removes any segment trimmed away completely.
 * A target at or above the current total leaves the segments untouched.
 */
export function trimSegmentsToTarget<T extends ReconcilableSegment>(segments: T[], target: number): void {
  let over = segments.filter((s) => s.counts).reduce((sum, s) => sum + s.minutes, 0) - target;
  for (let i = segments.length - 1; i >= 0 && over > 0; i--) {
    const s = segments[i];
    if (!s.counts) continue;
    const cut = Math.min(over, s.minutes);
    if (cut >= s.minutes) {
      segments.splice(i, 1);
    } else {
      s.minutes -= cut;
      s.endMin -= cut;
    }
    over -= cut;
  }
}

/**
 * Scale segment productivity so the duration-weighted average over counted
 * segments equals `targetAvg`, preserving relative shape. The correction is
 * distributed across the headroom each segment has toward 0 or 100, so
 * clamping cannot silently absorb part of it. Mutates in place.
 */
export function reconcileSegmentProductivity<T extends ReconcilableSegment>(
  segments: T[],
  targetAvg: number,
): void {
  const counted = segments.filter((s) => s.counts);
  const totalMin = counted.reduce((sum, s) => sum + s.minutes, 0);
  if (!totalMin) return;

  for (let pass = 0; pass < 4; pass++) {
    const current = counted.reduce((sum, s) => sum + s.minutes * s.productivity, 0) / totalMin;
    const delta = targetAvg - current;
    if (Math.abs(delta) < 0.005) return;

    const headroom = counted.map((s) => (delta > 0 ? 100 - s.productivity : s.productivity));
    const weighted = counted.reduce((sum, s, i) => sum + s.minutes * headroom[i], 0);
    if (weighted <= 0) return;

    const scale = (delta * totalMin) / weighted;
    for (let i = 0; i < counted.length; i++) {
      counted[i].productivity = Math.min(
        100,
        Math.max(0, Math.round(counted[i].productivity + headroom[i] * scale)),
      );
    }
  }
}
