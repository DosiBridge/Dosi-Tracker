import { describe, expect, it } from "vitest";
import {
  NOW,
  activities as primaryActivities,
  projects as primaryProjects,
  summary as primarySummary,
  users as primaryUsers,
} from "./mock-data";
import { datasetFor } from "./tenant-data";
import { minutesForProject, minutesForUser, totalMinutes } from "./metrics";
import { trackedMembers } from "./roles";
import type { Activity, Project, User } from "./types";

/* ============================================================================
 * CROSS-SURFACE CONSISTENCY
 *
 * The dashboard and /team render `user.trackedToday` and
 * `summary.totalTrackedToday`. Every report (weekly, time & activity, payroll,
 * projects) aggregates the ACTIVITY rows instead. Those two derivations MUST
 * describe the same reality: an owner reading "6h 12m" on the dashboard and
 * then paying against the payroll report cannot be shown two different numbers
 * for the same person and the same period.
 *
 * This spec re-derives tracked minutes straight from the activity rows with a
 * deliberately INDEPENDENT oracle (the two-line definition below) rather than
 * calling the production helper, so it pins the *concept*, not one particular
 * implementation of it, and stays a real check on `src/lib/metrics.ts`.
 * ========================================================================== */

/** The definition: an activity accounts for the wall-clock time it spans. */
function durationOf(a: Activity): number {
  return (Date.parse(a.endedAt) - Date.parse(a.startedAt)) / 60_000;
}
function sumMinutes(list: Activity[]): number {
  return list.reduce((s, a) => s + durationOf(a), 0);
}

/** "Day 0" = the demo's today, in UTC — the same bucket the reports group by
 *  (`endedAt.slice(0, 10)`), so it does not drift with the runner's timezone. */
const TODAY = NOW.toISOString().slice(0, 10);
function day0Of(list: Activity[], userId?: string): Activity[] {
  return list.filter(
    (a) => a.endedAt.slice(0, 10) === TODAY && (userId === undefined || a.userId === userId),
  );
}

const WEEK_MS = 7 * 24 * 60 * 60 * 1000;

function checkTenant(
  label: string,
  users: User[],
  projects: Project[],
  activities: Activity[],
  totalTrackedToday: number,
) {
  describe(label, () => {
    const members = trackedMembers(users);

    it.each(members.map((u) => [u.name, u] as const))(
      "%s: minutes summed from today's activity rows equal their trackedToday",
      (_name, user) => {
        // Exact equality — the generator lands the final session of the day
        // precisely on the target, so no tolerance is warranted or wanted.
        expect(sumMinutes(day0Of(activities, user.id))).toBe(user.trackedToday);
      },
    );

    it("summary.totalTrackedToday equals the sum of every member's day-0 activity minutes", () => {
      const fromActivities = members.reduce((s, u) => s + sumMinutes(day0Of(activities, u.id)), 0);
      expect(totalTrackedToday).toBe(fromActivities);
      // …and nothing outside the tracked roster contributes activity rows.
      expect(sumMinutes(day0Of(activities))).toBe(fromActivities);
    });

    it("every project's loggedThisWeek equals the minutes its rows account for", () => {
      const from = new Date(NOW.getTime() - WEEK_MS);
      for (const project of projects) {
        const fromActivities = sumMinutes(
          activities.filter((a) => a.projectId === project.id && Date.parse(a.endedAt) >= +from),
        );
        expect({ id: project.id, week: project.loggedThisWeek }).toEqual({
          id: project.id,
          week: fromActivities,
        });
        // A wider period can never hold less time than the one inside it.
        expect(project.loggedThisMonth).toBeGreaterThanOrEqual(project.loggedThisWeek);
        expect(project.loggedTotal).toBeGreaterThanOrEqual(project.loggedThisMonth);
      }
    });

    it("every activity row spans real, positive, already-elapsed time", () => {
      expect(activities.length).toBeGreaterThan(0);
      for (const a of activities) {
        expect(durationOf(a)).toBeGreaterThan(0);
        expect(Date.parse(a.endedAt)).toBeLessThanOrEqual(NOW.getTime());
        expect(Date.parse(a.startedAt)).toBeLessThan(Date.parse(a.endedAt));
      }
    });

    it("metrics.ts reproduces this spec's independent derivation exactly", () => {
      // The surfaces call metrics.ts; this spec derives the same numbers on its
      // own. If the two ever part company, one of them is lying to the user.
      expect(totalMinutes(activities)).toBe(sumMinutes(activities));
      for (const u of members) {
        expect(minutesForUser(activities, u.id)).toBe(
          sumMinutes(activities.filter((a) => a.userId === u.id)),
        );
      }
      for (const p of projects) {
        expect(minutesForProject(activities, p.id)).toBe(
          sumMinutes(activities.filter((a) => a.projectId === p.id)),
        );
      }
    });
  });
}

describe("tracked-time agreement across surfaces", () => {
  checkTenant(
    "w1 · Dosi Labs",
    primaryUsers,
    primaryProjects,
    primaryActivities,
    primarySummary.totalTrackedToday,
  );

  for (const id of ["w2", "w3"]) {
    const ds = datasetFor(id);
    checkTenant(
      `${id} · ${ds.workspaceId}`,
      ds.users,
      ds.projects,
      ds.activities,
      ds.summary.totalTrackedToday,
    );
  }

  it("keeps the row count renderable — /activities draws a card per row", () => {
    // Enough rows to account for the tracked time, few enough that the
    // activities page stays a list rather than a stress test.
    expect(primaryActivities.length).toBeGreaterThanOrEqual(150);
    expect(primaryActivities.length).toBeLessThanOrEqual(350);
  });
});
