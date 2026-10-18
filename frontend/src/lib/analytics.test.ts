import { describe, expect, it } from "vitest";
import {
  MAX_BLOCKS,
  OTHER,
  STRETCH_BUCKETS,
  activityHistogram,
  agentVersions,
  aggregateBlocks,
  appStats,
  appUsageFromBlocks,
  blocksFromApi,
  blocksStartingIn,
  busiestWeekday,
  clockLabel,
  coversDay,
  cumulative,
  dayLabel,
  dayStats,
  dayTimeline,
  devicesFromBlocks,
  focusStretches,
  gapKind,
  hourCellsFromBlocks,
  hourPieces,
  hourProfile,
  hourRangeLabel,
  hourTotals,
  layoutFlow,
  localSpan,
  localSpanLabel,
  longDayLabel,
  meanOf,
  memberClock,
  memberProjectFlow,
  memberProjectsFromBlocks,
  memberStats,
  movingAverage,
  paceTarget,
  parseDevices,
  parseHourly,
  peakHour,
  placeLabels,
  platformMix,
  projectDays,
  radarRows,
  rhythmMatrix,
  squarify,
  stretchHistogram,
  summarize,
  toBlocks,
  unmeasuredMinutes,
  weekdayOfKey,
  withBlockBounds,
  workWindows,
  zoneCalendar,
  type Block,
  type LabelPoint,
} from "./analytics";
import { APP_PALETTE, OTHER_APP_COLOR } from "./apps";
import { buildActivity, buildWindowInfo } from "@/test/factories";

/* Instants are written in UTC and read in a named zone, so every expectation
 * holds in whatever time zone the suite runs in. */

const at = (iso: string) => Date.parse(iso);
const UTC = zoneCalendar("UTC");

/** A GET /api/app/activity item (a bare timestamp is UTC, as ABP sends it). */
function dto(id: string, userId: string, projectId: string, startIso: string, minutes: number, extra: Record<string, unknown> = {}) {
  const start = at(/Z$/.test(startIso) ? startIso : `${startIso}Z`);
  return {
    id,
    userId,
    projectId,
    startedAt: startIso,
    endedAt: new Date(start + minutes * 60_000).toISOString(),
    productivity: 70,
    mouseClicks: 10,
    keyboardHits: 50,
    activeWindowsJson: JSON.stringify([{ AppName: "code.exe", WindowTitle: "main.rs", Seconds: minutes * 60 }]),
    ...extra,
  };
}

const blocksOf = (...rows: ReturnType<typeof dto>[]) => blocksFromApi(rows);

describe("zone calendar", () => {
  it("reads days, clock minutes and weekdays in the zone", () => {
    const dhaka = zoneCalendar("Asia/Dhaka");
    const t = at("2026-09-28T20:30:00Z"); // Tue 29 Sep, 02:30 in Dhaka
    expect(dhaka.offsetAt(t)).toBe(360);
    expect(dhaka.dayKey(t)).toBe("2026-09-29");
    expect(dhaka.minuteOfDay(t)).toBe(150);
    expect(dhaka.weekday(t)).toBe(1); // Tuesday, Monday-first
    expect(UTC.dayKey(t)).toBe("2026-09-28");
    const kathmandu = zoneCalendar("Asia/Kathmandu"); // +05:45
    expect(kathmandu.dayKey(at("2026-09-28T18:15:00Z"))).toBe("2026-09-29");
    expect(kathmandu.minuteOfDay(at("2026-09-28T18:15:00Z"))).toBe(0);
  });

  it("follows daylight saving: New York's day starts at 05:00 UTC in March, 04:00 after the switch", () => {
    const ny = zoneCalendar("America/New_York");
    expect(ny.offsetAt(at("2026-03-08T06:59:00Z"))).toBe(-300);
    expect(ny.offsetAt(at("2026-03-08T07:00:00Z"))).toBe(-240);
    expect(ny.dayKey(at("2026-03-08T04:59:00Z"))).toBe("2026-03-07");
    expect(ny.dayKey(at("2026-03-08T05:00:00Z"))).toBe("2026-03-08");
    expect(ny.dayStart("2026-03-08")).toBe(at("2026-03-08T05:00:00Z"));
    expect(ny.dayStart("2026-03-09")).toBe(at("2026-03-09T04:00:00Z"));
  });

  it("floors and ceils to whole local hours, even half-hour zones", () => {
    const kathmandu = zoneCalendar("Asia/Kathmandu");
    const t = at("2026-09-28T18:40:00Z"); // 00:25 local
    expect(kathmandu.hourFloor(t)).toBe(at("2026-09-28T18:15:00Z"));
    expect(kathmandu.hourCeil(t)).toBe(at("2026-09-28T19:15:00Z"));
    expect(kathmandu.hourCeil(at("2026-09-28T18:15:00Z"))).toBe(at("2026-09-28T18:15:00Z"));
  });

  it("an unknown zone reads as UTC, as the server does", () => {
    expect(zoneCalendar("Not/AZone").timeZone).toBe("UTC");
    expect(zoneCalendar("Not/AZone").dayKey(at("2026-09-28T23:30:00Z"))).toBe("2026-09-28");
  });
});

describe("labels", () => {
  it("keys, weekdays and labels", () => {
    expect(weekdayOfKey("2026-09-28")).toBe(0); // Monday
    expect(weekdayOfKey("2026-09-27")).toBe(6); // Sunday
    expect(dayLabel("2026-09-28")).toBe("Mon 28");
    expect(dayLabel("2026-09-28", true)).toBe("28 Sep");
    expect(longDayLabel("2026-10-04")).toBe("Sun, 4 Oct");
    expect(clockLabel(9 * 60 + 5)).toBe("09:05");
    expect(clockLabel(1440)).toBe("24:00");
    expect(clockLabel(-10)).toBe("00:00");
    expect(hourRangeLabel(9)).toBe("09:00–10:00");
  });
});

describe("blocks", () => {
  it("demo rows: drops unusable times, clamps scores, derives idle time, keeps app time and the note", () => {
    const blocks = toBlocks([
      buildActivity({ startedAt: "2026-09-28T10:00:00Z", endedAt: "2026-09-28T09:00:00Z" }), // ends before it starts
      buildActivity({ startedAt: "garbage", endedAt: "2026-09-28T10:00:00Z" }),
      buildActivity({
        id: "a1",
        startedAt: "2026-09-28T09:00:00Z",
        endedAt: "2026-09-28T09:30:00Z",
        productivity: 140,
        keyboardHits: -5,
        description: "  Fix   the login flow ",
        activeWindows: [buildWindowInfo({ appName: "code.exe", seconds: 1200 }), buildWindowInfo({ appName: " ", seconds: 50 })],
      }),
    ]);
    expect(blocks).toHaveLength(1);
    expect(blocks[0]).toMatchObject({
      id: "a1",
      minutes: 30,
      activity: 100,
      idleMinutes: 0,
      keys: 0,
      note: "Fix the login flow",
      apps: [{ name: "code.exe", seconds: 1200 }],
      endReason: null,
      utcOffset: null,
      platform: null,
    });
  });

  it("demo rows: a row's own note wins over its headline, and newer fields come along", () => {
    const withNote = { ...buildActivity({ description: "Tracked session" }), note: null, endReason: "break", utcOffsetMinutes: 345, platform: "macos" };
    const noted = { ...buildActivity({ description: "Headline" }), note: "  Real   note " };
    const [a, b] = toBlocks([withNote, noted] as unknown as Parameters<typeof toBlocks>[0]);
    expect(a).toMatchObject({ note: null, endReason: "break", utcOffset: 345, platform: "macos" });
    expect(b.note).toBe("Real note");
  });

  it("live rows: a missing activity % is 'not measured', and the device, its clock and why the block ended come along", () => {
    const blocks = blocksOf(
      dto("b", "u1", "p1", "2026-09-28T10:00:00", 10, {
        productivity: null,
        idleSeconds: 90,
        endReason: "lock",
        utcOffsetMinutes: 360,
        timeZone: "Asia/Dhaka",
        platform: "Windows",
        deviceId: "dev-1",
        deviceName: "RINA-PC",
        osVersion: "Windows 11 Pro 23H2 (10.0.22631)",
        agentVersion: "0.2.0",
        description: "Code review",
      }),
      dto("a", "u1", "p1", "2026-09-28T09:00:00Z", 10, {
        endReason: "warp-speed",
        utcOffsetMinutes: 9999,
        timeZone: "Mars/Olympus",
        platform: "amiga",
        timelineJson: JSON.stringify([{ Minute: 2, Active: false }, { Minute: 3, Active: true }]),
      }),
    );
    expect(blocks.map((b) => b.id)).toEqual(["a", "b"]); // oldest first
    expect(blocks[1]).toMatchObject({
      start: at("2026-09-28T10:00:00Z"), // a bare timestamp is UTC
      activity: null,
      idleMinutes: 1.5,
      endReason: "lock",
      utcOffset: 360,
      timeZone: "Asia/Dhaka",
      platform: "windows",
      deviceId: "dev-1",
      deviceName: "RINA-PC",
      agentVersion: "0.2.0",
      note: "Code review",
    });
    // Unknown values are dropped, not guessed; idle time comes from the timeline's inactive minute.
    expect(blocks[0]).toMatchObject({ endReason: null, utcOffset: null, timeZone: null, platform: null, idleMinutes: 1, activity: 70 });
  });

  it("live rows: one block read twice (offset paging) counts once; rows without an id or times are dropped", () => {
    const blocks = blocksFromApi([
      dto("a", "u1", "p1", "2026-09-28T09:00:00Z", 10),
      dto("a", "u1", "p1", "2026-09-28T09:00:00Z", 10),
      { ...dto("", "u1", "p1", "2026-09-28T09:00:00Z", 10) },
      { ...dto("c", "u1", "p1", "2026-09-28T09:00:00Z", 10), endedAt: "2026-09-28T08:00:00Z" },
      { ...dto("d", "u1", "p1", "2026-09-28T09:00:00Z", 10), startedAt: "nope" },
      null,
      "row",
    ]);
    expect(blocks.map((b) => b.id)).toEqual(["a"]);
    expect(blocksFromApi(null as unknown as unknown[])).toEqual([]);
  });

  it("live rows: a legacy block's focused window is credited with the whole block", () => {
    const [b] = blocksOf(dto("a", "u1", "p1", "2026-09-28T09:00:00Z", 10, { activeWindowsJson: JSON.stringify([{ AppName: "chrome.exe", WindowTitle: "x" }]) }));
    expect(b.apps).toEqual([{ name: "chrome.exe", seconds: 600 }]);
  });

  it("a period holds the blocks that STARTED inside it", () => {
    const blocks = blocksOf(
      dto("a", "u1", "p1", "2026-09-27T23:50:00Z", 20), // starts before, ends inside
      dto("b", "u1", "p1", "2026-09-28T12:00:00Z", 10),
      dto("c", "u1", "p1", "2026-09-28T23:55:00Z", 20), // starts inside, ends after
    );
    expect(blocksStartingIn(blocks, at("2026-09-28T00:00:00Z"), at("2026-09-29T00:00:00Z")).map((b) => b.id)).toEqual(["b", "c"]);
  });

  it("aggregates blocks by the day they started on in the zone", () => {
    const blocks = blocksOf(dto("a", "u1", "p1", "2026-09-28T20:00:00Z", 120)); // 02:00–04:00 on the 29th in Dhaka
    expect(aggregateBlocks(blocks, UTC).daily.map((d) => [d.day, d.minutes])).toEqual([["2026-09-28", 120]]);
    expect(aggregateBlocks(blocks, zoneCalendar("Asia/Dhaka")).daily.map((d) => [d.day, d.minutes])).toEqual([["2026-09-29", 120]]);
  });

  it("knows when the loaded blocks are missing a period's oldest days", () => {
    const blocks = blocksOf(dto("a", "u1", "p1", "2026-09-28T12:00:00Z", 10), dto("b", "u1", "p1", "2026-09-30T12:00:00Z", 10));
    expect(coversDay(blocks, false, "2026-09-25", UTC)).toBe(true);
    expect(coversDay(blocks, true, "2026-09-29", UTC)).toBe(true);
    expect(coversDay(blocks, true, "2026-09-28", UTC)).toBe(false); // the oldest loaded day may be partial
    expect(coversDay([], true, "2026-09-29", UTC)).toBe(false);
    expect(MAX_BLOCKS).toBe(5000);
  });
});

describe("hours of the zone", () => {
  it("splits time at every local hour and day boundary", () => {
    const pieces = hourPieces(at("2026-09-28T22:40:00Z"), at("2026-09-29T01:10:00Z"), UTC);
    expect(pieces.map((p) => [p.day, p.hour, p.minutes])).toEqual([
      ["2026-09-28", 22, 20],
      ["2026-09-28", 23, 60],
      ["2026-09-29", 0, 60],
      ["2026-09-29", 1, 10],
    ]);
    expect(pieces[0]).toMatchObject({ from: 22 * 60 + 40, to: 23 * 60, weekday: 0 });
    expect(pieces[1].to).toBe(1440);
    expect(hourPieces(at("2026-09-28T09:00:00Z"), at("2026-09-28T09:00:00Z"), UTC)).toEqual([]);
  });

  it("an hour skipped by daylight saving gets no time; an hour lived twice gets both", () => {
    const ny = zoneCalendar("America/New_York");
    // 01:30 EST → 03:30 EDT is one real hour.
    const spring = hourPieces(at("2026-03-08T06:30:00Z"), at("2026-03-08T07:30:00Z"), ny);
    expect(spring.map((p) => [p.hour, p.minutes, p.from, p.to])).toEqual([
      [1, 30, 90, 120],
      [3, 30, 180, 210],
    ]);
    // 01:30 EDT → 01:30 EST is one real hour, all of it in the hour 01.
    const fall = hourPieces(at("2026-11-01T05:30:00Z"), at("2026-11-01T06:30:00Z"), ny);
    expect(fall.map((p) => [p.hour, p.minutes])).toEqual([
      [1, 30],
      [1, 30],
    ]);
  });

  it("reads /reporting/hourly (Sunday-first on the wire) into Monday-first cells", () => {
    const cells = parseHourly([
      { dayOfWeek: 0, hour: 9, trackedMinutes: 30, activeMinutes: 20 }, // Sunday
      { dayOfWeek: 1, hour: 10, trackedMinutes: 60, activeMinutes: 45 }, // Monday
      { dayOfWeek: 7, hour: 1, trackedMinutes: 5 },
      { dayOfWeek: 2, hour: 24, trackedMinutes: 5 },
      { dayOfWeek: 3, hour: 4, trackedMinutes: 0 },
      null,
    ]);
    expect(cells).toEqual([
      { weekday: 6, hour: 9, minutes: 30 },
      { weekday: 0, hour: 10, minutes: 60 },
    ]);
    expect(parseHourly({ not: "an array" })).toEqual([]);
    const grid = rhythmMatrix(cells);
    expect(grid[0][10]).toBe(60);
    expect(grid[6][9]).toBe(30);
    expect(peakHour(hourTotals(cells))).toBe(10);
    expect(busiestWeekday(cells)).toBe(0);
    expect(busiestWeekday([])).toBeNull();
  });

  it("builds the same cells from blocks, split at the zone's hours", () => {
    const blocks = blocksOf(
      dto("a", "u1", "p1", "2026-09-28T09:30:00Z", 90), // Mon
      dto("b", "u2", "p1", "2026-09-29T10:00:00Z", 30), // Tue
    );
    const grid = rhythmMatrix(hourCellsFromBlocks(blocks, UTC));
    expect(grid[0][9]).toBe(30);
    expect(grid[0][10]).toBe(60);
    expect(grid[1][10]).toBe(30);
    expect(grid.flat().reduce((s, v) => s + v, 0)).toBe(120);
    // In Dhaka the same Monday block runs 15:30–17:00.
    const dhaka = rhythmMatrix(hourCellsFromBlocks(blocks, zoneCalendar("Asia/Dhaka")));
    expect([dhaka[0][15], dhaka[0][16]]).toEqual([30, 60]);
  });

  it("profiles the day by hour from blocks, with activity of the measured minutes only", () => {
    const blocks = blocksOf(
      dto("a", "u1", "p1", "2026-09-28T09:30:00Z", 90, { productivity: 90, keyboardHits: 90, mouseClicks: 0 }),
      dto("b", "u2", "p1", "2026-09-29T10:00:00Z", 30, { productivity: 30, keyboardHits: 0, mouseClicks: 0 }),
      dto("c", "u3", "p1", "2026-09-29T11:00:00Z", 30, { productivity: null }),
    );
    const profile = hourProfile(blocks, UTC);
    expect(profile).toHaveLength(24);
    expect(profile[10]).toMatchObject({ minutes: 90, activity: 70, keys: 60 }); // (60×90 + 30×30) / 90
    expect(profile[9]).toMatchObject({ minutes: 30, activity: 90 });
    expect(profile[11]).toMatchObject({ minutes: 30, activity: null });
    expect(peakHour(profile)).toBe(10);
    expect(peakHour(hourProfile([], UTC))).toBeNull();
  });
});

describe("days", () => {
  const blocks = blocksOf(
    dto("a", "u1", "p1", "2026-09-28T09:00:00Z", 120, { productivity: 80 }),
    dto("b", "u2", "p2", "2026-09-28T23:00:00Z", 120, { productivity: 40, idleSeconds: 600 }), // starts on the 28th
  );
  const days = ["2026-09-28", "2026-09-29", "2026-09-30"];

  it("one row per day of the period from the aggregates, a block counting on the day it started", () => {
    const daily = dayStats(aggregateBlocks(blocks, UTC), days);
    expect(daily[0]).toMatchObject({ minutes: 240, activity: 60, sessions: 2, members: 2, weekend: false, idleMinutes: 10 });
    expect(daily[1]).toMatchObject({ minutes: 0, activity: null, sessions: 0, members: 0, idleMinutes: null });
    expect(daily.map((d) => d.label)).toEqual(["Mon 28", "Tue 29", "Wed 30"]);
  });

  it("moving averages seeded with the days before, running totals and the pace line", () => {
    expect(movingAverage([2, 4, 6, 8], 3)).toEqual([2, 3, 4, 6]);
    expect(movingAverage([5], 0)).toEqual([5]);
    // With the two days before, the first day already averages a full window.
    expect(movingAverage([6, 9], 3, [0, 3])).toEqual([3, 6]);
    expect(movingAverage([6], 7, [1, 1, 1, 1, 1, 1, 1, 1])).toEqual([12 / 7]);
    expect(cumulative([1, 2, 3])).toEqual([1, 3, 6]);
    const week = dayStats(aggregateBlocks([], UTC), ["2026-09-25", "2026-09-26", "2026-09-27", "2026-09-28"]); // Fri, Sat, Sun, Mon
    expect(paceTarget(week, 480)).toEqual([480, 480, 480, 960]);
  });
});

describe("members' own clocks", () => {
  const blocks = blocksOf(
    dto("a1", "a", "p1", "2026-09-28T03:00:00Z", 10, { utcOffsetMinutes: 360 }), // Dhaka
    dto("a2", "a", "p1", "2026-09-29T03:00:00Z", 10, { utcOffsetMinutes: 345 }), // then Kathmandu
    dto("b1", "b", "p1", "2026-09-28T13:00:00Z", 10, { timeZone: "America/New_York" }),
    dto("c1", "c", "p1", "2026-09-28T13:00:00Z", 10),
    dto("d1", "d", "p1", "2026-09-28T13:00:00Z", 10),
  );
  const clock = memberClock(blocks, UTC, (id) => (id === "c" ? "Europe/Madrid" : null));

  it("reads each member's clock from the offset their agent reported nearest before the instant", () => {
    expect(clock("a", at("2026-09-28T10:00:00Z"))).toEqual({ minuteOfDay: 16 * 60, offset: 360, estimated: false });
    expect(clock("a", at("2026-09-29T05:00:00Z"))).toMatchObject({ minuteOfDay: 10 * 60 + 45, offset: 345 });
    expect(clock("a", at("2026-09-27T00:00:00Z")).offset).toBe(360); // before any block: the first one's
  });

  it("falls back to the zone their agent reported, then the roster's zone, then the report's (flagged as an estimate)", () => {
    expect(clock("b", at("2026-09-28T13:00:00Z"))).toMatchObject({ minuteOfDay: 9 * 60, offset: -240, estimated: false });
    expect(clock("c", at("2026-09-28T13:00:00Z"))).toMatchObject({ offset: 120, estimated: false });
    expect(clock("d", at("2026-09-28T13:00:00Z"))).toMatchObject({ minuteOfDay: 13 * 60, offset: 0, estimated: true });
  });

  it("labels a member's day on their own clock, naming the offset when it isn't the report's", () => {
    expect(localSpanLabel(clock, UTC, "a", at("2026-09-28T03:00:00Z"), at("2026-09-28T11:30:00Z"))).toBe("09:00–17:30 local (UTC+6)");
    expect(localSpanLabel(clock, zoneCalendar("Asia/Dhaka"), "a", at("2026-09-28T03:00:00Z"), at("2026-09-28T11:30:00Z"))).toBe("09:00–17:30");
    expect(localSpanLabel(clock, UTC, "d", at("2026-09-28T13:00:00Z"), at("2026-09-28T14:00:00Z"))).toBe("13:00–14:00");
    expect(localSpanLabel(clock, UTC, "a", null, null)).toBeNull();
    // A day that runs past local midnight is drawn to midnight.
    expect(localSpanLabel(clock, UTC, "a", at("2026-09-28T16:00:00Z"), at("2026-09-28T19:00:00Z"))).toBe("22:00–24:00 local (UTC+6)");
  });

  it("each day's working window puts every member on their own clock", () => {
    const memberDays = [
      { userId: "a", name: null, day: "2026-09-28", minutes: 400, sessions: 3, idleMinutes: null, firstStart: at("2026-09-28T03:00:00Z"), lastEnd: at("2026-09-28T11:30:00Z") },
      { userId: "b", name: null, day: "2026-09-28", minutes: 420, sessions: 3, idleMinutes: null, firstStart: at("2026-09-28T13:00:00Z"), lastEnd: at("2026-09-28T21:00:00Z") },
      { userId: "b", name: null, day: "2026-09-29", minutes: 60, sessions: 1, idleMinutes: null, firstStart: null, lastEnd: null },
    ];
    const windows = workWindows(memberDays, ["2026-09-28", "2026-09-29"], clock);
    // Dhaka 09:00–17:30 and New York 09:00–17:00: the team's day is 09:00–17:30, not 03:00–21:00 UTC.
    expect(windows[0]).toMatchObject({ start: 540, end: 1050, minutes: 820, estimated: false });
    expect(windows[1]).toMatchObject({ start: null, end: null, minutes: 0 });
    expect(localSpan(memberDays[2], clock)).toBeNull();
    expect(meanOf(windows.map((w) => w.start))).toBe(540);
    expect(meanOf([null])).toBeNull();
  });

  it("fills in day bounds an older server didn't send from the loaded blocks", () => {
    const filled = withBlockBounds(
      [{ userId: "a", name: null, day: "2026-09-28", minutes: 10, sessions: 1, idleMinutes: null, firstStart: null, lastEnd: null }],
      blocks,
      UTC,
    );
    expect(filled[0]).toMatchObject({ firstStart: at("2026-09-28T03:00:00Z"), lastEnd: at("2026-09-28T03:10:00Z") });
  });
});

describe("focus stretches", () => {
  const blocks = blocksOf(
    dto("a", "u1", "p1", "2026-09-28T09:00:00Z", 10),
    dto("b", "u1", "p1", "2026-09-28T09:12:00Z", 10), // 2-minute gap: same sitting
    dto("c", "u1", "p1", "2026-09-28T10:00:00Z", 20), // 38-minute gap: a new one
    dto("d", "u2", "p1", "2026-09-28T09:05:00Z", 300), // another member, 5 hours
  );

  it("merges back-to-back blocks per member, never across members", () => {
    const stretches = focusStretches(blocks);
    expect(stretches.map((s) => [s.userId, s.minutes, s.blocks])).toEqual([
      ["u1", 20, 2],
      ["u2", 300, 1],
      ["u1", 20, 1],
    ]);
    expect(focusStretches(blocks, 0)).toHaveLength(4);
  });

  it("buckets stretch lengths", () => {
    const histogram = stretchHistogram(focusStretches(blocks));
    expect(histogram.map((h) => h.label)).toEqual(STRETCH_BUCKETS.map((b) => b.label));
    expect(histogram[0]).toMatchObject({ count: 2, minutes: 40 });
    expect(histogram[4]).toMatchObject({ count: 1, minutes: 300 });
  });
});

describe("activity distribution", () => {
  it("bins measured minutes by score; 100 lands in the top bin; unmeasured blocks are left out and counted apart", () => {
    const blocks = blocksOf(
      dto("a", "u1", "p1", "2026-09-28T09:00:00Z", 30, { productivity: 100 }),
      dto("b", "u1", "p1", "2026-09-28T10:00:00Z", 10, { productivity: 95 }),
      dto("c", "u1", "p1", "2026-09-28T11:00:00Z", 20, { productivity: 5 }),
      dto("d", "u1", "p1", "2026-09-28T12:00:00Z", 45, { productivity: null }),
    );
    const bins = activityHistogram(blocks);
    expect(bins).toHaveLength(10);
    expect(bins[9]).toMatchObject({ label: "90–100%", minutes: 40, blocks: 2 });
    expect(bins[0]).toMatchObject({ label: "0–9%", minutes: 20, blocks: 1 });
    expect(bins.reduce((s, b) => s + b.minutes, 0)).toBe(60);
    expect(unmeasuredMinutes(blocks)).toBe(45);
  });
});

describe("members", () => {
  const blocks = blocksOf(
    dto("a", "u1", "p1", "2026-09-28T09:00:00Z", 120, { productivity: 80, keyboardHits: 100, mouseClicks: 20 }),
    dto("b", "u1", "p1", "2026-09-29T09:00:00Z", 120, { productivity: 60, keyboardHits: 100, mouseClicks: 20 }),
    dto("c", "u2", "p2", "2026-09-28T09:00:00Z", 60, { productivity: null, keyboardHits: 30, mouseClicks: 0 }),
  );
  const days = ["2026-09-28", "2026-09-29", "2026-09-30"];
  const stats = memberStats(aggregateBlocks(blocks, UTC), blocks, days, memberClock(blocks, UTC));

  it("summarises each member from the aggregates and their blocks, most time first", () => {
    const [u1, u2] = stats;
    expect(u1).toMatchObject({
      userId: "u1",
      minutes: 240,
      activity: 70,
      sessions: 2,
      daysActive: 2,
      daily: [120, 120, 0],
      avgStretch: 120,
      longestStretch: 120,
      inputPerHour: 60,
      consistency: 100,
      typicalStart: 540,
      typicalEnd: 660,
      localEstimated: true,
    });
    expect(u2).toMatchObject({ userId: "u2", minutes: 60, activity: null, daysActive: 1, consistency: null, inputPerHour: 30 });
  });

  it("knows only what the aggregates say about members whose blocks weren't loaded", () => {
    const [u1] = memberStats(aggregateBlocks(blocks, UTC), [], days, memberClock([], UTC));
    expect(u1).toMatchObject({ minutes: 240, avgStretch: null, longestStretch: null, inputPerHour: null, typicalStart: 540 });
  });

  it("scores consistency lower when daily hours swing", () => {
    const swinging = blocksOf(dto("a", "u1", "p1", "2026-09-28T09:00:00Z", 60), dto("b", "u1", "p1", "2026-09-29T09:00:00Z", 300));
    expect(memberStats(aggregateBlocks(swinging, UTC), swinging, days, memberClock(swinging, UTC))[0].consistency).toBe(33);
  });

  it("puts members on comparable 0–100 radar scales, an unmeasured figure at 0", () => {
    const rows = radarRows(stats, 2);
    expect(rows.find((r) => r.dimension === "Hours")).toMatchObject({ u1: 100, u2: 25 });
    expect(rows.find((r) => r.dimension === "Activity")).toMatchObject({ u1: 70, u2: 0 });
    expect(rows.find((r) => r.dimension === "Days active")).toMatchObject({ u1: 100, u2: 50 });
    expect(rows.find((r) => r.dimension === "Consistency")).toMatchObject({ u1: 100, u2: 0 });
    expect(rows.map((r) => r.dimension)).toEqual(["Hours", "Activity", "Days active", "Focus", "Input", "Consistency"]);
  });
});

describe("projects and apps", () => {
  const blocks = blocksOf(
    dto("a", "u1", "p1", "2026-09-28T09:00:00Z", 120, {
      activeWindowsJson: JSON.stringify([
        { AppName: "code.exe", Seconds: 5400 },
        { AppName: "Chrome", Seconds: 1800 },
      ]),
    }),
    dto("b", "u2", "p2", "2026-09-28T23:30:00Z", 60, { activeWindowsJson: JSON.stringify([{ AppName: "CODE.EXE", Seconds: 3600 }]) }),
    dto("c", "u3", "p3", "2026-09-29T09:00:00Z", 30, { activeWindowsJson: "[]" }),
  );

  it("splits each day's hours by the top projects, the rest as Other, each block on its start day", () => {
    const rows = projectDays(blocks, ["2026-09-28", "2026-09-29"], ["p1"], UTC);
    expect(rows[0]).toMatchObject({ day: "2026-09-28", label: "Mon 28", p1: 2, [OTHER]: 1 });
    expect(rows[1]).toMatchObject({ p1: 0, [OTHER]: 0.5 });
  });

  it("builds the member → project flow from member × project minutes, grouping the smallest as Others", () => {
    const pairs = memberProjectsFromBlocks(blocks);
    const flow = memberProjectFlow(pairs, (id) => id.toUpperCase(), (id) => `Project ${id}`);
    expect(flow.nodes.map((n) => n.name)).toEqual(["U1", "U2", "U3", "Project p1", "Project p2", "Project p3"]);
    expect(flow.links).toEqual([
      { source: 0, target: 3, value: 2 },
      { source: 1, target: 4, value: 1 },
      { source: 2, target: 5, value: 0.5 },
    ]);
    const grouped = memberProjectFlow(pairs, (id) => id, (id) => id, 2, 2);
    expect(grouped.nodes.map((n) => n.name)).toEqual(["u1", "Other members", "p1", "Other projects"]);
    expect(grouped.links).toEqual([
      { source: 0, target: 2, value: 2 },
      { source: 1, target: 3, value: 1.5 },
    ]);
    expect(memberProjectFlow([{ userId: "", projectId: "p", minutes: 5 }], (id) => id, (id) => id)).toEqual({ nodes: [], links: [] });
  });

  it("ranks apps by focus time under one canonical name and colour, however each agent spelt them", () => {
    const usage = appUsageFromBlocks(blocks);
    expect(usage).toEqual([
      { name: "Visual Studio Code", minutes: 150 },
      { name: "Google Chrome", minutes: 30 },
    ]);
    expect(appStats(usage, 8)).toEqual([
      { app: "Visual Studio Code", minutes: 150, share: 83, color: APP_PALETTE[0] },
      { app: "Google Chrome", minutes: 30, share: 17, color: APP_PALETTE[6] },
    ]);
    const merged = appStats(
      [
        { name: "chrome.exe", minutes: 30 },
        { name: "Google Chrome", minutes: 30 },
        { name: "Code.exe", minutes: 90 },
        { name: "", minutes: 50 },
      ],
      1,
    );
    expect(merged).toEqual([
      { app: "Visual Studio Code", minutes: 90, share: 60, color: APP_PALETTE[0] },
      { app: "Other apps", minutes: 60, share: 40, color: OTHER_APP_COLOR },
    ]);
  });
});

describe("day timeline", () => {
  const blocks = blocksOf(
    dto("a1", "a", "p1", "2026-09-28T09:00:00Z", 10, { endReason: "interval" }),
    dto("a2", "a", "p1", "2026-09-28T09:10:00Z", 10, { endReason: "break" }),
    dto("a3", "a", "p2", "2026-09-28T09:35:00Z", 10, { endReason: "idle" }),
    dto("a4", "a", "p2", "2026-09-28T10:30:00Z", 10, { endReason: "lock" }),
    dto("a5", "a", "p2", "2026-09-28T11:00:00Z", 10),
    dto("a6", "a", "p2", "2026-09-28T11:10:30Z", 10, { endReason: "pause" }), // 30 s after the last: a seam, not a gap
    dto("b1", "b", "p1", "2026-09-28T08:30:00Z", 10, { endReason: "sleep" }),
    dto("b2", "b", "p1", "2026-09-28T13:00:00Z", 10),
    dto("c1", "c", "p1", "2026-09-29T09:00:00Z", 10), // another day
  );
  const day = dayTimeline(blocks, "2026-09-28", UTC);

  it("lays out each member's blocks, earliest starter first, on whole hours of the zone", () => {
    expect(day.lanes.map((l) => l.userId)).toEqual(["b", "a"]);
    expect(day.from).toBe(at("2026-09-28T08:00:00Z"));
    expect(day.to).toBe(at("2026-09-28T14:00:00Z"));
    expect(day.lanes[1].segments).toHaveLength(6);
    expect(day.lanes[1].minutes).toBe(60);
    expect(day.lanes[1]).toMatchObject({ first: at("2026-09-28T09:00:00Z"), last: at("2026-09-28T11:20:30Z") });
  });

  it("labels each gap with why the block before it ended", () => {
    const gaps = day.lanes[1].gaps.map((g) => [new Date(g.start).toISOString().slice(11, 16), new Date(g.end).toISOString().slice(11, 16), g.reason]);
    expect(gaps).toEqual([
      ["09:20", "09:35", "break"],
      ["09:45", "10:30", "idle"],
      ["10:40", "11:00", "lock"],
    ]);
    expect(day.lanes[0].gaps.map((g) => g.reason)).toEqual(["sleep"]);
  });

  it("is empty for a day without blocks", () => {
    const none = dayTimeline(blocks, "2026-10-01", UTC);
    expect(none.lanes).toEqual([]);
    expect(none.to - none.from).toBe(86_400_000);
  });

  it("names every kind of gap", () => {
    expect(gapKind("break")).toMatchObject({ kind: "break", label: "Break" });
    expect(gapKind("idle")).toMatchObject({ kind: "idle", label: "Idle" });
    expect(gapKind("lock")).toMatchObject({ kind: "lock", label: "Locked", description: "Screen locked" });
    expect(gapKind("sleep")).toMatchObject({ kind: "sleep", label: "Asleep" });
    expect(gapKind("pause")).toMatchObject({ kind: "off", label: "Paused" });
    expect(gapKind("shutdown")).toMatchObject({ kind: "off", label: "Off", description: "Shut down" });
    expect(gapKind("interval")).toMatchObject({ kind: "untracked", label: "Not tracked" });
    expect(gapKind(null)).toMatchObject({ kind: "untracked" });
  });
});

describe("devices", () => {
  it("reads /reporting/devices and counts devices and time per platform", () => {
    const rows = parseDevices([
      { userId: "a", userName: "ana", deviceId: "d1", deviceName: "ANA-PC", platform: "windows", osVersion: "Windows 11", agentVersion: "0.2.0", lastActivityAt: "2026-09-28T10:00:00", trackedMinutes: 200 },
      { userId: "b", deviceId: "d2", platform: "Windows", agentVersion: "0.10.1", lastActivityAt: "2026-09-28T11:00:00Z", trackedMinutes: 100 },
      { userId: "b", deviceId: "d3", platform: "macos", agentVersion: "0.2.0", trackedMinutes: 100 },
      { userId: "c", trackedMinutes: 50 },
      { userId: "d", trackedMinutes: 0 },
      { trackedMinutes: 10 },
    ]);
    expect(rows).toHaveLength(4);
    expect(rows[0]).toMatchObject({ platform: "windows", deviceName: "ANA-PC", lastActivityAt: at("2026-09-28T10:00:00Z") });
    expect(rows[1].platform).toBe("windows");
    expect(platformMix(rows)).toEqual([
      { platform: "windows", label: "Windows", devices: 2, members: 2, minutes: 300, share: 67 },
      { platform: "macos", label: "macOS", devices: 1, members: 1, minutes: 100, share: 22 },
      { platform: null, label: "Not reported", devices: 1, members: 1, minutes: 50, share: 11 },
    ]);
    expect(agentVersions(rows)).toEqual([
      { version: "0.10.1", devices: 1 },
      { version: "0.2.0", devices: 2 },
      { version: null, devices: 1 },
    ]);
    expect(parseDevices("nope")).toEqual([]);
    expect(platformMix([])).toEqual([]);
  });

  it("platformMix orders Windows, macOS, Linux, and Not reported, computing durations, devices, members, and apportioned shares", () => {
    const rows = parseDevices([
      { userId: "u-win", deviceId: "d-win-1", platform: "windows", trackedMinutes: 300 },
      { userId: "u-mac", deviceId: "d-mac-1", platform: "macos", trackedMinutes: 150 },
      { userId: "u-lin", deviceId: "d-lin-1", platform: "linux", trackedMinutes: 100 },
      { userId: "u-lin", deviceId: "d-lin-2", platform: "Linux", trackedMinutes: 50 },
      { userId: "u-unk", deviceId: "d-unk-1", trackedMinutes: 50 },
    ]);

    expect(platformMix(rows)).toEqual([
      { platform: "windows", label: "Windows", devices: 1, members: 1, minutes: 300, share: 46 },
      { platform: "macos", label: "macOS", devices: 1, members: 1, minutes: 150, share: 23 },
      { platform: "linux", label: "Linux", devices: 2, members: 1, minutes: 150, share: 23 },
      { platform: null, label: "Not reported", devices: 1, members: 1, minutes: 50, share: 8 },
    ]);
  });

  it("gathers the same rows from blocks that carry device details", () => {
    const blocks = blocksOf(
      dto("a", "u1", "p1", "2026-09-28T09:00:00Z", 30, { platform: "windows", deviceId: "d1", agentVersion: "0.1.0" }),
      dto("b", "u1", "p1", "2026-09-28T10:00:00Z", 30, { platform: "windows", deviceId: "d1", agentVersion: "0.2.0" }),
      dto("c", "u2", "p1", "2026-09-28T10:00:00Z", 30),
    );
    expect(devicesFromBlocks(blocks)).toEqual([
      { userId: "u1", userName: null, deviceId: "d1", deviceName: null, platform: "windows", osVersion: null, agentVersion: "0.2.0", lastActivityAt: at("2026-09-28T10:30:00Z"), minutes: 60 },
    ]);
    expect(devicesFromBlocks(toBlocks([buildActivity()]))).toEqual([]);
  });
});

describe("summary", () => {
  it("rolls the period up: totals from the aggregates, stretches from the blocks, rhythm from the hour cells", () => {
    const blocks = blocksOf(
      dto("a", "u1", "p1", "2026-09-28T09:00:00Z", 120, { productivity: 80, keyboardHits: 100, mouseClicks: 20, idleSeconds: 600 }),
      dto("b", "u2", "p1", "2026-09-29T14:00:00Z", 60, { productivity: 50, keyboardHits: 60, mouseClicks: 0 }),
    );
    const days = ["2026-09-28", "2026-09-29", "2026-09-30"];
    const agg = aggregateBlocks(blocks, UTC);
    const s = summarize(agg, days, blocks, hourCellsFromBlocks(blocks, UTC));
    expect(s).toEqual({
      totalMinutes: 180,
      activity: 70,
      idleMinutes: 10,
      sessions: 2,
      members: 2,
      daysWorked: 2,
      avgStretch: 90,
      longestStretch: 120,
      inputPerHour: 60,
      peakHour: 9,
      busiestWeekday: 0,
    });
    const empty = summarize(aggregateBlocks([], UTC), days, [], []);
    expect(empty).toMatchObject({ totalMinutes: 0, activity: null, idleMinutes: null, peakHour: null, busiestWeekday: null, avgStretch: null, inputPerHour: null });
  });
});

describe("layouts", () => {
  it("squarifies tiles that fill the area exactly, largest first", () => {
    const tiles = squarify([{ v: 6 }, { v: 6 }, { v: 4 }, { v: 3 }, { v: 2 }, { v: 2 }, { v: 1 }, { v: 0 }], (d) => d.v, 600, 400);
    expect(tiles).toHaveLength(7); // the zero gets no tile
    expect(tiles[0].item.v).toBe(6);
    const area = tiles.reduce((s, t) => s + t.w * t.h, 0);
    expect(area).toBeCloseTo(600 * 400, 6);
    for (const t of tiles) {
      expect(t.x).toBeGreaterThanOrEqual(-1e-9);
      expect(t.y).toBeGreaterThanOrEqual(-1e-9);
      expect(t.x + t.w).toBeLessThanOrEqual(600 + 1e-9);
      expect(t.y + t.h).toBeLessThanOrEqual(400 + 1e-9);
      // Each tile's area is its share of the whole.
      expect(t.w * t.h).toBeCloseTo((t.item.v / 24) * 600 * 400, 6);
    }
    expect(squarify([{ v: 0 }], (d) => d.v, 100, 100)).toEqual([]);
    expect(squarify([{ v: 5 }], (d) => d.v, 0, 100)).toEqual([]);
  });

  it("lays the flow out in two columns, bands stacked inside their nodes", () => {
    const flow = {
      nodes: [
        { name: "A", kind: "member" as const, id: "a" },
        { name: "B", kind: "member" as const, id: "b" },
        { name: "P", kind: "project" as const, id: "p" },
        { name: "Q", kind: "project" as const, id: "q" },
      ],
      links: [
        { source: 0, target: 2, value: 3 },
        { source: 0, target: 3, value: 1 },
        { source: 1, target: 3, value: 2 },
      ],
    };
    const { nodes, links } = layoutFlow(flow, 500, 230, { nodeWidth: 10, gap: 20, left: 100, right: 100 });
    // 6 hours over 230 − 20 px of nodes: 35 px per hour.
    expect(nodes[0]).toMatchObject({ x: 100, y: 0, height: 140, value: 4 });
    expect(nodes[1]).toMatchObject({ x: 100, y: 160, height: 70, value: 2 });
    expect(nodes[2]).toMatchObject({ x: 390, height: 105, value: 3 });
    expect(nodes[3]).toMatchObject({ x: 390, height: 105, value: 3 });
    // A's bands: to P first (P sits higher), then to Q, stacked from A's top.
    expect(links[0]).toMatchObject({ sy: 52.5, thickness: 105, x0: 110, x1: 390 });
    expect(links[1]).toMatchObject({ sy: 122.5, thickness: 35 });
    // Q receives A's band above B's (A sits higher).
    const q = nodes[3];
    expect(links[1].ty).toBeCloseTo(q.y + 17.5, 6);
    expect(links[2].ty).toBeCloseTo(q.y + 35 + 35, 6);
  });

  it("places labels that never overlap each other or another mark, and leaves out those with no room", () => {
    const area = { x0: 0, y0: 0, x1: 300, y1: 200 };
    const points: LabelPoint[] = [
      { id: "p1", x: 100, y: 100, r: 10, width: 40, height: 14 },
      { id: "p2", x: 100, y: 125, r: 10, width: 40, height: 14 }, // just below p1: its label can't go above
      { id: "p3", x: 106, y: 112, r: 8, width: 40, height: 14 }, // between them: only the right is free
      { id: "corner", x: 4, y: 4, r: 6, width: 60, height: 14 }, // in the corner: only the lower-right diagonal fits
    ];
    const placed = placeLabels(points, area);
    expect(placed.get("p1")).toEqual({ x: 100, y: 80 }); // above
    expect(placed.get("p2")).toEqual({ x: 100, y: 145 }); // below
    expect(placed.get("p3")).toEqual({ x: 137, y: 112 }); // right
    expect(placed.get("corner")?.x).toBeGreaterThan(30);
    // A label wider than the area has nowhere to go.
    expect(placeLabels([{ id: "wide", x: 50, y: 20, r: 6, width: 200, height: 14 }], { x0: 0, y0: 0, x1: 100, y1: 40 }).size).toBe(0);
    const boxes = [...placed.entries()].map(([id, c]) => {
      const p = points.find((x) => x.id === id)!;
      return { id, x0: c.x - p.width / 2, y0: c.y - p.height / 2, x1: c.x + p.width / 2, y1: c.y + p.height / 2 };
    });
    for (const a of boxes) {
      expect(a.x0).toBeGreaterThanOrEqual(area.x0);
      expect(a.y1).toBeLessThanOrEqual(area.y1);
      for (const b of boxes) {
        if (a === b) continue;
        expect(a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1).toBe(false);
      }
      for (const p of points) {
        if (p.id === a.id) continue;
        const nx = Math.max(a.x0, Math.min(p.x, a.x1));
        const ny = Math.max(a.y0, Math.min(p.y, a.y1));
        expect((p.x - nx) ** 2 + (p.y - ny) ** 2).toBeGreaterThanOrEqual(p.r ** 2);
      }
    }
  });

  it("keeps a crowd of bubbles' labels apart", () => {
    const crowd: LabelPoint[] = Array.from({ length: 12 }, (_, i) => ({ id: `m${i}`, x: 150 + (i % 4) * 12, y: 100 + Math.floor(i / 4) * 12, r: 7, width: 36, height: 14 }));
    const placed = placeLabels(crowd, { x0: 0, y0: 0, x1: 320, y1: 220 });
    const boxes = [...placed.values()].map((c) => ({ x0: c.x - 18, y0: c.y - 7, x1: c.x + 18, y1: c.y + 7 }));
    for (let i = 0; i < boxes.length; i++) {
      for (let j = i + 1; j < boxes.length; j++) {
        const [a, b] = [boxes[i], boxes[j]];
        expect(a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1).toBe(false);
      }
    }
    expect(placed.size).toBeGreaterThan(0);
    expect(placed.size).toBeLessThan(crowd.length); // some had to give way
  });
});

it("blocks are typed with the end reason and device fields", () => {
  const sample: Pick<Block, "endReason" | "platform" | "utcOffset"> = { endReason: "sleep", platform: "macos", utcOffset: -420 };
  expect(sample.endReason).toBe("sleep");
});
