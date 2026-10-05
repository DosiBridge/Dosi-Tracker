import { describe, expect, it } from "vitest";
import {
  averagePerWorkedDay,
  buildWeekMatrix,
  heatStep,
  timesheetWeek,
  weekMatrixFromActivities,
  type GridMember,
} from "./timesheet";
import type { Activity } from "./types";

const week = ["2026-09-21", "2026-09-22", "2026-09-23", "2026-09-24", "2026-09-25", "2026-09-26", "2026-09-27"];
const roster: GridMember[] = [
  { id: "a", name: "Ann", designation: "Owner", status: "active" },
  { id: "b", name: "Ben", designation: "Member", status: "offline" },
];

describe("buildWeekMatrix — one row per member (never the tenant summed under one name)", () => {
  it("buckets each member-day into its own row and column", () => {
    const { members, matrix } = buildWeekMatrix(
      [
        { userId: "a", date: "2026-09-21T00:00:00Z", trackedMinutes: 60 },
        { userId: "b", date: "2026-09-23T00:00:00", trackedMinutes: 125.4 }, // the local day, written without a zone
        { userId: "a", date: "2026-09-27", trackedMinutes: 30 },
      ],
      week,
      roster,
    );
    expect(members.map((m) => m.id)).toEqual(["a", "b"]);
    expect(matrix[0]).toEqual([60, 0, 0, 0, 0, 0, 30]);
    expect(matrix[1]).toEqual([0, 0, 125, 0, 0, 0, 0]);
  });

  it("keeps roster members with no time as a row of zeros", () => {
    const { matrix } = buildWeekMatrix([], week, roster);
    expect(matrix).toEqual([Array(7).fill(0), Array(7).fill(0)]);
  });

  it("appends members the roster doesn't know, named by the backend", () => {
    const { members, matrix } = buildWeekMatrix(
      [
        { userId: "c", userName: "carol", date: "2026-09-22", trackedMinutes: 10 },
        { userId: "d", userName: "  ", date: "2026-09-22", trackedMinutes: 5 },
      ],
      week,
      roster,
    );
    expect(members.map((m) => m.name)).toEqual(["Ann", "Ben", "carol", "Member"]);
    expect(matrix[2][1]).toBe(10);
    expect(matrix[3][1]).toBe(5);
  });

  it("sums a member-day reported twice and ignores rows outside the week or malformed", () => {
    const { matrix, members } = buildWeekMatrix(
      [
        { userId: "a", date: "2026-09-24", trackedMinutes: 20 },
        { userId: "a", date: "2026-09-24", trackedMinutes: 22 },
        { userId: "a", date: "2026-09-20", trackedMinutes: 999 }, // Sunday before
        { userId: "a", date: "2026-09-28", trackedMinutes: 999 }, // Monday after
        { userId: "a", date: "garbage", trackedMinutes: 999 },
        { userId: "a", date: "2026-09-25", trackedMinutes: -40 }, // negative clamps to 0
        { userId: "a", date: "2026-09-26", trackedMinutes: Number.NaN },
        { userId: "a", date: "2026-09-26", trackedMinutes: null },
        null as unknown as { userId: string; date: string },
      ],
      week,
      roster,
    );
    expect(members).toHaveLength(2);
    expect(matrix[0]).toEqual([0, 0, 0, 42, 0, 0, 0]);
  });

  it("tolerates a non-array payload", () => {
    const { matrix } = buildWeekMatrix(null as unknown as [], week, roster);
    expect(matrix.flat().every((v) => v === 0)).toBe(true);
  });
});

describe("timesheetWeek — Monday to Sunday in the reporting zone", () => {
  it("finds the week holding today in the zone, with its bounds and the span to show", () => {
    // Sun 27 Sep 20:00 UTC is already Mon 28 Sep in Dhaka (UTC+6): a new week there.
    const now = new Date("2026-09-27T20:00:00.000Z");
    const utc = timesheetWeek(now, "UTC");
    expect(utc).toMatchObject({ days: week, today: "2026-09-27", span: "21 Sep – 27 Sep 2026" });
    expect(utc.from.toISOString()).toBe("2026-09-21T00:00:00.000Z");
    expect(utc.to.toISOString()).toBe("2026-09-28T00:00:00.000Z");

    const dhaka = timesheetWeek(now, "Asia/Dhaka");
    expect(dhaka.days[0]).toBe("2026-09-28");
    expect(dhaka.today).toBe("2026-09-28");
    expect(dhaka.from.toISOString()).toBe("2026-09-27T18:00:00.000Z");
    expect(dhaka.span).toBe("28 Sep – 4 Oct 2026");
  });

  it("steps back whole weeks, and never forward past this one", () => {
    const now = new Date("2026-09-23T12:00:00.000Z");
    expect(timesheetWeek(now, "UTC", -1).days[0]).toBe("2026-09-14");
    expect(timesheetWeek(now, "UTC", -1).span).toBe("14 Sep – 20 Sep 2026");
    expect(timesheetWeek(now, "UTC", 2).days[0]).toBe("2026-09-21");
    expect(timesheetWeek(new Date("2026-12-30T12:00:00.000Z"), "UTC").span).toBe("28 Dec 2026 – 3 Jan 2027");
  });
});

describe("weekMatrixFromActivities — the demo's grid, from its own rows", () => {
  const row = (over: Partial<Activity>): Activity => ({
    id: "x",
    userId: "a",
    projectId: "p",
    startedAt: "2026-09-21T09:00:00.000Z",
    endedAt: "2026-09-21T09:10:00.000Z",
    description: "",
    productivity: 50,
    mouseClicks: 0,
    keyboardHits: 0,
    activeWindows: [],
    runningPrograms: [],
    screen: { app: "App", kind: "docs", accent: "#000" },
    hasWebcam: false,
    online: false,
    ...over,
  });

  it("adds each block's length on the day it started, for roster members only", () => {
    const { matrix } = weekMatrixFromActivities(
      [
        row({ id: "1" }),
        row({ id: "2", startedAt: "2026-09-21T10:00:00.000Z", endedAt: "2026-09-21T10:20:30.000Z" }),
        row({ id: "3", userId: "b", startedAt: "2026-09-27T23:55:00.000Z", endedAt: "2026-09-28T00:05:00.000Z" }),
        row({ id: "4", userId: "stranger" }),
        row({ id: "5", startedAt: "2026-09-28T09:00:00.000Z", endedAt: "2026-09-28T09:10:00.000Z" }), // next week
        row({ id: "6", endedAt: "garbage" }),
      ],
      week,
      roster,
      "UTC",
    );
    expect(matrix[0]).toEqual([31, 0, 0, 0, 0, 0, 0]);
    expect(matrix[1]).toEqual([0, 0, 0, 0, 0, 0, 10]);
  });

  it("reads the start in the zone", () => {
    const late = row({ startedAt: "2026-09-21T20:00:00.000Z", endedAt: "2026-09-21T20:30:00.000Z" });
    expect(weekMatrixFromActivities([late], week, roster, "Asia/Dhaka").matrix[0]).toEqual([0, 30, 0, 0, 0, 0, 0]);
  });
});

describe("heatStep and averagePerWorkedDay", () => {
  it("steps by whole hours of a day", () => {
    expect([0, -5, 1, 119, 120, 239, 240, 359, 360, 900].map(heatStep)).toEqual([0, 0, 1, 1, 2, 2, 3, 3, 4, 4]);
  });

  it("averages over the days with tracked time — a 38-hour week of 5 days is 7h 36m a day, not 38h", () => {
    expect(averagePerWorkedDay([480, 450, 0, 510, 480, 360, 0])).toEqual({ minutes: 456, days: 5 });
    expect(averagePerWorkedDay([0, 0, 0])).toEqual({ minutes: 0, days: 0 });
  });
});
