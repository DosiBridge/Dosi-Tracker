import { describe, expect, it } from "vitest";
import { buildWeekMatrix, type GridMember } from "./timesheet";

const monday = new Date("2026-09-21T00:00:00.000Z");
const roster: GridMember[] = [
  { id: "a", name: "Ann", designation: "Owner", status: "active" },
  { id: "b", name: "Ben", designation: "Member", status: "offline" },
];

describe("buildWeekMatrix — one row per member (never the tenant summed under one name)", () => {
  it("buckets each member-day into its own row and column", () => {
    const { members, matrix } = buildWeekMatrix(
      [
        { userId: "a", date: "2026-09-21T00:00:00Z", trackedMinutes: 60 },
        { userId: "b", date: "2026-09-23T00:00:00", trackedMinutes: 125.4 }, // no zone suffix
        { userId: "a", date: "2026-09-27", trackedMinutes: 30 },
      ],
      monday,
      roster,
    );
    expect(members.map((m) => m.id)).toEqual(["a", "b"]);
    expect(matrix[0]).toEqual([60, 0, 0, 0, 0, 0, 30]);
    expect(matrix[1]).toEqual([0, 0, 125, 0, 0, 0, 0]);
  });

  it("keeps roster members with no time as a row of zeros", () => {
    const { matrix } = buildWeekMatrix([], monday, roster);
    expect(matrix).toEqual([Array(7).fill(0), Array(7).fill(0)]);
  });

  it("appends members the roster doesn't know, named by the backend", () => {
    const { members, matrix } = buildWeekMatrix(
      [
        { userId: "c", userName: "carol", date: "2026-09-22", trackedMinutes: 10 },
        { userId: "d", userName: "  ", date: "2026-09-22", trackedMinutes: 5 },
      ],
      monday,
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
      monday,
      roster,
    );
    expect(members).toHaveLength(2);
    expect(matrix[0]).toEqual([0, 0, 0, 42, 0, 0, 0]);
  });

  it("tolerates a non-array payload", () => {
    const { matrix } = buildWeekMatrix(null as unknown as [], monday, roster);
    expect(matrix.flat().every((v) => v === 0)).toBe(true);
  });
});
