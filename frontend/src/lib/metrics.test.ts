import { beforeEach, describe, expect, it } from "vitest";
import { buildActivity, resetFactories } from "@/test/factories";
import {
  ACTIVITY_METRIC,
  IDLE_METRIC,
  activeMinutes,
  activitiesOnZonedDay,
  activityPercent,
  activityPercentOf,
  averageProductivity,
  dayKey,
  dayKeyIn,
  formatActivityPercent,
  idleMinutesOf,
  idleSecondsOf,
  isActivityMeasured,
  measuredMinutes,
  minutesOnZonedDay,
  totalIdleMinutes,
  distinctActiveDays,
  durationMinutes,
  inRange,
  minutesByProject,
  minutesByUser,
  minutesForProject,
  minutesForUser,
  minutesInRange,
  minutesOnDay,
  reconcileSegmentProductivity,
  totalMinutes,
  trimSegmentsToTarget,
} from "./metrics";

const at = (iso: string) => new Date(iso);

/** A row of `mins` minutes ending at `endIso`. */
function block(endIso: string, mins: number, extra: Parameters<typeof buildActivity>[0] = {}) {
  return buildActivity({
    startedAt: new Date(Date.parse(endIso) - mins * 60_000).toISOString(),
    endedAt: endIso,
    ...extra,
  });
}

beforeEach(resetFactories);

describe("durationMinutes", () => {
  it("measures the wall-clock span, not the project's capture interval", () => {
    expect(durationMinutes(block("2026-07-14T15:00:00.000Z", 47))).toBe(47);
  });

  it("is 0 for a zero-length row", () => {
    const a = buildActivity({
      startedAt: "2026-07-14T09:00:00.000Z",
      endedAt: "2026-07-14T09:00:00.000Z",
    });
    expect(durationMinutes(a)).toBe(0);
  });

  it("clamps a reversed row to 0 rather than subtracting time from a total", () => {
    const a = buildActivity({
      startedAt: "2026-07-14T10:00:00.000Z",
      endedAt: "2026-07-14T09:00:00.000Z",
    });
    expect(durationMinutes(a)).toBe(0);
  });

  it("is 0 for an unparseable timestamp", () => {
    expect(durationMinutes(buildActivity({ startedAt: "not-a-date" }))).toBe(0);
  });

  it("keeps sub-minute precision", () => {
    expect(durationMinutes(block("2026-07-14T15:00:00.000Z", 0.5))).toBe(0.5);
  });
});

describe("totalMinutes", () => {
  it("is 0 for empty input", () => {
    expect(totalMinutes([])).toBe(0);
  });

  it("is the row's own duration for a single row", () => {
    expect(totalMinutes([block("2026-07-14T12:00:00.000Z", 62)])).toBe(62);
  });

  it("sums rows of differing lengths", () => {
    const list = [
      block("2026-07-14T10:00:00.000Z", 35),
      block("2026-07-14T11:30:00.000Z", 75),
      block("2026-07-14T12:00:00.000Z", 0),
    ];
    expect(totalMinutes(list)).toBe(110);
  });
});

describe("minutesInRange", () => {
  const from = at("2026-07-14T00:00:00.000Z");
  const to = at("2026-07-14T23:59:59.999Z");

  it("is 0 for empty input", () => {
    expect(minutesInRange([], from, to)).toBe(0);
  });

  it("books a row that STARTS before the window but ends inside it in full", () => {
    // A session straddling midnight belongs to the day it landed on — the whole
    // block, not a prorated slice. One rule, applied everywhere, beats a split
    // that only some surfaces would remember to do.
    const straddler = block("2026-07-14T00:20:00.000Z", 50); // started 23:30 the previous day
    expect(minutesInRange([straddler], from, to)).toBe(50);
  });

  it("excludes a row that ended just before the window opened", () => {
    const before = block("2026-07-13T23:59:59.998Z", 40);
    expect(minutesInRange([before], from, to)).toBe(0);
  });

  it("includes rows landing exactly on either boundary", () => {
    const onOpen = block("2026-07-14T00:00:00.000Z", 10);
    const onClose = block("2026-07-14T23:59:59.999Z", 20);
    expect(minutesInRange([onOpen, onClose], from, to)).toBe(30);
  });

  it("ignores rows that land after the window closes", () => {
    expect(minutesInRange([block("2026-07-15T00:00:00.001Z", 60)], from, to)).toBe(0);
  });
});

describe("inRange / dayKey", () => {
  it("keys a row to the UTC day it ended on", () => {
    expect(dayKey(block("2026-07-14T00:05:00.000Z", 45))).toBe("2026-07-14");
  });

  it("reports membership on the endedAt instant", () => {
    const a = block("2026-07-14T12:00:00.000Z", 30);
    expect(inRange(a, at("2026-07-14T11:59:00.000Z"), at("2026-07-14T12:01:00.000Z"))).toBe(true);
    expect(inRange(a, at("2026-07-14T12:01:00.000Z"), at("2026-07-14T13:00:00.000Z"))).toBe(false);
  });
});

describe("minutesForUser / minutesForProject", () => {
  const list = [
    block("2026-07-14T10:00:00.000Z", 60, { userId: "u1", projectId: "p1" }),
    block("2026-07-14T12:00:00.000Z", 45, { userId: "u1", projectId: "p2" }),
    block("2026-07-13T10:00:00.000Z", 90, { userId: "u1", projectId: "p1" }),
    block("2026-07-14T10:00:00.000Z", 30, { userId: "u2", projectId: "p1" }),
  ];

  it("is 0 for empty input and for an unknown member", () => {
    expect(minutesForUser([], "u1")).toBe(0);
    expect(minutesForUser(list, "nobody")).toBe(0);
  });

  it("sums every row for the member when no window is given", () => {
    expect(minutesForUser(list, "u1")).toBe(195);
  });

  it("narrows to the window when one is given", () => {
    const from = at("2026-07-14T00:00:00.000Z");
    const to = at("2026-07-14T23:59:59.999Z");
    expect(minutesForUser(list, "u1", from, to)).toBe(105);
  });

  it("splits the same rows by project without double counting", () => {
    expect(minutesForProject(list, "p1")).toBe(180);
    expect(minutesForProject(list, "p2")).toBe(45);
    expect(minutesForProject(list, "p1") + minutesForProject(list, "p2")).toBe(totalMinutes(list));
  });
});

describe("minutesOnDay", () => {
  const list = [
    block("2026-07-14T10:00:00.000Z", 60, { userId: "u1" }),
    block("2026-07-14T23:50:00.000Z", 40, { userId: "u2" }),
    block("2026-07-13T10:00:00.000Z", 20, { userId: "u1" }),
  ];

  it("totals a UTC day for the whole team or one member", () => {
    expect(minutesOnDay(list, "2026-07-14")).toBe(100);
    expect(minutesOnDay(list, "2026-07-14", "u1")).toBe(60);
    expect(minutesOnDay(list, "2026-07-12")).toBe(0);
  });
});

describe("averageProductivity", () => {
  it("is 0 for empty input", () => {
    expect(averageProductivity([])).toBe(0);
  });

  it("returns the row's own score for a single row", () => {
    expect(averageProductivity([block("2026-07-14T10:00:00.000Z", 30, { productivity: 73 })])).toBe(73);
  });

  it("weights by duration, so a long mediocre block outweighs a short great one", () => {
    const list = [
      block("2026-07-14T10:00:00.000Z", 90, { productivity: 50 }),
      block("2026-07-14T11:00:00.000Z", 10, { productivity: 100 }),
    ];
    // Unweighted this would read 75; the honest answer is 55.
    expect(averageProductivity(list)).toBe(55);
  });

  it("ignores zero-length rows instead of dividing by zero", () => {
    const list = [
      block("2026-07-14T10:00:00.000Z", 0, { productivity: 99 }),
      block("2026-07-14T11:00:00.000Z", 60, { productivity: 40 }),
    ];
    expect(averageProductivity(list)).toBe(40);
    expect(averageProductivity([block("2026-07-14T10:00:00.000Z", 0, { productivity: 99 })])).toBe(0);
  });
});

describe("distinctActiveDays", () => {
  it("is 0 for empty input", () => {
    expect(distinctActiveDays([], "u1")).toBe(0);
  });

  it("counts UTC days, collapsing several sessions on one day", () => {
    const list = [
      block("2026-07-14T10:00:00.000Z", 60, { userId: "u1" }),
      block("2026-07-14T16:00:00.000Z", 60, { userId: "u1" }),
      block("2026-07-13T10:00:00.000Z", 60, { userId: "u1" }),
      block("2026-07-12T10:00:00.000Z", 60, { userId: "u2" }),
    ];
    expect(distinctActiveDays(list, "u1")).toBe(2);
    expect(distinctActiveDays(list, "u2")).toBe(1);
  });

  it("does not count a day whose only row is zero-length", () => {
    const list = [
      block("2026-07-14T10:00:00.000Z", 60, { userId: "u1" }),
      block("2026-07-13T10:00:00.000Z", 0, { userId: "u1" }),
    ];
    expect(distinctActiveDays(list, "u1")).toBe(1);
  });
});

describe("minutesByUser / minutesByProject", () => {
  it("are empty maps for empty input", () => {
    expect(minutesByUser([]).size).toBe(0);
    expect(minutesByProject([]).size).toBe(0);
  });

  it("agree with the per-entity helpers over the same rows", () => {
    const list = [
      block("2026-07-14T10:00:00.000Z", 55, { userId: "u1", projectId: "p1" }),
      block("2026-07-14T12:00:00.000Z", 65, { userId: "u2", projectId: "p1" }),
      block("2026-07-14T14:00:00.000Z", 25, { userId: "u1", projectId: "p2" }),
    ];
    const byUser = minutesByUser(list);
    expect(byUser.get("u1")).toBe(minutesForUser(list, "u1"));
    expect(byUser.get("u2")).toBe(minutesForUser(list, "u2"));
    expect(minutesByProject(list).get("p1")).toBe(minutesForProject(list, "p1"));
    expect([...byUser.values()].reduce((s, v) => s + v, 0)).toBe(totalMinutes(list));
  });
});

describe("trimSegmentsToTarget", () => {
  const seg = (minutes: number, counts = true) => ({ minutes, endMin: 100 + minutes, productivity: 50, counts });

  it("trims the tail so counted minutes land exactly on target", () => {
    const segs = [seg(30), seg(30), seg(30)];
    trimSegmentsToTarget(segs, 70);
    expect(segs.reduce((s, x) => s + x.minutes, 0)).toBe(70);
    // The trim comes off the end, leaving earlier segments untouched.
    expect(segs[0].minutes).toBe(30);
  });

  it("keeps endMin consistent with the trimmed duration", () => {
    const segs = [{ minutes: 60, endMin: 160, productivity: 50, counts: true }];
    trimSegmentsToTarget(segs, 45);
    expect(segs[0].minutes).toBe(45);
    expect(segs[0].endMin).toBe(145);
  });

  it("drops segments that are trimmed away entirely", () => {
    const segs = [seg(30), seg(30), seg(30)];
    trimSegmentsToTarget(segs, 30);
    expect(segs).toHaveLength(1);
    expect(segs[0].minutes).toBe(30);
  });

  it("never trims segments that do not count toward the total", () => {
    const segs = [seg(30), seg(20, false), seg(30)];
    trimSegmentsToTarget(segs, 40);
    expect(segs.filter((s) => s.counts).reduce((s, x) => s + x.minutes, 0)).toBe(40);
    expect(segs.find((s) => !s.counts)!.minutes).toBe(20); // the break survives intact
  });

  it("leaves everything alone when already at or under target", () => {
    const segs = [seg(30), seg(30)];
    trimSegmentsToTarget(segs, 90);
    expect(segs.map((s) => s.minutes)).toEqual([30, 30]);
  });

  it("handles an empty timeline", () => {
    const segs: ReturnType<typeof seg>[] = [];
    expect(() => trimSegmentsToTarget(segs, 30)).not.toThrow();
    expect(segs).toHaveLength(0);
  });
});

describe("reconcileSegmentProductivity", () => {
  const seg = (minutes: number, productivity: number, counts = true) => ({
    minutes,
    endMin: 100 + minutes,
    productivity,
    counts,
  });
  const weighted = (segs: { minutes: number; productivity: number; counts: boolean }[]) => {
    const counted = segs.filter((s) => s.counts);
    const total = counted.reduce((s, x) => s + x.minutes, 0);
    return counted.reduce((s, x) => s + x.minutes * x.productivity, 0) / total;
  };

  it("raises a low day to the target average", () => {
    const segs = [seg(60, 50), seg(60, 60), seg(60, 55)];
    reconcileSegmentProductivity(segs, 80);
    expect(Math.round(weighted(segs))).toBe(80);
  });

  it("lowers a high day to the target average", () => {
    const segs = [seg(60, 95), seg(30, 90)];
    reconcileSegmentProductivity(segs, 70);
    expect(Math.round(weighted(segs))).toBe(70);
  });

  it("preserves relative shape — a weaker block stays weaker", () => {
    const segs = [seg(60, 40), seg(60, 90)];
    reconcileSegmentProductivity(segs, 75);
    expect(segs[0].productivity).toBeLessThan(segs[1].productivity);
  });

  it("weights by duration, not by segment count", () => {
    // One long low block should pull harder than one short high block.
    const segs = [seg(180, 40), seg(20, 90)];
    reconcileSegmentProductivity(segs, 60);
    expect(Math.round(weighted(segs))).toBe(60);
  });

  it("ignores segments that do not count toward tracked time", () => {
    const segs = [seg(60, 50), seg(30, 0, false)];
    reconcileSegmentProductivity(segs, 80);
    expect(segs[1].productivity).toBe(0); // the break is untouched
    expect(Math.round(weighted(segs))).toBe(80);
  });

  it("keeps every value inside 0-100", () => {
    const segs = [seg(60, 10), seg(60, 15)];
    reconcileSegmentProductivity(segs, 99);
    for (const s of segs) {
      expect(s.productivity).toBeGreaterThanOrEqual(0);
      expect(s.productivity).toBeLessThanOrEqual(100);
    }
  });

  it("does nothing when there is no counted time", () => {
    const segs = [seg(30, 0, false)];
    expect(() => reconcileSegmentProductivity(segs, 80)).not.toThrow();
    expect(segs[0].productivity).toBe(0);
  });
});

describe("the Activity glossary", () => {
  it("names the metric once, for every screen", () => {
    expect(ACTIVITY_METRIC.label).toBe("Activity");
    expect(ACTIVITY_METRIC.description).toMatch(/keyboard or mouse input/);
    expect(ACTIVITY_METRIC.notMeasured).toBe("Not measured");
    expect(IDLE_METRIC.label).toBe("Idle");
  });
});

describe("Activity % — minute-weighted over MEASURED blocks", () => {
  const measured = (end: string, mins: number, pct: number) => block(end, mins, { productivity: pct });
  const unmeasured = (end: string, mins: number) => block(end, mins, { productivity: 0, activityMeasured: false });

  it("a block is measured unless it says otherwise (demo and older rows say nothing)", () => {
    expect(isActivityMeasured(measured("2026-07-14T10:00:00.000Z", 10, 50))).toBe(true);
    expect(isActivityMeasured(unmeasured("2026-07-14T10:00:00.000Z", 10))).toBe(false);
    expect(isActivityMeasured(block("2026-07-14T10:00:00.000Z", 10, { activityMeasured: true }))).toBe(true);
  });

  it("activityPercentOf: the block's %, rounded and clamped, or null when not measured", () => {
    expect(activityPercentOf(measured("2026-07-14T10:00:00.000Z", 10, 72.4))).toBe(72);
    expect(activityPercentOf(measured("2026-07-14T10:00:00.000Z", 10, 140))).toBe(100);
    expect(activityPercentOf(measured("2026-07-14T10:00:00.000Z", 10, -5))).toBe(0);
    expect(activityPercentOf(unmeasured("2026-07-14T10:00:00.000Z", 10))).toBeNull();
    expect(activityPercentOf(block("2026-07-14T10:00:00.000Z", 10, { productivity: Number.NaN }))).toBeNull();
  });

  it("leaves unmeasured blocks out instead of counting them as 0%", () => {
    const rows = [measured("2026-07-14T10:00:00.000Z", 60, 80), unmeasured("2026-07-14T12:00:00.000Z", 60)];
    expect(activityPercent(rows)).toBe(80); // not 40
    expect(averageProductivity(rows)).toBe(80);
    expect(measuredMinutes(rows)).toBe(60);
    expect(activeMinutes(rows)).toBe(48);
  });

  it("weights by minutes and rounds once, at the end", () => {
    const rows = [measured("2026-07-14T10:00:00.000Z", 90, 50), measured("2026-07-14T11:00:00.000Z", 10, 95.5)];
    // (90 × 50 + 10 × 95.5) / 100 = 54.55
    expect(activityPercent(rows)).toBe(55);
    expect(activeMinutes(rows)).toBeCloseTo(54.55, 5);
  });

  it("is null when nothing was measured — and the legacy average reads 0 for it", () => {
    expect(activityPercent([])).toBeNull();
    expect(activityPercent([unmeasured("2026-07-14T10:00:00.000Z", 30)])).toBeNull();
    expect(activityPercent([measured("2026-07-14T10:00:00.000Z", 0, 90)])).toBeNull(); // no minutes to rate
    expect(averageProductivity([unmeasured("2026-07-14T10:00:00.000Z", 30)])).toBe(0);
  });

  it("formats as a percentage, or a dash when there is nothing to show", () => {
    expect(formatActivityPercent(72)).toBe("72%");
    expect(formatActivityPercent(71.6)).toBe("72%");
    expect(formatActivityPercent(0)).toBe("0%");
    expect(formatActivityPercent(null)).toBe("—");
    expect(formatActivityPercent(undefined)).toBe("—");
    expect(formatActivityPercent(Number.NaN)).toBe("—");
  });
});

describe("idle time", () => {
  /** A block of `mins` minutes starting 10:00 UTC with the given timeline buckets. */
  const withTimeline = (mins: number, buckets: { minute: number; active: boolean }[], extra: Parameters<typeof buildActivity>[0] = {}) =>
    buildActivity({
      startedAt: "2026-07-14T10:00:00.000Z",
      endedAt: new Date(Date.parse("2026-07-14T10:00:00.000Z") + mins * 60_000).toISOString(),
      timeline: buckets.map((b) => ({ ...b, keyboardHits: b.active ? 5 : 0, mouseClicks: 0 })),
      ...extra,
    });

  it("takes the agent's own idle seconds, clamped to the block", () => {
    expect(idleSecondsOf(block("2026-07-14T10:10:00.000Z", 10, { idleSeconds: 120 }))).toBe(120);
    expect(idleSecondsOf(block("2026-07-14T10:10:00.000Z", 10, { idleSeconds: 9999 }))).toBe(600);
    expect(idleSecondsOf(block("2026-07-14T10:10:00.000Z", 10, { idleSeconds: -4 }))).toBe(0);
    // A kept-idle block reports its whole length, measured or not.
    expect(idleSecondsOf(block("2026-07-14T10:10:00.000Z", 10, { idleSeconds: 600, activityMeasured: false }))).toBe(600);
  });

  it("otherwise sums the inactive timeline buckets; the last bucket absorbs the remainder (contract §1)", () => {
    // 10.4 minutes (624 s) → 10 buckets; the last one spans 624 − 9 × 60 = 84 s.
    const block10 = withTimeline(10.4, [
      { minute: 0, active: true },
      { minute: 3, active: false },
      { minute: 9, active: false },
    ]);
    expect(idleSecondsOf(block10)).toBe(60 + 84);
    expect(idleMinutesOf(block10)).toBe(2.4);
  });

  it("ignores buckets beyond the block and counts missing buckets as not idle", () => {
    expect(idleSecondsOf(withTimeline(5, [{ minute: 7, active: false }, { minute: 1, active: false }]))).toBe(60);
  });

  it("is unknown without a figure or a timeline, or when input wasn't measured", () => {
    expect(idleSecondsOf(block("2026-07-14T10:10:00.000Z", 10))).toBeNull();
    expect(idleSecondsOf(withTimeline(5, [{ minute: 0, active: false }], { activityMeasured: false }))).toBeNull();
    expect(idleMinutesOf(block("2026-07-14T10:10:00.000Z", 10))).toBeNull();
  });

  it("totalIdleMinutes adds the known idle time and nothing for unknown blocks", () => {
    const rows = [
      block("2026-07-14T10:10:00.000Z", 10, { idleSeconds: 180 }),
      block("2026-07-14T11:10:00.000Z", 10),
      withTimeline(4, [{ minute: 2, active: false }]),
    ];
    expect(totalIdleMinutes(rows)).toBe(4);
    expect(totalIdleMinutes([])).toBe(0);
  });
});

describe("calendar days of a named zone", () => {
  // Ends 20:30 UTC on the 14th: still the 14th in UTC and New York, already the 15th in Dhaka (UTC+6).
  const late = block("2026-07-14T20:30:00.000Z", 30);

  it("dayKeyIn books a block to the day its end falls on in that zone", () => {
    expect(dayKeyIn(late, "UTC")).toBe("2026-07-14");
    expect(dayKeyIn(late, "Asia/Dhaka")).toBe("2026-07-15");
    expect(dayKeyIn(late, "America/New_York")).toBe("2026-07-14");
    expect(dayKeyIn(buildActivity({ endedAt: "nope" }), "UTC")).toBe("");
  });

  it("activitiesOnZonedDay / minutesOnZonedDay group by the zone's day, optionally per member", () => {
    const early = block("2026-07-14T09:00:00.000Z", 60, { userId: "u2" });
    const rows = [late, early];
    expect(activitiesOnZonedDay(rows, "2026-07-15", "Asia/Dhaka")).toEqual([late]);
    expect(minutesOnZonedDay(rows, "2026-07-14", "Asia/Dhaka")).toBe(60);
    expect(minutesOnZonedDay(rows, "2026-07-14", "UTC")).toBe(90);
    expect(minutesOnZonedDay(rows, "2026-07-14", "UTC", "u2")).toBe(60);
  });
});
