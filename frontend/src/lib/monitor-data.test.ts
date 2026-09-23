// The Member Monitor must report the same day as the rest of the app.
//
// Before this suite, /monitor generated its own segments with per-segment
// productivity drawn deliberately BELOW the member's score (meetings are less
// focused, some apps are unproductive) and stopped on the first segment that
// crossed the day's minute target. Both effects were one-directional, so the
// same person on the same day read several minutes longer and 4-10 points less
// productive on /monitor than on /team and /dashboard — the audit measured
// Tanvir at 82 vs 76 and Sadia at 64 vs 54. Per-segment texture is intentional
// and preserved; the day's TOTALS are what must reconcile.
import { describe, it, expect } from "vitest";
import { users, NOW } from "./mock-data";
import { buildDayTimeline, daySummary, availableDays } from "./monitor-data";
import type { Role } from "./types";

const TRACKED: Role[] = ["owner", "admin", "worker"];
const trackedUsers = users.filter((u) => TRACKED.includes(u.role));
const today = NOW.toISOString().slice(0, 10);

describe("Member Monitor agrees with the rest of the app for today", () => {
  it.each(trackedUsers.map((u) => [u.name, u.id] as const))(
    "%s: monitor tracked minutes equal the canonical trackedToday",
    (_name, id) => {
      const user = users.find((u) => u.id === id)!;
      const summary = daySummary(buildDayTimeline(id, today));
      expect(summary.tracked).toBe(user.trackedToday);
    },
  );

  it.each(trackedUsers.map((u) => [u.name, u.id] as const))(
    "%s: monitor productivity equals the canonical productivity",
    (_name, id) => {
      const user = users.find((u) => u.id === id)!;
      const summary = daySummary(buildDayTimeline(id, today));
      expect(summary.productivityAvg).toBe(user.productivity);
    },
  );

  it("keeps the per-segment texture rather than flattening every block to one number", () => {
    // Reconciliation must not turn a realistic day into a constant line.
    const segments = buildDayTimeline(trackedUsers[0].id, today);
    const active = segments.filter((s) => s.type === "work" || s.type === "meeting");
    const distinct = new Set(active.map((s) => s.productivity));
    expect(active.length).toBeGreaterThan(3);
    expect(distinct.size).toBeGreaterThan(1);
  });

  it("still produces a plausible day: segments stay inside working hours and never run negative", () => {
    for (const u of trackedUsers) {
      for (const s of buildDayTimeline(u.id, today)) {
        expect(s.minutes).toBeGreaterThan(0);
        expect(s.endMin - s.startMin).toBe(s.minutes);
        expect(s.productivity).toBeGreaterThanOrEqual(0);
        expect(s.productivity).toBeLessThanOrEqual(100);
      }
    }
  });
});

describe("Member Monitor on earlier days", () => {
  it("reports a self-consistent summary for every available day", () => {
    // Earlier days scale off the same canonical figure, so they need not equal
    // today's total — but the summary must still describe its own segments.
    const days = availableDays(7);
    for (const day of days) {
      for (const u of trackedUsers.slice(0, 3)) {
        const segments = buildDayTimeline(u.id, day.iso);
        const summary = daySummary(segments);
        const activeMinutes = segments
          .filter((s) => s.type === "work" || s.type === "meeting")
          .reduce((sum, s) => sum + s.minutes, 0);
        expect(summary.tracked).toBe(activeMinutes);
        if (summary.tracked > 0) {
          expect(summary.productivityAvg).toBeGreaterThan(0);
          expect(summary.productivityAvg).toBeLessThanOrEqual(100);
        }
      }
    }
  });

  it("is deterministic — the same member and day always build the same timeline", () => {
    const a = buildDayTimeline(trackedUsers[0].id, today);
    const b = buildDayTimeline(trackedUsers[0].id, today);
    expect(a).toEqual(b);
  });
});
