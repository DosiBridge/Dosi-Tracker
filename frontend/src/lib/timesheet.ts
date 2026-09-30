import type { User } from "./types";

/* ============================================================================
 * Timesheet grid from GET /api/app/reporting/user-daily-series.
 *
 * One row PER MEMBER, seven UTC days (Mon–Sun) per row. Pure so the bucketing
 * (dates the backend may send without a zone suffix, rows outside the week,
 * members missing from the roster) is unit-tested.
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

const DAY_MS = 24 * 60 * 60 * 1000;

/**
 * Build the week matrix. `roster` fixes the row order (and gives members with
 * no time a row of zeros); anyone present only in the series is appended,
 * named by the backend's userName. Rows outside [weekStart, weekStart+7d) are
 * ignored; a member-day reported twice is summed.
 */
export function buildWeekMatrix(
  points: UserDailyPoint[],
  weekStart: Date,
  roster: GridMember[],
): { members: GridMember[]; matrix: number[][] } {
  const members: GridMember[] = [...roster];
  const index = new Map<string, number>(members.map((m, i) => [m.id, i]));
  const matrix: number[][] = members.map(() => Array<number>(7).fill(0));

  for (const p of Array.isArray(points) ? points : []) {
    if (!p || typeof p.userId !== "string") continue;
    // Anchor on the calendar date only — the backend may serialize without a zone suffix.
    const dayUtc = Date.parse(`${String(p.date).slice(0, 10)}T00:00:00Z`);
    if (!Number.isFinite(dayUtc)) continue;
    const di = Math.floor((dayUtc - weekStart.getTime()) / DAY_MS);
    if (di < 0 || di > 6) continue;

    let row = index.get(p.userId);
    if (row === undefined) {
      row = members.length;
      index.set(p.userId, row);
      members.push({ id: p.userId, name: (p.userName ?? "").trim() || "Member", designation: "Member", status: "offline" });
      matrix.push(Array<number>(7).fill(0));
    }
    const minutes = Number(p.trackedMinutes ?? 0);
    matrix[row][di] += Number.isFinite(minutes) ? Math.max(0, Math.round(minutes)) : 0;
  }
  return { members, matrix };
}
