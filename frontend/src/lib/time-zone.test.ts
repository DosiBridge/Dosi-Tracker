// Reporting-zone arithmetic: a calendar day of a named zone must map to the
// exact UTC instants the backend buckets by — through DST changes (New York,
// London), half- and quarter-hour offsets (Kathmandu +05:45) and zones whose
// clocks jump at midnight — using nothing but Intl.
import { afterEach, describe, expect, it, vi } from "vitest";
import fc from "fast-check";
import {
  addZonedDays,
  browserTimeZone,
  countDays,
  effectiveTimeZone,
  formatDayKey,
  formatDayRange,
  formatInZone,
  formatUtcOffset,
  isDayKey,
  isValidTimeZone,
  normalizeTimeZone,
  startOfZonedDay,
  supportedTimeZones,
  timeZoneCity,
  timeZoneLabel,
  utcOffsetMinutes,
  validateDayRange,
  zonedDayKey,
  zonedDayRange,
  zonedHour,
  zonedParts,
  zonedPeriod,
} from "./time-zone";

const at = (iso: string) => new Date(iso);
const HOUR = 3_600_000;

afterEach(() => {
  vi.restoreAllMocks();
});

describe("zone ids", () => {
  it("accepts IANA zone ids the platform knows", () => {
    for (const zone of ["UTC", "Asia/Dhaka", "Asia/Kathmandu", "America/New_York", "America/Argentina/Buenos_Aires", "Etc/GMT+5"]) {
      expect(isValidTimeZone(zone)).toBe(true);
    }
  });

  it("rejects offsets, unknown ids, padding and non-strings", () => {
    for (const value of ["+05:30", "UTC+6", "Mars/Phobos", "", " Asia/Dhaka", "Asia/Dhaka ", `Asia/${"x".repeat(70)}`, 42, null, undefined]) {
      expect(isValidTimeZone(value)).toBe(false);
    }
  });

  it("normalizes a zone's letter case and surrounding space, and refuses anything invalid", () => {
    expect(normalizeTimeZone("asia/dhaka")).toBe("Asia/Dhaka");
    expect(normalizeTimeZone("  Europe/London ")).toBe("Europe/London");
    expect(normalizeTimeZone("Mars/Phobos")).toBeNull();
    expect(normalizeTimeZone(undefined)).toBeNull();
  });

  it("speaks the current IANA name even where the platform still uses the old one", () => {
    // ICU calls these by their pre-rename names; servers with current tzdata may only know the new ones.
    expect(normalizeTimeZone("Asia/Calcutta")).toBe("Asia/Kolkata");
    expect(normalizeTimeZone("asia/kathmandu")).toBe("Asia/Kathmandu");
    expect(normalizeTimeZone("Europe/Kiev")).toBe("Europe/Kyiv");
    expect(normalizeTimeZone("US/Eastern")).toBe("America/New_York");
  });

  it("reads the browser's zone, falling back to UTC when the platform can't say", () => {
    const real = Intl.DateTimeFormat.prototype.resolvedOptions;
    const spy = vi.spyOn(Intl.DateTimeFormat.prototype, "resolvedOptions");
    spy.mockImplementation(function (this: Intl.DateTimeFormat) {
      return { ...real.call(this), timeZone: "Asia/Kathmandu" };
    });
    expect(browserTimeZone()).toBe("Asia/Kathmandu");
    spy.mockImplementation(function (this: Intl.DateTimeFormat) {
      return { ...real.call(this), timeZone: "+05:45" };
    });
    expect(browserTimeZone()).toBe("UTC");
    spy.mockImplementation(() => {
      throw new Error("no Intl");
    });
    expect(browserTimeZone()).toBe("UTC");
  });

  it("lists the platform's zones by their current names, sorted, always with UTC", () => {
    const zones = supportedTimeZones();
    expect(zones).toContain("UTC");
    expect(zones).toContain("Asia/Kathmandu");
    expect(zones).toContain("Asia/Kolkata");
    expect(zones).not.toContain("Asia/Calcutta");
    expect(zones).toEqual([...zones].sort());
    expect(new Set(zones).size).toBe(zones.length);
  });

  it("still offers UTC where the platform can't list its zones", () => {
    const intl = Intl as { supportedValuesOf?: unknown };
    const original = intl.supportedValuesOf;
    intl.supportedValuesOf = undefined;
    try {
      expect(supportedTimeZones()).toEqual(["UTC"]);
    } finally {
      intl.supportedValuesOf = original;
    }
  });
});

describe("effectiveTimeZone — the reporting zone (contract §7)", () => {
  it("prefers the viewer's own zone, then the workspace default, then the browser's", () => {
    expect(effectiveTimeZone({ user: "Asia/Dhaka", workspace: "Europe/London", browser: "America/New_York" })).toEqual({
      timeZone: "Asia/Dhaka",
      source: "user",
    });
    expect(effectiveTimeZone({ user: null, workspace: "Europe/London", browser: "America/New_York" })).toEqual({
      timeZone: "Europe/London",
      source: "workspace",
    });
    expect(effectiveTimeZone({ workspace: "", browser: "America/New_York" })).toEqual({
      timeZone: "America/New_York",
      source: "browser",
    });
  });

  it("skips unusable candidates and ends at UTC", () => {
    expect(effectiveTimeZone({ user: "Mars/Phobos", workspace: "asia/kathmandu" })).toEqual({
      timeZone: "Asia/Kathmandu",
      source: "workspace",
    });
    expect(effectiveTimeZone({})).toEqual({ timeZone: "UTC", source: "browser" });
  });
});

describe("zoned clock readings", () => {
  it("reads Kathmandu's +05:45 to the minute", () => {
    expect(zonedParts(at("2026-07-14T18:14:59Z"), "Asia/Kathmandu")).toEqual({
      year: 2026,
      month: 7,
      day: 14,
      hour: 23,
      minute: 59,
      second: 59,
      weekday: 2,
    });
    expect(zonedDayKey(at("2026-07-14T18:15:00Z"), "Asia/Kathmandu")).toBe("2026-07-15");
    expect(zonedHour(at("2026-07-14T18:15:00Z"), "Asia/Kathmandu")).toBe(0);
    expect(utcOffsetMinutes(at("2026-07-14T12:00:00Z"), "Asia/Kathmandu")).toBe(345);
  });

  it("follows New York and London through their DST changes", () => {
    expect(utcOffsetMinutes(at("2026-01-15T12:00:00Z"), "America/New_York")).toBe(-300);
    expect(utcOffsetMinutes(at("2026-07-15T12:00:00Z"), "America/New_York")).toBe(-240);
    expect(utcOffsetMinutes(at("2026-01-15T12:00:00Z"), "Europe/London")).toBe(0);
    expect(utcOffsetMinutes(at("2026-07-15T12:00:00Z"), "Europe/London")).toBe(60);
    // 2:00 doesn't exist in New York on 8 March: 1:59 is followed by 3:00.
    expect(zonedHour(at("2026-03-08T06:59:00Z"), "America/New_York")).toBe(1);
    expect(zonedHour(at("2026-03-08T07:00:00Z"), "America/New_York")).toBe(3);
    // 1:00–1:59 happens twice in New York on 1 November.
    expect(zonedHour(at("2026-11-01T05:30:00Z"), "America/New_York")).toBe(1);
    expect(zonedHour(at("2026-11-01T06:30:00Z"), "America/New_York")).toBe(1);
  });

  it("reads an unknown zone as UTC, as the server does", () => {
    expect(zonedDayKey(at("2026-07-14T23:30:00Z"), "Mars/Phobos")).toBe("2026-07-14");
    expect(utcOffsetMinutes(at("2026-07-14T23:30:00Z"), "Mars/Phobos")).toBe(0);
  });

  it("accepts epoch milliseconds and rejects an invalid date", () => {
    expect(zonedDayKey(Date.UTC(2026, 6, 14, 20), "Asia/Dhaka")).toBe("2026-07-15");
    expect(() => zonedParts(new Date("not a date"), "UTC")).toThrow(RangeError);
    expect(() => zonedDayKey(Number.NaN, "UTC")).toThrow(RangeError);
  });
});

describe("startOfZonedDay — a calendar day's first instant, as UTC", () => {
  it("is local midnight in Kathmandu (18:15 UTC the day before)", () => {
    expect(startOfZonedDay("2026-07-15", "Asia/Kathmandu").toISOString()).toBe("2026-07-14T18:15:00.000Z");
  });

  it("gives New York a 23-hour day when clocks spring forward and a 25-hour day when they fall back", () => {
    const march8 = startOfZonedDay("2026-03-08", "America/New_York");
    const march9 = startOfZonedDay("2026-03-09", "America/New_York");
    expect(march8.toISOString()).toBe("2026-03-08T05:00:00.000Z");
    expect(march9.toISOString()).toBe("2026-03-09T04:00:00.000Z");
    expect(march9.getTime() - march8.getTime()).toBe(23 * HOUR);

    const nov1 = startOfZonedDay("2026-11-01", "America/New_York");
    const nov2 = startOfZonedDay("2026-11-02", "America/New_York");
    expect(nov1.toISOString()).toBe("2026-11-01T04:00:00.000Z");
    expect(nov2.toISOString()).toBe("2026-11-02T05:00:00.000Z");
    expect(nov2.getTime() - nov1.getTime()).toBe(25 * HOUR);
  });

  it("gives London its short spring day and long autumn day", () => {
    expect(startOfZonedDay("2026-03-29", "Europe/London").toISOString()).toBe("2026-03-29T00:00:00.000Z");
    expect(startOfZonedDay("2026-03-30", "Europe/London").toISOString()).toBe("2026-03-29T23:00:00.000Z");
    expect(startOfZonedDay("2026-10-25", "Europe/London").toISOString()).toBe("2026-10-24T23:00:00.000Z");
    expect(startOfZonedDay("2026-10-26", "Europe/London").toISOString()).toBe("2026-10-26T00:00:00.000Z");
  });

  it("starts a day at its first existing minute when the clocks jump at midnight (Santiago)", () => {
    // 6 Sep 2026 has no 00:00 in Santiago: 23:59:59 on the 5th is followed by 01:00 on the 6th.
    const start = startOfZonedDay("2026-09-06", "America/Santiago");
    expect(start.toISOString()).toBe("2026-09-06T04:00:00.000Z");
    expect(zonedDayKey(start.getTime() - 1000, "America/Santiago")).toBe("2026-09-05");
  });

  it("is plain UTC midnight in UTC, and in an unknown zone", () => {
    expect(startOfZonedDay("2026-07-14", "UTC").toISOString()).toBe("2026-07-14T00:00:00.000Z");
    expect(startOfZonedDay("2026-07-14", "Mars/Phobos").toISOString()).toBe("2026-07-14T00:00:00.000Z");
  });

  it("rejects something that isn't a calendar day", () => {
    expect(() => startOfZonedDay("2026-02-30", "UTC")).toThrow(RangeError);
  });

  it("brackets every instant: a day's start ≤ the instant < the next day's start", () => {
    const zones = fc.constantFrom("UTC", "Asia/Kathmandu", "Asia/Dhaka", "America/New_York", "Europe/London", "America/Santiago", "Pacific/Kiritimati", "Pacific/Pago_Pago");
    const instant = fc.integer({ min: Date.UTC(2024, 0, 1), max: Date.UTC(2028, 11, 31) });
    fc.assert(
      fc.property(instant, zones, (t, zone) => {
        const day = zonedDayKey(t, zone);
        const start = startOfZonedDay(day, zone).getTime();
        const next = startOfZonedDay(addZonedDays(day, 1), zone).getTime();
        expect(start).toBeLessThanOrEqual(t);
        expect(t).toBeLessThan(next);
        expect(zonedDayKey(start, zone)).toBe(day);
      }),
      { numRuns: 300 },
    );
  });
});

describe("day keys", () => {
  it("recognises real calendar dates only", () => {
    expect(isDayKey("2024-02-29")).toBe(true);
    for (const value of ["2026-02-29", "2026-02-30", "2026-13-01", "2026-7-1", "20260701", "", null, 20260701]) {
      expect(isDayKey(value)).toBe(false);
    }
  });

  it("adds calendar days across months, years and leap days", () => {
    expect(addZonedDays("2026-12-31", 1)).toBe("2027-01-01");
    expect(addZonedDays("2024-02-28", 1)).toBe("2024-02-29");
    expect(addZonedDays("2026-03-01", -1)).toBe("2026-02-28");
    expect(addZonedDays("2026-07-15", 0)).toBe("2026-07-15");
    expect(addZonedDays("2026-07-15", -366)).toBe("2025-07-14");
    expect(() => addZonedDays("15/07/2026", 1)).toThrow(RangeError);
  });

  it("counts the days a range covers, both ends included", () => {
    expect(countDays("2026-07-01", "2026-07-15")).toBe(15);
    expect(countDays("2026-07-15", "2026-07-15")).toBe(1);
    expect(countDays("2026-07-15", "2026-07-01")).toBe(0);
    expect(countDays("2025-12-28", "2026-01-03")).toBe(7);
  });

  it("resolves a range of days to every day and its exact UTC bounds (New York over the March change)", () => {
    const range = zonedDayRange("2026-03-07", "2026-03-09", "America/New_York");
    expect(range.days).toEqual(["2026-03-07", "2026-03-08", "2026-03-09"]);
    expect(range.from.toISOString()).toBe("2026-03-07T05:00:00.000Z");
    expect(range.to.toISOString()).toBe("2026-03-10T04:00:00.000Z");
  });

  it("an inverted range is empty", () => {
    const range = zonedDayRange("2026-07-15", "2026-07-01", "UTC");
    expect(range.days).toEqual([]);
    expect(range.to.getTime()).toBe(range.from.getTime());
  });
});

describe("zonedPeriod — a preset's whole days in the reporting zone", () => {
  // Wed 15 Jul 2026, 20:00 UTC: already Thursday the 16th in Dhaka, still Wednesday in New York.
  const now = Date.UTC(2026, 6, 15, 20, 0);

  it("reads 'today' in the reporting zone, not the browser's", () => {
    expect(zonedPeriod("7d", now, "Asia/Dhaka")).toMatchObject({ fromKey: "2026-07-10", toKey: "2026-07-16" });
    expect(zonedPeriod("7d", now, "America/New_York")).toMatchObject({ fromKey: "2026-07-09", toKey: "2026-07-15" });
    const dhaka = zonedPeriod("7d", now, "Asia/Dhaka");
    expect(dhaka.days).toHaveLength(7);
    expect(dhaka.from.toISOString()).toBe("2026-07-09T18:00:00.000Z");
    expect(dhaka.to.toISOString()).toBe("2026-07-16T18:00:00.000Z");
  });

  it("runs the current week Monday → today, and earlier weeks Monday → Sunday", () => {
    expect(zonedPeriod("week", now, "Asia/Dhaka")).toMatchObject({ fromKey: "2026-07-13", toKey: "2026-07-16" });
    expect(zonedPeriod("week", now, "Asia/Dhaka", -1)).toMatchObject({ fromKey: "2026-07-06", toKey: "2026-07-12" });
  });

  it("runs this month to date, then whole calendar months (across New Year and leap years)", () => {
    expect(zonedPeriod("month", now, "UTC")).toMatchObject({ fromKey: "2026-07-01", toKey: "2026-07-15" });
    expect(zonedPeriod("month", now, "UTC", -1)).toMatchObject({ fromKey: "2026-06-01", toKey: "2026-06-30" });
    expect(zonedPeriod("month", now, "UTC", -7)).toMatchObject({ fromKey: "2025-12-01", toKey: "2025-12-31" });
    expect(zonedPeriod("month", Date.UTC(2024, 2, 10, 12), "UTC", -1)).toMatchObject({ fromKey: "2024-02-01", toKey: "2024-02-29" });
  });

  it("steps rolling 30-day windows back without overlap", () => {
    expect(zonedPeriod("30d", now, "UTC")).toMatchObject({ fromKey: "2026-06-16", toKey: "2026-07-15" });
    expect(zonedPeriod("30d", now, "UTC", -1)).toMatchObject({ fromKey: "2026-05-17", toKey: "2026-06-15" });
  });

  it("treats a future offset as the current period", () => {
    expect(zonedPeriod("7d", now, "UTC", 3)).toEqual(zonedPeriod("7d", now, "UTC"));
    expect(zonedPeriod("week", now, "UTC", Number.NaN)).toEqual(zonedPeriod("week", now, "UTC"));
  });

  it("bounds a week over New York's spring change by exact local midnights (a 167-hour week)", () => {
    const week = zonedPeriod("week", Date.UTC(2026, 2, 12, 15), "America/New_York", -1);
    expect(week).toMatchObject({ fromKey: "2026-03-02", toKey: "2026-03-08" });
    expect(week.from.toISOString()).toBe("2026-03-02T05:00:00.000Z");
    expect(week.to.toISOString()).toBe("2026-03-09T04:00:00.000Z");
    expect(week.to.getTime() - week.from.getTime()).toBe(167 * HOUR);
  });
});

describe("validateDayRange", () => {
  const rules = { today: "2026-07-20", maxDays: 366 };

  it("accepts a range in order that ends by today, reporting its length", () => {
    expect(validateDayRange({ from: "2026-07-01", to: "2026-07-15" }, rules)).toEqual({ ok: true, days: 15 });
    expect(validateDayRange({ from: "2026-07-20", to: "2026-07-20" }, rules)).toEqual({ ok: true, days: 1 });
  });

  it("names the field to fix and why", () => {
    expect(validateDayRange({ from: "", to: "2026-07-15" }, rules)).toEqual({ ok: false, field: "from", message: "Enter a start date." });
    expect(validateDayRange({ from: "2026-07-01", to: "" }, rules)).toEqual({ ok: false, field: "to", message: "Enter an end date." });
    expect(validateDayRange({ from: "2026-07-15", to: "2026-07-01" }, rules)).toMatchObject({ ok: false, field: "to" });
    expect(validateDayRange({ from: "2026-07-15", to: "2026-07-21" }, rules)).toEqual({
      ok: false,
      field: "to",
      message: "The range can't end after today.",
    });
    expect(validateDayRange({ from: "2025-01-01", to: "2026-07-01" }, rules)).toEqual({
      ok: false,
      field: "from",
      message: "Choose 366 days or fewer — this range has 547.",
    });
  });

  it("applies only the rules it is given", () => {
    expect(validateDayRange({ from: "2020-01-01", to: "2030-01-01" })).toEqual({ ok: true, days: 3654 });
  });
});

describe("labels", () => {
  it("writes days and ranges the way the reports do", () => {
    expect(formatDayKey("2026-07-15")).toBe("15 Jul");
    expect(formatDayKey("2026-07-15", { year: true, weekday: true })).toBe("Wed, 15 Jul 2026");
    expect(formatDayRange("2026-07-01", "2026-07-15")).toBe("1 Jul – 15 Jul 2026");
    expect(formatDayRange("2025-12-28", "2026-01-03")).toBe("28 Dec 2025 – 3 Jan 2026");
    expect(formatDayRange("2026-07-15", "2026-07-15")).toBe("15 Jul 2026");
  });

  it("formats an instant as read in the zone (the zone option always wins)", () => {
    const t = Date.UTC(2026, 6, 14, 18, 15);
    expect(formatInZone(t, "Asia/Kathmandu", { hour: "2-digit", minute: "2-digit", hourCycle: "h23" })).toBe("00:00");
    expect(formatInZone(t, "Asia/Kathmandu", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" })).toBe("15 Jul 2026");
    expect(formatInZone(new Date(t), "UTC", { day: "numeric", month: "short" }, "en-US")).toBe("Jul 14");
  });

  it("writes UTC offsets compactly", () => {
    expect(formatUtcOffset(0)).toBe("UTC");
    expect(formatUtcOffset(360)).toBe("UTC+6");
    expect(formatUtcOffset(345)).toBe("UTC+5:45");
    expect(formatUtcOffset(-210)).toBe("UTC-3:30");
    expect(formatUtcOffset(Number.NaN)).toBe("UTC");
  });

  it("labels a zone by its place and current offset", () => {
    expect(timeZoneCity("America/Argentina/Buenos_Aires")).toBe("Buenos Aires");
    expect(timeZoneLabel("Asia/Dhaka", at("2026-07-14T12:00:00Z"))).toBe("Dhaka (UTC+6)");
    expect(timeZoneLabel("Asia/Kathmandu", at("2026-07-14T12:00:00Z"))).toBe("Kathmandu (UTC+5:45)");
    expect(timeZoneLabel("America/New_York", at("2026-01-14T12:00:00Z"))).toBe("New York (UTC-5)");
    expect(timeZoneLabel("UTC")).toBe("UTC");
    expect(timeZoneLabel("Mars/Phobos")).toBe("UTC");
  });
});
