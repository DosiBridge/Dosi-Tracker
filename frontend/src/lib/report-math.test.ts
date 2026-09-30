import { describe, it, expect } from "vitest";
import {
  ALL,
  activeDaysByUser,
  activitySplit,
  asArray,
  computePayroll,
  laborCost,
  memberName,
  ratesByMember,
  rangeForKeyAt,
  reportQuery,
  safePercent,
  type MemberRate,
  type UserDayRow,
} from "./report-math";
import { rangeForKey, rangePresets, type RangeKey } from "./reports-data";
import { NOW } from "./mock-data";

const KEYS: RangeKey[] = rangePresets.map((p) => p.key);

describe("rangeForKeyAt", () => {
  it("matches the demo presets exactly when pivoted on the demo clock", () => {
    for (const key of KEYS) {
      const at = rangeForKeyAt(key, new Date(NOW));
      const demo = rangeForKey(key);
      expect(at.from.getTime(), key).toBe(demo.from.getTime());
      expect(at.to.getTime(), key).toBe(demo.to.getTime());
      expect(at.label, key).toBe(demo.label);
    }
  });

  it("pivots on the clock it is given, not the demo clock", () => {
    const now = new Date(2030, 4, 20, 15, 30, 0, 0);
    const today = rangeForKeyAt("today", now);
    expect(today.from.getTime()).toBe(new Date(2030, 4, 20, 0, 0, 0, 0).getTime());
    expect(today.to.getTime()).toBe(now.getTime());

    const yesterday = rangeForKeyAt("yesterday", now);
    expect(yesterday.from.getTime()).toBe(new Date(2030, 4, 19, 0, 0, 0, 0).getTime());
    expect(yesterday.to.getTime()).toBe(new Date(2030, 4, 19, 23, 59, 59, 999).getTime());

    expect(rangeForKeyAt("7d", now).from.getTime()).toBe(new Date(2030, 4, 13, 15, 30).getTime());
    expect(rangeForKeyAt("30d", now).from.getTime()).toBe(new Date(2030, 3, 20, 15, 30).getTime());
    expect(rangeForKeyAt("month", now).from.getTime()).toBe(new Date(2030, 4, 1).getTime());
    expect(rangeForKeyAt("custom", now)).toMatchObject({ label: "Custom" });
    expect(rangeForKeyAt("custom", now).from.getTime()).toBe(new Date(2030, 4, 13, 15, 30).getTime());
  });

  it("never mutates the clock it is given", () => {
    const now = new Date(2030, 0, 15, 10, 0);
    const before = now.getTime();
    for (const key of KEYS) rangeForKeyAt(key, now);
    expect(now.getTime()).toBe(before);
  });
});

describe("reportQuery", () => {
  const from = new Date("2030-01-01T00:00:00.000Z");
  const to = new Date("2030-01-08T00:00:00.000Z");

  it("always carries the ISO range", () => {
    const qs = new URLSearchParams(reportQuery({ from, to }));
    expect(qs.get("From")).toBe("2030-01-01T00:00:00.000Z");
    expect(qs.get("To")).toBe("2030-01-08T00:00:00.000Z");
    expect(qs.has("ProjectId")).toBe(false);
    expect(qs.has("UserId")).toBe(false);
  });

  it("sends real selections and drops 'all' / empty ones", () => {
    const qs = new URLSearchParams(reportQuery({ from, to, projectId: "p-1", userId: "u-1" }));
    expect(qs.get("ProjectId")).toBe("p-1");
    expect(qs.get("UserId")).toBe("u-1");

    const none = new URLSearchParams(reportQuery({ from, to, projectId: ALL, userId: "" }));
    expect(none.has("ProjectId")).toBe(false);
    expect(none.has("UserId")).toBe(false);

    const nulls = new URLSearchParams(reportQuery({ from, to, projectId: null, userId: ALL }));
    expect([...nulls.keys()]).toEqual(["From", "To"]);
  });
});

describe("asArray / safePercent / memberName", () => {
  it("asArray passes arrays through and maps anything else to []", () => {
    expect(asArray<number>([1, 2])).toEqual([1, 2]);
    expect(asArray(null)).toEqual([]);
    expect(asArray({ items: [1] })).toEqual([]);
  });

  it("safePercent never yields NaN or Infinity", () => {
    expect(safePercent(1, 4)).toBe(25);
    expect(safePercent(2, 3)).toBe(67);
    expect(safePercent(0, 0)).toBe(0);
    expect(safePercent(5, 0)).toBe(0);
    expect(safePercent(5, -1)).toBe(0);
    expect(safePercent(Number.NaN, 10)).toBe(0);
    expect(safePercent(1, Number.POSITIVE_INFINITY)).toBe(0);
  });

  it("memberName prefers the roster name and falls back to a short id label", () => {
    expect(memberName("0f7c9a2e-1111-2222-3333-444455556666", { name: "Rina Das" })).toBe("Rina Das");
    expect(memberName("0f7c9a2e-1111-2222-3333-444455556666", { name: "   " })).toBe("Member 0f7c9a2e");
    expect(memberName("0f7c9a2e-1111", null)).toBe("Member 0f7c9a2e");
    expect(memberName("abc")).toBe("Member abc");
  });
});

describe("ratesByMember / laborCost", () => {
  it("maps members to their rate, treating missing/invalid rates as unpriced", () => {
    const rates = ratesByMember([
      { userId: "a", hourlyRate: 40 },
      { userId: "b", hourlyRate: null },
      { userId: "c" },
      { userId: "d", hourlyRate: -5 },
      { userId: "a", hourlyRate: 30 }, // duplicate membership keeps the higher rate
      null as unknown as MemberRate,
      { userId: 7 } as unknown as MemberRate,
    ]);
    expect(rates.get("a")).toBe(40);
    expect(rates.get("b")).toBe(0);
    expect(rates.get("c")).toBe(0);
    expect(rates.get("d")).toBe(0);
    expect(rates.size).toBe(4);
  });

  it("costs a project as each member's hours × their rate, skipping unpriced time", () => {
    const rates = new Map([
      ["a", 40],
      ["b", 0],
    ]);
    const cost = laborCost(
      [
        { userId: "a", trackedMinutes: 90 },
        { userId: "b", trackedMinutes: 60 },
        { userId: "c", trackedMinutes: 30 }, // no membership at all
        { userId: "a", trackedMinutes: Number.NaN },
      ],
      rates,
    );
    expect(cost).toEqual({ minutes: 180, pricedMinutes: 90, cost: 60 });
  });
});

describe("computePayroll", () => {
  it("sums each project's own rate × hours — not the highest rate × all hours", () => {
    const lines = computePayroll(
      [{ userId: "a", trackedMinutes: 180 }],
      [
        { projectId: "p1", perUser: [{ userId: "a", trackedMinutes: 120 }], rates: new Map([["a", 50]]) },
        { projectId: "p2", perUser: [{ userId: "a", trackedMinutes: 60 }], rates: new Map([["a", 20]]) },
      ],
    );
    expect(lines).toEqual([
      { userId: "a", minutes: 180, hours: 3, pricedHours: 3, amount: 120, rate: 40, projectCount: 2 },
    ]);
    // The old max-rate formula would have paid 3h × $50 = $150.
    expect(lines[0].amount).not.toBe(150);
  });

  it("keeps unpriced hours in the hours but not in the pay (blended rate drops)", () => {
    const [line] = computePayroll(
      [{ userId: "a", trackedMinutes: 120 }],
      [
        { projectId: "p1", perUser: [{ userId: "a", trackedMinutes: 60 }], rates: new Map([["a", 30]]) },
        { projectId: "p2", perUser: [{ userId: "a", trackedMinutes: 60 }], rates: new Map() },
      ],
    );
    expect(line).toMatchObject({ hours: 2, pricedHours: 1, amount: 30, rate: 15, projectCount: 2 });
  });

  it("lists members with hours but no priced project at a zero rate, and members seen only per project", () => {
    const lines = computePayroll(
      [
        { userId: "idle", trackedMinutes: 0 },
        { userId: "unpriced", trackedMinutes: 45 },
      ],
      [
        {
          projectId: "p1",
          perUser: [
            { userId: "unpriced", trackedMinutes: 45 },
            { userId: "late", trackedMinutes: 30 },
            { userId: "idle", trackedMinutes: 0 },
          ],
          rates: new Map([["late", 10]]),
        },
      ],
    );
    const byId = new Map(lines.map((l) => [l.userId, l]));
    expect(byId.get("idle")).toEqual({ userId: "idle", minutes: 0, hours: 0, pricedHours: 0, amount: 0, rate: 0, projectCount: 0 });
    expect(byId.get("unpriced")).toMatchObject({ hours: 0.75, amount: 0, rate: 0, projectCount: 1 });
    expect(byId.get("late")).toMatchObject({ minutes: 30, hours: 0.5, amount: 5, rate: 10, projectCount: 1 });
  });

  it("rounds money and hours to cents", () => {
    const [line] = computePayroll(
      [{ userId: "a", trackedMinutes: 100 }],
      [{ projectId: "p1", perUser: [{ userId: "a", trackedMinutes: 100 }], rates: new Map([["a", 33.33]]) }],
    );
    expect(line.hours).toBe(1.67);
    expect(line.amount).toBe(55.55);
    expect(line.rate).toBe(33.33);
  });

  it("returns nothing for no input", () => {
    expect(computePayroll([], [])).toEqual([]);
  });
});

describe("activitySplit", () => {
  it("splits tracked time into the active share and the rest", () => {
    expect(activitySplit(200, 75)).toEqual({ productive: 150, other: 50, focus: 75 });
  });

  it("clamps the score and survives garbage input without NaN", () => {
    expect(activitySplit(100, 140)).toEqual({ productive: 100, other: 0, focus: 100 });
    expect(activitySplit(100, -5)).toEqual({ productive: 0, other: 100, focus: 0 });
    expect(activitySplit(100, Number.NaN)).toEqual({ productive: 0, other: 100, focus: 0 });
    expect(activitySplit(Number.NaN, 50)).toEqual({ productive: 0, other: 0, focus: 0 });
    expect(activitySplit(0, 0)).toEqual({ productive: 0, other: 0, focus: 0 });
  });
});

describe("activeDaysByUser", () => {
  it("counts distinct UTC days per member and ignores malformed rows", () => {
    const days = activeDaysByUser([
      { userId: "a", date: "2030-01-01T00:00:00Z" },
      { userId: "a", date: "2030-01-01T00:00:00" }, // same day, other serialization
      { userId: "a", date: "2030-01-02T00:00:00Z" },
      { userId: "b", date: "2030-01-02T00:00:00Z" },
      null as unknown as UserDayRow,
      { userId: "c" } as unknown as UserDayRow,
    ]);
    expect(days.get("a")).toBe(2);
    expect(days.get("b")).toBe(1);
    expect(days.has("c")).toBe(false);
  });

  it("is empty for no rows", () => {
    expect(activeDaysByUser([]).size).toBe(0);
  });
});
