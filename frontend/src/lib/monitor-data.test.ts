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
import { afterEach, describe, it, expect } from "vitest";
import { users, NOW } from "./mock-data";
import {
  appBreakdown,
  availableDays,
  buildDayTimeline,
  daySummary,
  fmtMin,
  hourlyBuckets,
  type DaySegment,
} from "./monitor-data";
import { installLiveDataset, projects, resetTenantDataForTests } from "./tenant-data";
import type { Project, Role } from "./types";
import { colorFromString } from "./utils";

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

  it("anchors the day list on the clock it is given (a live session passes the real one)", () => {
    const days = availableDays(3, new Date("2026-09-23T10:00:00.000Z"));
    expect(days.map((d) => d.iso)).toEqual(["2026-09-23", "2026-09-22", "2026-09-21"]);
    expect(days.map((d) => d.isToday)).toEqual([true, false, false]);
    expect(availableDays(1)[0].iso).toBe(today);
  });
});

// Regression: "Cannot read properties of undefined (reading 'id')". With real
// (or partial) tenant data the generator's project fallback was `[projects[0]]`
// — `[undefined]` in a workspace with no projects — and `p.memberIds.includes`
// threw for a project without a member list. Every lookup is now guarded.
describe("buildDayTimeline guards (never throws on partial data)", () => {
  afterEach(() => resetTenantDataForTests());

  const member = trackedUsers.find((u) => u.trackedToday > 0)!;

  it("builds a project-less day instead of crashing when the workspace has no projects", () => {
    projects.length = 0;
    let segments: DaySegment[] = [];
    expect(() => {
      segments = buildDayTimeline(member.id, today);
    }).not.toThrow();
    expect(segments.length).toBeGreaterThan(0);
    expect(segments.every((s) => s.projectId === null)).toBe(true);
    expect(daySummary(segments).tracked).toBe(member.trackedToday);
  });

  it("tolerates projects without a member list", () => {
    for (const p of projects) (p as Partial<Project>).memberIds = undefined;
    expect(() => buildDayTimeline(member.id, today)).not.toThrow();
    expect(daySummary(buildDayTimeline(member.id, today)).tracked).toBe(member.trackedToday);
  });

  it("returns an empty day for an unknown member or a malformed day key", () => {
    expect(buildDayTimeline("nobody", today)).toEqual([]);
    expect(buildDayTimeline(member.id, "")).toEqual([]);
    expect(buildDayTimeline(member.id, undefined as unknown as string)).toEqual([]);
  });

  it("never generates a day for a real (live) tenant, even for a member with tracked time", () => {
    installLiveDataset({
      workspaceId: "live-monitor-test",
      users: [{ ...member }],
      projects: [],
      activities: [],
      now: NOW,
    });
    expect(buildDayTimeline(member.id, today)).toEqual([]);
  });
});

describe("day roll-ups over an arbitrary axis", () => {
  const block = (startMin: number, endMin: number, app = "Code"): DaySegment => ({
    id: `b${startMin}`,
    type: "work",
    startMin,
    endMin,
    minutes: endMin - startMin,
    app,
    windowTitle: "",
    projectId: null,
    productivity: 50,
    mouseClicks: 0,
    keyboardHits: 0,
    screen: { app, kind: "docs", accent: "#000" },
  });

  it("buckets the working day by default and any wider window on request", () => {
    const early = [block(150, 210)];
    expect(hourlyBuckets(early)).toHaveLength(12);
    expect(hourlyBuckets(early).every((b) => b.minutes === 0)).toBe(true);
    const wide = hourlyBuckets(early, 120, 1200);
    expect(wide[0]).toEqual({ hour: "02:00", minutes: 30 });
    expect(wide[1]).toEqual({ hour: "03:00", minutes: 30 });
    expect(wide).toHaveLength(18);
  });

  it("ends a focus run at a gap between real blocks, but not at a sub-2-minute seam", () => {
    expect(daySummary([block(540, 600), block(602, 660)]).longestFocus).toBe(118);
    expect(daySummary([block(540, 600), block(603, 660)]).longestFocus).toBe(60);
    expect(daySummary([block(540, 600), block(700, 790)]).longestFocus).toBe(90);
  });

  it("formats the end of the day as 24:00", () => {
    expect(fmtMin(1440)).toBe("24:00");
    expect(fmtMin(0)).toBe("00:00");
    expect(fmtMin(605)).toBe("10:05");
  });

  it("gives apps outside the demo catalog a stable color of their own", () => {
    const [row] = appBreakdown([block(600, 630, "Obscure Tool")]);
    expect(row).toEqual({ app: "Obscure Tool", minutes: 30, accent: colorFromString("Obscure Tool", 60, 50), category: "neutral" });
    expect(appBreakdown([block(600, 630, "constructor")])[0].accent).toBe(colorFromString("constructor", 60, 50));
  });
});
