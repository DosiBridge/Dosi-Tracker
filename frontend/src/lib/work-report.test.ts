import { describe, expect, it } from "vitest";
import {
  DAILY_LOG_HEADERS,
  MAX_REPORT_DAYS,
  NOTES_CSV_HEADERS,
  TARGET_DAY_MINUTES,
  activityLevel,
  activityLevelLabel,
  aggregateTracked,
  aggregatesFromLive,
  apportionPercent,
  blockIdleMinutes,
  buildHighlights,
  calendarWeeks,
  changeVs,
  comparisonFor,
  comparisonLabel,
  comparisonSpan,
  comparisonText,
  formatActivity,
  formatHours,
  formatStampInZone,
  formatTotal,
  idleShare,
  inputsFromAggregates,
  membersOnDay,
  noteBreakdown,
  notesByDay,
  notesByMember,
  notesCsvRows,
  optionalMinutes,
  optionalPercent,
  parseDailySeries,
  parseSummary,
  parseUserDailySeries,
  periodBounds,
  periodSpan,
  periodTitle,
  resolvePeriod,
  selectionTitle,
  sumDecimalHours,
  summaryText,
  trackedRowFromActivity,
  barPercent,
  buildDailyLog,
  buildTimesheetMatrix,
  chartSeries,
  cleanMinutes,
  cleanPercent,
  countMembers,
  dailyLogCsvRows,
  decimalHours,
  formatAxisDay,
  formatDayLabel,
  formatHM,
  formatHoursLong,
  formatPeriod,
  formatStamp,
  heatLevel,
  hourTicks,
  inputsFromActivities,
  inputsFromLive,
  isWeekend,
  matrixCsv,
  perMemberDay,
  projectBreakdown,
  projectLabel,
  reportSpan,
  scopeLabel,
  summarizeLog,
  utcDayKey,
  weekdayShort,
  type DailyPoint,
  type MemberDayPoint,
  type NoteBlock,
  type PeriodPreset,
  type PeriodSelection,
  type TrackedRow,
} from "./work-report";
import { buildActivity } from "@/test/factories";

/* All expectations below are timezone-independent: local-calendar inputs are
 * built with the local Date(y, m, d, …) constructor, zoned ones name their
 * IANA zone, and day keys are calendar days. */

const keys = (from: string, count: number) =>
  Array.from({ length: count }, (_, i) => new Date(Date.parse(`${from}T00:00:00Z`) + i * 86_400_000).toISOString().slice(0, 10));
const utcDay = (ms: number) => new Date(ms).toISOString().slice(0, 10);
const at = (iso: string) => Date.parse(iso);

describe("sanitising", () => {
  it("cleanMinutes keeps positive finite numbers and zeroes everything else", () => {
    expect(cleanMinutes(42.5)).toBe(42.5);
    expect(cleanMinutes("30")).toBe(30);
    expect(cleanMinutes(-5)).toBe(0);
    expect(cleanMinutes(Number.NaN)).toBe(0);
    expect(cleanMinutes(Number.POSITIVE_INFINITY)).toBe(0);
    expect(cleanMinutes(null)).toBe(0);
    expect(cleanMinutes(undefined)).toBe(0);
    expect(cleanMinutes("")).toBe(0);
    expect(cleanMinutes("abc")).toBe(0);
    expect(cleanMinutes(true)).toBe(0);
  });

  it("cleanPercent clamps to 0–100 and zeroes non-finite input", () => {
    expect(cleanPercent(72.4)).toBe(72.4);
    expect(cleanPercent(140)).toBe(100);
    expect(cleanPercent(-3)).toBe(0);
    expect(cleanPercent(Number.NaN)).toBe(0);
    expect(cleanPercent("55")).toBe(55);
    expect(cleanPercent({})).toBe(0);
  });

  it("optionalPercent reads 'not measured' as null, never as 0", () => {
    expect(optionalPercent(null)).toBeNull();
    expect(optionalPercent(undefined)).toBeNull();
    expect(optionalPercent("")).toBeNull();
    expect(optionalPercent(Number.NaN)).toBeNull();
    expect(optionalPercent("abc")).toBeNull();
    expect(optionalPercent(0)).toBe(0);
    expect(optionalPercent("64.5")).toBe(64.5);
    expect(optionalPercent(130)).toBe(100);
    expect(optionalPercent(-4)).toBe(0);
  });

  it("optionalMinutes reads an absent figure as unknown (null)", () => {
    expect(optionalMinutes(undefined)).toBeNull();
    expect(optionalMinutes(null)).toBeNull();
    expect(optionalMinutes("x")).toBeNull();
    expect(optionalMinutes(0)).toBe(0);
    expect(optionalMinutes(-3)).toBe(0);
    expect(optionalMinutes("12.5")).toBe(12.5);
  });
});

describe("day keys and labels", () => {
  it("reads the calendar day straight off server strings without shifting it", () => {
    expect(utcDayKey("2026-09-28T00:00:00")).toBe("2026-09-28");
    expect(utcDayKey("2026-09-28T00:00:00Z")).toBe("2026-09-28");
    expect(utcDayKey("2026-09-28")).toBe("2026-09-28");
    expect(utcDayKey(" 2026-09-28T23:59:59.999Z ")).toBe("2026-09-28");
    expect(utcDayKey(new Date("2026-09-28T23:30:00Z"))).toBe("2026-09-28");
  });

  it("rejects unreadable or impossible dates", () => {
    expect(utcDayKey("2026-02-30")).toBeNull();
    expect(utcDayKey("2026-13-01")).toBeNull();
    expect(utcDayKey("28/09/2026")).toBeNull();
    expect(utcDayKey("")).toBeNull();
    expect(utcDayKey(null)).toBeNull();
    expect(utcDayKey(20260928)).toBeNull();
    expect(utcDayKey(new Date(Number.NaN))).toBeNull();
  });

  it("knows weekends and weekday names", () => {
    expect(weekdayShort("2026-09-28")).toBe("Mon");
    expect(weekdayShort("2026-10-04")).toBe("Sun");
    expect(isWeekend("2026-10-03")).toBe(true); // Saturday
    expect(isWeekend("2026-10-04")).toBe(true); // Sunday
    expect(isWeekend("2026-10-02")).toBe(false); // Friday
    expect(isWeekend("2026-09-28")).toBe(false); // Monday
  });

  it("formats day labels deterministically", () => {
    expect(formatDayLabel("2026-09-28")).toBe("Mon, 28 Sep");
    expect(formatDayLabel("2026-10-04", { year: true })).toBe("Sun, 4 Oct 2026");
    expect(formatAxisDay("2026-09-28", false)).toBe("Mon 28");
    expect(formatAxisDay("2026-09-28", true)).toBe("28 Sep");
  });

  it("formats the period: one year at the end, both across New Year, a single day, none", () => {
    expect(formatPeriod(keys("2026-09-28", 7))).toBe("Mon, 28 Sep – Sun, 4 Oct 2026");
    expect(formatPeriod(keys("2026-12-30", 7))).toBe("Wed, 30 Dec 2026 – Tue, 5 Jan 2027");
    expect(formatPeriod(["2026-10-04"])).toBe("Sun, 4 Oct 2026");
    expect(formatPeriod([])).toBe("—");
  });

  it("stamps the generation time in local time, or as the reporting zone reads it", () => {
    expect(formatStamp(new Date(2026, 9, 4, 15, 30))).toBe("4 Oct 2026, 15:30");
    expect(formatStamp(new Date(2026, 0, 9, 7, 5))).toBe("9 Jan 2026, 07:05");
    expect(formatStamp(new Date(Number.NaN))).toBe("—");
    const instant = new Date("2026-10-04T09:30:00Z");
    expect(formatStampInZone(instant, "Asia/Dhaka")).toBe("4 Oct 2026, 15:30");
    expect(formatStampInZone(instant, "America/New_York")).toBe("4 Oct 2026, 05:30");
    expect(formatStampInZone(instant, "Asia/Kathmandu")).toBe("4 Oct 2026, 15:15");
    expect(formatStampInZone(new Date(Number.NaN), "UTC")).toBe("—");
  });
});

describe("reportSpan — whole calendar days", () => {
  it("snaps a rolling 'last 7 days' window to exactly 7 full days ending today", () => {
    const now = new Date(2026, 9, 4, 15, 30);
    const from = new Date(2026, 8, 27, 15, 30);
    const span = reportSpan(from, now);
    expect(span.days).toEqual(keys("2026-09-28", 7));
    expect(span.from.toISOString()).toBe("2026-09-28T00:00:00.000Z");
    expect(span.to.toISOString()).toBe("2026-10-05T00:00:00.000Z");
  });

  it("keeps a midnight-aligned start (today, this month, yesterday)", () => {
    const now = new Date(2026, 9, 4, 15, 30);
    expect(reportSpan(new Date(2026, 9, 4), now).days).toEqual(["2026-10-04"]);
    expect(reportSpan(new Date(2026, 9, 1), now).days).toEqual(keys("2026-10-01", 4));
    const yesterday = reportSpan(new Date(2026, 9, 3), new Date(2026, 9, 3, 23, 59, 59, 999));
    expect(yesterday.days).toEqual(["2026-10-03"]);
    expect(yesterday.to.toISOString()).toBe("2026-10-04T00:00:00.000Z");
  });

  it("a 30-day rolling window is 30 days", () => {
    const span = reportSpan(new Date(2026, 8, 4, 9, 0), new Date(2026, 9, 4, 9, 0));
    expect(span.days).toHaveLength(30);
    expect(span.days[0]).toBe("2026-09-05");
    expect(span.days[29]).toBe("2026-10-04");
  });

  it("never snaps a same-day mid-day range to nothing", () => {
    const span = reportSpan(new Date(2026, 9, 4, 9, 0), new Date(2026, 9, 4, 17, 0));
    expect(span.days).toEqual(["2026-10-04"]);
  });

  it("spans months and leap days", () => {
    expect(reportSpan(new Date(2028, 1, 27), new Date(2028, 2, 1, 12)).days).toEqual([
      "2028-02-27",
      "2028-02-28",
      "2028-02-29",
      "2028-03-01",
    ]);
  });

  it("returns no days for inverted or invalid ranges", () => {
    const a = new Date(2026, 9, 4);
    const b = new Date(2026, 9, 1);
    expect(reportSpan(a, b).days).toEqual([]);
    expect(reportSpan(new Date(Number.NaN), a).days).toEqual([]);
    expect(reportSpan(a, new Date(Number.NaN)).days).toEqual([]);
    expect(reportSpan(b, a, 0).days).toEqual([]);
  });

  it("caps very long ranges to the most recent days the backend accepts", () => {
    const span = reportSpan(new Date(2020, 0, 1), new Date(2026, 9, 4, 12));
    expect(span.days).toHaveLength(MAX_REPORT_DAYS);
    expect(span.days.at(-1)).toBe("2026-10-04");
    expect(span.from.toISOString().slice(0, 10)).toBe(span.days[0]);
    expect(reportSpan(new Date(2026, 9, 1), new Date(2026, 9, 4), 2).days).toEqual(["2026-10-03", "2026-10-04"]);
  });

  it("does not mutate its inputs", () => {
    const from = new Date(2026, 8, 27, 15, 30);
    const to = new Date(2026, 9, 4, 15, 30);
    const [f, t] = [from.getTime(), to.getTime()];
    reportSpan(from, to);
    expect([from.getTime(), to.getTime()]).toEqual([f, t]);
  });
});

describe("formatting hours", () => {
  it("formats h:mm", () => {
    expect(formatHM(452)).toBe("7:32");
    expect(formatHM(5)).toBe("0:05");
    expect(formatHM(0)).toBe("0:00");
    expect(formatHM(2460)).toBe("41:00");
    expect(formatHM(59.6)).toBe("1:00"); // rounds before splitting — never "0:60"
    expect(formatHM(Number.NaN)).toBe("0:00");
    expect(formatHM(-30)).toBe("0:00");
  });

  it("formats '7h 32m'", () => {
    expect(formatHoursLong(452)).toBe("7h 32m");
    expect(formatHoursLong(45)).toBe("45m");
    expect(formatHoursLong(480)).toBe("8h");
    expect(formatHoursLong(0)).toBe("0m");
    expect(formatHoursLong(119.6)).toBe("2h"); // not "1h 60m"
    expect(formatHoursLong("garbage")).toBe("0m");
  });

  it("converts to decimal hours with 2 places", () => {
    expect(decimalHours(452)).toBe(7.53);
    expect(decimalHours(90)).toBe(1.5);
    expect(decimalHours(1)).toBe(0.02);
    expect(decimalHours(-60)).toBe(0);
  });

  it("a decimal total is the sum of the rounded rows above it, not the rounded sum of the minutes", () => {
    const week = [452, 452, 452, 452, 452]; // five days of 7:32
    expect(sumDecimalHours(week)).toBe(37.65); // 5 × 7.53 — the minutes alone would round to 37.67
    expect(formatTotal(week, "decimal")).toBe("37.65");
    expect(formatTotal(week, "duration")).toBe("37h 40m");
    expect(formatTotal(week, "clock")).toBe("37:40");
    expect(formatTotal([], "decimal")).toBe("0.00");
    expect(sumDecimalHours([1, 1, 1])).toBe(0.06);
    expect(sumDecimalHours([Number.NaN, -5, 30])).toBe(0.5);
  });

  it("reads unmeasured activity as a dash", () => {
    expect(formatActivity(72.6)).toBe("73%");
    expect(formatActivity(0)).toBe("0%");
    expect(formatActivity(null)).toBe("—");
    expect(formatActivity(undefined)).toBe("—");
  });

  it("idle time as a whole-percent share of the tracked time", () => {
    expect(idleShare(60, 480)).toBe(13);
    expect(idleShare(0, 480)).toBe(0);
    expect(idleShare(30, 0)).toBe(0);
    expect(idleShare(600, 480)).toBe(100);
  });
});

describe("activityLevel", () => {
  it("uses the app's 75/50 thresholds and 'none' without time or a measurement", () => {
    expect(activityLevel(75)).toBe("good");
    expect(activityLevel(74.9)).toBe("ok");
    expect(activityLevel(50)).toBe("ok");
    expect(activityLevel(49)).toBe("low");
    expect(activityLevel(0)).toBe("low");
    expect(activityLevel(90, 0)).toBe("none");
    expect(activityLevel(null, 120)).toBe("none");
    expect(activityLevel(undefined)).toBe("none");
    expect(activityLevel(Number.NaN)).toBe("low");
    expect(activityLevelLabel.none).toBe("—");
  });
});

describe("buildDailyLog", () => {
  const days = keys("2026-09-28", 7);

  it("lays out one row per day with zero days included and weekends flagged", () => {
    const log = buildDailyLog(days, [{ date: "2026-09-29T00:00:00Z", minutes: 452, activity: 80, sessions: 9 }]);
    expect(log.map((r) => r.date)).toEqual(days);
    expect(log[1]).toEqual({
      date: "2026-09-29",
      weekday: "Tue",
      label: "Tue, 29 Sep",
      weekend: false,
      minutes: 452,
      activity: 80,
      measuredMinutes: 452,
      idleMinutes: null,
      sessions: 9,
      level: "good",
    });
    expect(log[0]).toMatchObject({ minutes: 0, activity: null, sessions: 0, level: "none", idleMinutes: null });
    expect(log.filter((r) => r.weekend).map((r) => r.weekday)).toEqual(["Sat", "Sun"]);
  });

  it("merges several points on one day with activity weighted by the minutes it was measured over", () => {
    const log = buildDailyLog(["2026-09-28"], [
      { date: "2026-09-28T09:00:00Z", minutes: 90, activity: 50, sessions: 1 },
      { date: "2026-09-28", minutes: 30, activity: 90, sessions: 1 },
    ]);
    // (90×50 + 30×90) / 120 = 60
    expect(log[0]).toMatchObject({ minutes: 120, activity: 60, sessions: 2, level: "ok" });
  });

  it("leaves unmeasured minutes out of the day's activity, and says when nothing was measured", () => {
    const log = buildDailyLog(["2026-09-28", "2026-09-29"], [
      { date: "2026-09-28", minutes: 60, activity: 80, sessions: 1 },
      { date: "2026-09-28", minutes: 120, activity: null, sessions: 2 }, // a Mac block from an older agent
      { date: "2026-09-29", minutes: 45, sessions: 1 },
      { date: "2026-09-29", minutes: 60, activity: 50, measuredMinutes: 0 }, // a score without measured minutes counts for nothing
    ]);
    expect(log[0]).toMatchObject({ minutes: 180, activity: 80, measuredMinutes: 60, level: "good" });
    expect(log[1]).toMatchObject({ minutes: 105, activity: null, measuredMinutes: 0, level: "none" });
  });

  it("sums idle time per day, capped at the day's minutes; days without any idle figure read unknown", () => {
    const log = buildDailyLog(["2026-09-28", "2026-09-29"], [
      { date: "2026-09-28", minutes: 60, idleMinutes: 12.4 },
      { date: "2026-09-28", minutes: 30, idleMinutes: 50 }, // more idle than the block: capped at its 30
      { date: "2026-09-29", minutes: 45 },
    ]);
    expect(log[0].idleMinutes).toBe(42);
    expect(log[1].idleMinutes).toBeNull();
  });

  it("rounds each day to whole minutes", () => {
    const log = buildDailyLog(["2026-09-28", "2026-09-29"], [
      { date: "2026-09-28", minutes: 30.4 },
      { date: "2026-09-29", minutes: 30.4 },
    ]);
    expect(log.map((r) => r.minutes)).toEqual([30, 30]);
    expect(summarizeLog(log).totalMinutes).toBe(60); // sum of the rows, not round(60.8)
  });

  it("ignores garbage points and sanitises NaN / negative values", () => {
    const log = buildDailyLog(["2026-09-28"], [
      null as unknown as DailyPoint,
      { date: "not a date", minutes: 60 },
      { date: "2026-09-28", minutes: Number.NaN, activity: 90, sessions: 1 },
      { date: "2026-09-28", minutes: -60, activity: 90 },
      { date: "2026-09-28", minutes: 60, activity: Number.NaN, sessions: -2 },
    ]);
    expect(log).toHaveLength(1);
    // The 60 real minutes carry an unreadable score: not measured, not 0 %.
    expect(log[0]).toMatchObject({ minutes: 60, activity: null, sessions: 1, level: "none" });
  });

  it("keeps tracked time that falls outside the period as an extra row, but not empty points", () => {
    const log = buildDailyLog(["2026-09-28"], [
      { date: "2026-09-27", minutes: 15, sessions: 1 },
      { date: "2026-09-30", minutes: 0, sessions: 0 },
    ]);
    expect(log.map((r) => r.date)).toEqual(["2026-09-27", "2026-09-28"]);
  });

  it("dedupes repeated days and is empty for no days and no points", () => {
    expect(buildDailyLog(["2026-09-28", "2026-09-28T00:00:00Z"], []).map((r) => r.date)).toEqual(["2026-09-28"]);
    expect(buildDailyLog([], [])).toEqual([]);
  });
});

describe("summarizeLog", () => {
  it("totals, averages and counts from the rows", () => {
    const log = buildDailyLog(keys("2026-09-28", 7), [
      { date: "2026-09-28", minutes: 480, activity: 80, sessions: 10, idleMinutes: 40 },
      { date: "2026-09-29", minutes: 240, activity: 50, sessions: 5, idleMinutes: 20 },
      { date: "2026-10-03", minutes: 60, activity: 20, sessions: 1, idleMinutes: 30 }, // Saturday
    ]);
    const s = summarizeLog(log);
    expect(s).toMatchObject({
      totalMinutes: 780,
      dayCount: 7,
      workedDays: 3,
      weekdayCount: 5,
      averagePerWorkedDay: 260,
      sessions: 16,
      maxDayMinutes: 480,
      targetDays: 1,
      weekendMinutes: 60,
      idleMinutes: 90,
    });
    // (480×80 + 240×50 + 60×20) / 780 = 66.15… → 66
    expect(s.averageActivity).toBe(66);
    expect(s.level).toBe("ok");
    expect(s.busiest?.date).toBe("2026-09-28");
  });

  it("takes the period's activity from a source that knows it exactly", () => {
    const log = buildDailyLog(keys("2026-09-28", 2), [
      { date: "2026-09-28", minutes: 300, activity: 80 },
      { date: "2026-09-29", minutes: 152, activity: 60 },
    ]);
    expect(summarizeLog(log, { activity: 73.27 }).averageActivity).toBe(73);
    expect(summarizeLog(log, { activity: null })).toMatchObject({ averageActivity: null, level: "none" });
    expect(summarizeLog(log).averageActivity).toBe(73); // (300×80 + 152×60) / 452
  });

  it("an empty period reads as zeros and dashes, never NaN", () => {
    const s = summarizeLog(buildDailyLog(keys("2026-09-28", 3), []));
    expect(s).toMatchObject({
      totalMinutes: 0,
      dayCount: 3,
      workedDays: 0,
      averagePerWorkedDay: 0,
      averageActivity: null,
      level: "none",
      busiest: null,
      maxDayMinutes: 0,
      idleMinutes: null,
    });
    expect(summarizeLog([])).toMatchObject({ dayCount: 0, totalMinutes: 0, averagePerWorkedDay: 0 });
    // Even a source's figure is no activity for a period without time.
    expect(summarizeLog(buildDailyLog(keys("2026-09-28", 1), []), { activity: 80 }).averageActivity).toBeNull();
  });

  it("picks the earliest day on a tie for busiest", () => {
    const log = buildDailyLog(["2026-09-28", "2026-09-29"], [
      { date: "2026-09-28", minutes: 100 },
      { date: "2026-09-29", minutes: 100 },
    ]);
    expect(summarizeLog(log).busiest?.date).toBe("2026-09-28");
  });
});

describe("bars and heat", () => {
  it("barPercent is proportional with a visible minimum and no NaN", () => {
    expect(barPercent(240, 480)).toBe(50);
    expect(barPercent(480, 480)).toBe(100);
    expect(barPercent(600, 480)).toBe(100);
    expect(barPercent(1, 480)).toBe(2);
    expect(barPercent(0, 480)).toBe(0);
    expect(barPercent(60, 0)).toBe(0);
    expect(barPercent(Number.NaN, 480)).toBe(0);
  });

  it("heatLevel buckets into quarters of the max", () => {
    expect(heatLevel(0, 480)).toBe(0);
    expect(heatLevel(1, 480)).toBe(1);
    expect(heatLevel(120, 480)).toBe(1);
    expect(heatLevel(121, 480)).toBe(2);
    expect(heatLevel(240, 480)).toBe(2);
    expect(heatLevel(360, 480)).toBe(3);
    expect(heatLevel(361, 480)).toBe(4);
    expect(heatLevel(999, 480)).toBe(4);
    expect(heatLevel(60, 0)).toBe(0);
    expect(heatLevel(-5, 480)).toBe(0);
  });

  it("hourTicks picks round steps covering the max and the floor", () => {
    expect(hourTicks(7.5, 8)).toEqual({ top: 8, ticks: [0, 2, 4, 6, 8] });
    expect(hourTicks(9.2, 8)).toEqual({ top: 10, ticks: [0, 2, 4, 6, 8, 10] });
    expect(hourTicks(32.5)).toEqual({ top: 40, ticks: [0, 8, 16, 24, 32, 40] });
    expect(hourTicks(3)).toEqual({ top: 3, ticks: [0, 1, 2, 3] });
    expect(hourTicks(0)).toEqual({ top: 1, ticks: [0, 1] });
    expect(hourTicks(Number.NaN)).toEqual({ top: 1, ticks: [0, 1] });
    expect(hourTicks(150)).toEqual({ top: 192, ticks: [0, 48, 96, 144, 192] });
    for (const max of [0.4, 5, 11, 23, 47, 99, 700]) {
      const { top, ticks } = hourTicks(max);
      expect(top).toBeGreaterThanOrEqual(max);
      expect(ticks.length - 1).toBeLessThanOrEqual(5);
      expect(ticks.at(-1)).toBe(top);
    }
  });

  it("chartSeries converts minutes to hours and carries activity and idle time", () => {
    const log = buildDailyLog(["2026-10-03"], [{ date: "2026-10-03", minutes: 452, activity: 70, idleMinutes: 31 }]);
    expect(chartSeries(log)).toEqual([
      { date: "2026-10-03", label: "Sat, 3 Oct", hours: 7.53, minutes: 452, activity: 70, idleMinutes: 31, weekend: true },
    ]);
  });
});

describe("buildTimesheetMatrix", () => {
  const nameOf = (id: string, hint: string | null) => ({ a: "Ana", b: "Ben" })[id] ?? hint ?? `Member ${id}`;

  it("builds members × days with row, column and grand totals", () => {
    const m = buildTimesheetMatrix(["2026-09-28", "2026-09-29"], [
      { userId: "a", date: "2026-09-28T00:00:00Z", minutes: 60 },
      { userId: "a", date: "2026-09-29", minutes: 30 },
      { userId: "b", date: "2026-09-28", minutes: 120 },
      { userId: "b", date: "2026-09-28", minutes: 15 }, // merged into the same cell
    ], nameOf);
    expect(m.days).toEqual(["2026-09-28", "2026-09-29"]);
    expect(m.rows).toEqual([
      { userId: "b", name: "Ben", cells: [135, 0], total: 135, workedDays: 1, idleMinutes: null },
      { userId: "a", name: "Ana", cells: [60, 30], total: 90, workedDays: 2, idleMinutes: null },
    ]);
    expect(m.dayTotals).toEqual([195, 30]);
    expect(m.grandTotal).toBe(225);
    expect(m.maxCell).toBe(135);
    expect(m.idleTotal).toBeNull();
  });

  it("adds up each member's idle time when the source reports it", () => {
    const m = buildTimesheetMatrix(["2026-09-28", "2026-09-29"], [
      { userId: "a", date: "2026-09-28", minutes: 60, idleMinutes: 10.4 },
      { userId: "a", date: "2026-09-29", minutes: 30, idleMinutes: 5 },
      { userId: "b", date: "2026-09-28", minutes: 40 },
    ], nameOf);
    expect(m.rows.find((r) => r.userId === "a")?.idleMinutes).toBe(15);
    expect(m.rows.find((r) => r.userId === "b")?.idleMinutes).toBeNull();
    expect(m.idleTotal).toBe(15);
  });

  it("totals equal the sum of the rounded cells", () => {
    const m = buildTimesheetMatrix(["2026-09-28", "2026-09-29"], [
      { userId: "a", date: "2026-09-28", minutes: 10.4 },
      { userId: "a", date: "2026-09-29", minutes: 10.4 },
      { userId: "b", date: "2026-09-28", minutes: 10.4 },
    ], nameOf);
    expect(m.rows.find((r) => r.userId === "a")?.total).toBe(20);
    expect(m.dayTotals).toEqual([20, 10]);
    expect(m.grandTotal).toBe(30);
  });

  it("uses the source's name hint, orders ties by name, and drops empty / malformed rows", () => {
    const m = buildTimesheetMatrix(["2026-09-28"], [
      { userId: "z", date: "2026-09-28", minutes: 60, name: " zed " },
      { userId: "a", date: "2026-09-28", minutes: 60 },
      { userId: "idle", date: "2026-09-28", minutes: 0 },
      { userId: "", date: "2026-09-28", minutes: 60 },
      { userId: 7, date: "2026-09-28", minutes: 60 } as MemberDayPoint,
      { userId: "x", date: "garbage", minutes: 60 },
      { userId: "tiny", date: "2026-09-28", minutes: 0.2 }, // rounds to nothing
      null as unknown as MemberDayPoint,
    ], nameOf);
    expect(m.rows.map((r) => r.name)).toEqual(["Ana", "zed"]);
  });

  it("adds a column for tracked time outside the given days instead of dropping it", () => {
    const m = buildTimesheetMatrix(["2026-09-28"], [{ userId: "a", date: "2026-09-30", minutes: 45 }], nameOf);
    expect(m.days).toEqual(["2026-09-28", "2026-09-30"]);
    expect(m.rows[0].cells).toEqual([0, 45]);
  });

  it("is empty without points", () => {
    const m = buildTimesheetMatrix(keys("2026-09-28", 3), [], nameOf);
    expect(m.rows).toEqual([]);
    expect(m.dayTotals).toEqual([0, 0, 0]);
    expect(m.grandTotal).toBe(0);
    expect(m.maxCell).toBe(0);
    expect(perMemberDay(m)).toEqual({ minutes: 0, memberDays: 0 });
  });

  it("averages a team per member-day, not per calendar day", () => {
    const m = buildTimesheetMatrix(["2026-09-28", "2026-09-29"], [
      { userId: "a", date: "2026-09-28", minutes: 480 },
      { userId: "a", date: "2026-09-29", minutes: 420 },
      { userId: "b", date: "2026-09-28", minutes: 300 },
    ], nameOf);
    // 1200 minutes over 3 member-days (the calendar-day average would read 10h).
    expect(perMemberDay(m)).toEqual({ minutes: 400, memberDays: 3 });
  });
});

describe("projects", () => {
  it("apportions whole percentages that always add up to 100", () => {
    expect(apportionPercent([1, 1, 1])).toEqual([34, 33, 33]);
    expect(apportionPercent([2, 1])).toEqual([67, 33]);
    expect(apportionPercent([0, 0])).toEqual([0, 0]);
    expect(apportionPercent([])).toEqual([]);
    expect(apportionPercent([5, Number.NaN, -1])).toEqual([100, 0, 0]);
    const odd = apportionPercent([7, 13, 29, 51, 3, 11]);
    expect(odd.reduce((s, v) => s + v, 0)).toBe(100);
  });

  it("labels unknown projects with a short id", () => {
    expect(projectLabel("0f7c9a2e-1111", { title: "Apollo" })).toBe("Apollo");
    expect(projectLabel("0f7c9a2e-1111", { title: "  " })).toBe("Project 0f7c9a2e");
    expect(projectLabel("p9", null)).toBe("Project p9");
  });

  it("breaks hours down per project, largest first, merged, without zero rows", () => {
    const lookup = (id: string) => (id === "p1" ? { title: "Apollo", color: "#ff0000" } : undefined);
    const list = projectBreakdown(
      [
        { projectId: "p1", minutes: 60 },
        { projectId: "p2-unknown-id", minutes: 120 },
        { projectId: "p1", minutes: 60 },
        { projectId: "p3", minutes: 0 },
        { projectId: "", minutes: 50 },
        { projectId: "p4", minutes: Number.NaN },
        null as unknown as { projectId: string; minutes: number },
      ],
      lookup,
    );
    expect(list).toEqual([
      { projectId: "p1", name: "Apollo", color: "#ff0000", minutes: 120, share: 50, archived: false },
      { projectId: "p2-unknown-id", name: "Project p2-unkno", color: null, minutes: 120, share: 50, archived: false },
    ]);
    expect(projectBreakdown([], lookup)).toEqual([]);
  });

  it("keeps archived projects with time (so the shares cover every minute), named by the server when the roster can't", () => {
    const list = projectBreakdown(
      [
        { projectId: "old", minutes: 90, title: "Legacy CRM", archived: true },
        { projectId: "p1", minutes: 30 },
      ],
      (id) => (id === "p1" ? { title: "Apollo", archived: false } : undefined),
    );
    expect(list.map((p) => [p.name, p.archived, p.share])).toEqual([
      ["Legacy CRM", true, 75],
      ["Apollo", false, 25],
    ]);
  });
});

describe("scope and members", () => {
  it("describes the scope", () => {
    expect(scopeLabel()).toBe("All projects · All members");
    expect(scopeLabel("Apollo", null)).toBe("Apollo · All members");
    expect(scopeLabel(" ", "Rina Das")).toBe("All projects · Rina Das");
  });

  it("counts distinct members with time", () => {
    expect(
      countMembers([
        { userId: "a", minutes: 10 },
        { userId: "a", minutes: 5 },
        { userId: "b", minutes: 0 },
        { userId: "c", minutes: 3 },
        { userId: 4, minutes: 3 },
      ]),
    ).toBe(2);
    expect(countMembers([])).toBe(0);
  });
});

/* ----------------------------- Aggregates ----------------------------- */

function row(userId: string, projectId: string, startIso: string, minutes: number, extra: Partial<TrackedRow> = {}): TrackedRow {
  const start = at(startIso);
  return { userId, projectId, start, end: start + minutes * 60_000, minutes, activity: 70, idleMinutes: null, keys: 0, clicks: 0, ...extra };
}

describe("aggregateTracked — the backend's rules over blocks", () => {
  const rows = [
    row("u1", "p1", "2026-09-28T23:30:00Z", 60, { activity: 80, idleMinutes: 6, keys: 100, clicks: 10 }), // runs past midnight
    row("u1", "p2", "2026-09-29T09:00:00Z", 30, { activity: null }), // not measured
    row("u2", "p1", "2026-09-29T10:00:00Z", 60, { activity: 50, idleMinutes: 30, keys: 50, clicks: 5 }),
  ];
  const agg = aggregateTracked(rows, utcDay);

  it("books each block wholly to the day it started on", () => {
    expect(agg.daily).toEqual([
      { day: "2026-09-28", minutes: 60, sessions: 1, activity: 80, measuredMinutes: 60, idleMinutes: 6, keys: 100, clicks: 10 },
      { day: "2026-09-29", minutes: 90, sessions: 2, activity: 50, measuredMinutes: 60, idleMinutes: 30, keys: 50, clicks: 5 },
    ]);
  });

  it("weights activity by measured minutes and sums only the idle time that was measured", () => {
    expect(agg.totals).toEqual({ minutes: 150, sessions: 3, activity: 65, idleMinutes: 36, keys: 150, clicks: 15 });
    expect(agg.members).toEqual([
      { userId: "u1", name: null, minutes: 90, sessions: 2, activity: 80, idleMinutes: 6 },
      { userId: "u2", name: null, minutes: 60, sessions: 1, activity: 50, idleMinutes: 30 },
    ]);
    expect(agg.projects).toEqual([
      { projectId: "p1", title: null, archived: false, minutes: 120, sessions: 2, activity: 65, idleMinutes: 36 },
      { projectId: "p2", title: null, archived: false, minutes: 30, sessions: 1, activity: null, idleMinutes: null },
    ]);
  });

  it("keeps each member's day with its first start and last end, and the member × project minutes", () => {
    expect(agg.memberDays.map((d) => [d.userId, d.day, d.minutes, d.idleMinutes])).toEqual([
      ["u1", "2026-09-28", 60, 6],
      ["u2", "2026-09-29", 60, 30],
      ["u1", "2026-09-29", 30, null],
    ]);
    expect(agg.memberDays[0]).toMatchObject({ firstStart: at("2026-09-28T23:30:00Z"), lastEnd: at("2026-09-29T00:30:00Z") });
    expect(agg.memberProjects).toEqual([
      { userId: "u1", projectId: "p1", minutes: 60 },
      { userId: "u2", projectId: "p1", minutes: 60 },
      { userId: "u1", projectId: "p2", minutes: 30 },
    ]);
  });

  it("is empty — with nothing measured — for no blocks", () => {
    const none = aggregateTracked([], utcDay);
    expect(none.totals).toEqual({ minutes: 0, sessions: 0, activity: null, idleMinutes: null, keys: null, clicks: null });
    expect(none.daily).toEqual([]);
    expect(none.memberProjects).toEqual([]);
  });
});

describe("blockIdleMinutes — the agents' idle rule", () => {
  it("prefers the idle seconds the agent reported, capped at the block", () => {
    expect(blockIdleMinutes({ minutes: 10, idleSeconds: 120, activity: 90 })).toBe(2);
    expect(blockIdleMinutes({ minutes: 1, idleSeconds: 500, activity: 0 })).toBe(1);
    expect(blockIdleMinutes({ minutes: 10, idleSeconds: "90", activity: null })).toBe(1.5);
  });

  it("else counts the inactive minute buckets, the last bucket absorbing the remainder", () => {
    // 10m45s → 11 buckets, the last 45 s long.
    const timeline = [
      { minute: 3, active: false },
      { minute: 4, active: true },
      { minute: 10, active: false },
      { minute: 3, active: false }, // a repeated minute counts once
      { minute: 12, active: false }, // past the block
    ];
    expect(blockIdleMinutes({ minutes: 10.75, timeline, activity: 80 })).toBe(1.75);
    // 10m15s → 10 buckets, the last one 75 s long (up to 89 s, contract §1).
    expect(blockIdleMinutes({ minutes: 10.25, timeline: [{ minute: 9, active: false }], activity: 90 })).toBe(1.25);
  });

  it("falls back to the share without input only where asked (the demo dataset), and is unknown otherwise", () => {
    expect(blockIdleMinutes({ minutes: 10, activity: 70, fromActivity: true })).toBe(3);
    expect(blockIdleMinutes({ minutes: 10, activity: 70 })).toBeNull();
    expect(blockIdleMinutes({ minutes: 10, activity: null, fromActivity: true })).toBeNull();
  });

  it("a demo activity becomes a tracked row with its idle time derived from its activity %", () => {
    const r = trackedRowFromActivity(buildActivity({ productivity: 85, keyboardHits: 900, mouseClicks: 200 }));
    expect(r).toMatchObject({ minutes: 10, activity: 85, idleMinutes: 1.5, keys: 900, clicks: 200 });
    expect(trackedRowFromActivity(buildActivity({ startedAt: "nope" }))).toBeNull();
    expect(trackedRowFromActivity(buildActivity({ startedAt: "2026-07-14T14:10:00Z", endedAt: "2026-07-14T14:00:00Z" }))).toBeNull();
  });

  it("a row whose input was not counted keeps its counts null, and aggregates add only the measured ones", () => {
    const unmeasured = trackedRowFromActivity(buildActivity({ keyboardHits: null, mouseClicks: null }));
    expect(unmeasured).toMatchObject({ keys: null, clicks: null });
    const measured = trackedRowFromActivity(buildActivity({ keyboardHits: 5, mouseClicks: 0, startedAt: "2026-07-14T15:00:00Z", endedAt: "2026-07-14T15:10:00Z" }));
    const agg = aggregateTracked([unmeasured!, measured!], utcDay);
    expect(agg.totals).toMatchObject({ keys: 5, clicks: 0, sessions: 2 });
    expect(aggregateTracked([unmeasured!], utcDay).totals).toMatchObject({ keys: null, clicks: null });
    expect(aggregateTracked([unmeasured!], utcDay).daily[0]).toMatchObject({ keys: null, clicks: null });
  });

  it("a row flagged as unmeasured is 'not measured', not 0 %; reported idle seconds win", () => {
    const unmeasured = { ...buildActivity({ productivity: 0 }), activityMeasured: false } as Parameters<typeof trackedRowFromActivity>[0];
    expect(trackedRowFromActivity(unmeasured)).toMatchObject({ activity: null, idleMinutes: null });
    const reported = { ...buildActivity({ productivity: 90 }), idleSeconds: 240 } as Parameters<typeof trackedRowFromActivity>[0];
    expect(trackedRowFromActivity(reported)).toMatchObject({ activity: 90, idleMinutes: 4 });
  });
});

describe("parsing the reporting payloads", () => {
  it("reads a summary, with the per-member, per-project and member × project rows", () => {
    const s = parseSummary({
      totalTrackedMinutes: 452,
      totalActivities: 14,
      averageProductivity: null,
      totalIdleMinutes: 40.5,
      totalKeyboardHits: 10,
      totalMouseClicks: 5,
      perUser: [{ userId: "a", userName: "ana", name: "Ana", trackedMinutes: 300, activityCount: 10, averageProductivity: 80, idleMinutes: 30 }, null, { trackedMinutes: 3 }],
      perProject: [{ projectId: "p1", projectTitle: "Apollo", isArchived: true, trackedMinutes: 452, activityCount: 14, averageProductivity: null, idleMinutes: 40.5 }],
      perUserProject: [{ userId: "a", projectId: "p1", trackedMinutes: 300 }, { userId: "a" }],
    });
    expect(s.totals).toEqual({ minutes: 452, sessions: 14, activity: null, idleMinutes: 40.5, keys: 10, clicks: 5 });
    expect(s.members).toEqual([{ userId: "a", name: "Ana", minutes: 300, sessions: 10, activity: 80, idleMinutes: 30 }]);
    expect(s.projects).toEqual([{ projectId: "p1", title: "Apollo", archived: true, minutes: 452, sessions: 14, activity: null, idleMinutes: 40.5 }]);
    expect(s.memberProjects).toEqual([{ userId: "a", projectId: "p1", minutes: 300 }]);
  });

  it("reads what an older server leaves out as unknown, never as zero", () => {
    const s = parseSummary({ totalTrackedMinutes: 452, averageProductivity: 73.27, perUser: [{ userId: "a", userName: "ana", trackedMinutes: 452, averageProductivity: 0 }] });
    expect(s.totals.idleMinutes).toBeNull();
    expect(s.totals.activity).toBe(73.27);
    expect(s.members[0]).toMatchObject({ name: "ana", idleMinutes: null, activity: 0 });
    expect(s.memberProjects).toBeNull();
    expect(parseSummary(null)).toEqual({ totals: { minutes: 0, sessions: 0, activity: null, idleMinutes: null, keys: null, clicks: null }, members: [], projects: [], memberProjects: null });
  });

  it("reads totals over no measured block as not measured (null), and a measured zero as 0", () => {
    const none = parseSummary({ totalKeyboardHits: 0, totalMouseClicks: 0, keyboardMeasuredActivities: 0, mouseMeasuredActivities: 0, totalActivities: 3 });
    expect(none.totals).toMatchObject({ keys: null, clicks: null, sessions: 3 });
    const partly = parseSummary({ totalKeyboardHits: 0, totalMouseClicks: 12, keyboardMeasuredActivities: 2, mouseMeasuredActivities: 1 });
    expect(partly.totals).toMatchObject({ keys: 0, clicks: 12 });
    // An older server says nothing about measurement: its numbers stand.
    expect(parseSummary({ totalKeyboardHits: 4, totalMouseClicks: 0 }).totals).toMatchObject({ keys: 4, clicks: 0 });
  });

  it("reads the daily series as calendar days of the requested zone", () => {
    expect(
      parseDailySeries([
        { date: "2026-07-14T00:00:00", trackedMinutes: 90, activityCount: 6, averageProductivity: 72, idleMinutes: 8, keyboardHits: 40, mouseClicks: 4 },
        { date: "2026-07-15T00:00:00", trackedMinutes: 0, activityCount: 0, averageProductivity: null },
        { date: "2026-07-16T00:00:00", trackedMinutes: 30, activityCount: 3, keyboardHits: 0, mouseClicks: 0, keyboardMeasuredActivities: 0, mouseMeasuredActivities: 2 },
        { date: "garbage", trackedMinutes: 9 },
        7,
      ]),
    ).toEqual([
      { day: "2026-07-14", minutes: 90, sessions: 6, activity: 72, idleMinutes: 8, keys: 40, clicks: 4 },
      { day: "2026-07-15", minutes: 0, sessions: 0, activity: null, idleMinutes: null, keys: null, clicks: null },
      // Nothing that day counted keys: not measured, while its clicks were measured as zero.
      { day: "2026-07-16", minutes: 30, sessions: 3, activity: null, idleMinutes: null, keys: null, clicks: 0 },
    ]);
    expect(parseDailySeries({ not: "an array" })).toEqual([]);
  });

  it("reads the per-member series with the day's first start and last end as UTC instants", () => {
    expect(
      parseUserDailySeries([
        { userId: "a", userName: "ana", date: "2026-07-14T00:00:00", trackedMinutes: 90, activityCount: 6, idleMinutes: 5, firstStartedAt: "2026-07-14T03:00:00", lastEndedAt: "2026-07-14T11:30:00Z" },
        { userId: "b", date: "2026-07-14", trackedMinutes: 30 },
        { userId: "", date: "2026-07-14", trackedMinutes: 30 },
      ]),
    ).toEqual([
      { userId: "a", name: "ana", day: "2026-07-14", minutes: 90, sessions: 6, idleMinutes: 5, firstStart: at("2026-07-14T03:00:00Z"), lastEnd: at("2026-07-14T11:30:00Z") },
      { userId: "b", name: null, day: "2026-07-14", minutes: 30, sessions: 0, idleMinutes: null, firstStart: null, lastEnd: null },
    ]);
  });

  it("who worked on a day, most time first, with their idle time and day bounds", () => {
    const agg = aggregatesFromLive(
      null,
      [],
      [
        { userId: "a", userName: "ana", date: "2026-07-14", trackedMinutes: 90.4, idleMinutes: 5.2, firstStartedAt: "2026-07-14T03:00:00Z", lastEndedAt: "2026-07-14T11:00:00Z" },
        { userId: "b", userName: "ben", date: "2026-07-14", trackedMinutes: 120 },
        { userId: "c", date: "2026-07-14", trackedMinutes: 0.3 },
        { userId: "a", date: "2026-07-15", trackedMinutes: 60 },
      ],
    );
    const names = (id: string, hint: string | null) => (id === "a" ? "Ana" : (hint ?? id));
    expect(membersOnDay(agg.memberDays, "2026-07-14", names)).toEqual([
      { userId: "b", name: "ben", minutes: 120, idleMinutes: null, sessions: 0, firstStart: null, lastEnd: null },
      { userId: "a", name: "Ana", minutes: 90, idleMinutes: 5, sessions: 0, firstStart: at("2026-07-14T03:00:00Z"), lastEnd: at("2026-07-14T11:00:00Z") },
    ]);
    expect(membersOnDay(agg.memberDays, "2026-07-20", names)).toEqual([]);
  });
});

describe("sources", () => {
  it("demo: each seeded activity counts on the day it started, in the reporting zone", () => {
    const acts = [
      buildActivity({ userId: "u1", projectId: "p1", startedAt: "2026-07-13T23:50:00Z", endedAt: "2026-07-14T00:10:00Z", productivity: 60 }),
      buildActivity({ userId: "u2", projectId: "p2", startedAt: "2026-07-14T09:00:00Z", endedAt: "2026-07-14T09:40:00Z", productivity: 90 }),
    ];
    const utc = inputsFromActivities(acts);
    const log = buildDailyLog(["2026-07-13", "2026-07-14"], utc.daily);
    expect(log[0]).toMatchObject({ minutes: 20, activity: 60, sessions: 1 });
    expect(log[1]).toMatchObject({ minutes: 40, activity: 90, sessions: 1 });
    expect(utc.activity).toBe(80); // (20×60 + 40×90) / 60
    expect(countMembers(utc.members)).toBe(2);
    expect(projectBreakdown(utc.projects, () => undefined).map((p) => p.minutes)).toEqual([40, 20]);

    // In Dhaka (UTC+6) the first block starts at 05:50 on the 14th: both land on that day.
    const dhaka = buildDailyLog(["2026-07-13", "2026-07-14"], inputsFromActivities(acts, "Asia/Dhaka").daily);
    expect(dhaka.map((r) => r.minutes)).toEqual([0, 60]);
    expect(inputsFromActivities([])).toEqual({ daily: [], memberDays: [], projects: [], members: [], activity: null });
  });

  it("live: maps the reporting payloads and tolerates garbage", () => {
    const inputs = inputsFromLive(
      {
        averageProductivity: 72,
        perUser: [{ userId: "a", trackedMinutes: 90 }, null],
        perProject: [{ projectId: "p1", trackedMinutes: 90, projectTitle: "Apollo" }],
      },
      [{ date: "2026-09-28T00:00:00", trackedMinutes: 90, averageProductivity: 72, activityCount: 6, idleMinutes: 12 }, 3],
      [{ userId: "a", userName: "ana", date: "2026-09-28T00:00:00Z", trackedMinutes: 90, activityCount: 6, idleMinutes: 12 }],
    );
    expect(inputs.daily).toEqual([{ date: "2026-09-28", minutes: 90, activity: 72, measuredMinutes: undefined, idleMinutes: 12, sessions: 6 }]);
    expect(inputs.memberDays).toEqual([{ userId: "a", date: "2026-09-28", minutes: 90, name: "ana", idleMinutes: 12 }]);
    expect(inputs.projects).toEqual([{ projectId: "p1", minutes: 90, title: "Apollo", archived: false }]);
    expect(inputs.members).toEqual([{ userId: "a", minutes: 90 }]);
    expect(inputs.activity).toBe(72);

    expect(inputsFromLive(null, { not: "an array" }, undefined)).toEqual({ daily: [], memberDays: [], projects: [], members: [], activity: null });
  });

  it("the same blocks give the same report whichever way they arrive", () => {
    const rows = [
      row("a", "p1", "2026-09-28T09:00:00Z", 120, { activity: 80, idleMinutes: 10 }),
      row("b", "p1", "2026-09-29T09:00:00Z", 60, { activity: null, idleMinutes: 20 }),
    ];
    const fromBlocks = inputsFromAggregates(aggregateTracked(rows, utcDay));
    const log = buildDailyLog(keys("2026-09-28", 2), fromBlocks.daily);
    const s = summarizeLog(log, { activity: fromBlocks.activity });
    expect(s).toMatchObject({ totalMinutes: 180, averageActivity: 80, idleMinutes: 30, sessions: 2 });
    expect(log.map((r) => r.activity)).toEqual([80, null]);
  });
});

describe("CSV", () => {
  it("exports the daily log with idle time and a Total row; activity is blank on empty or unmeasured days", () => {
    const log = buildDailyLog(["2026-09-28", "2026-09-29", "2026-09-30"], [
      { date: "2026-09-28", minutes: 452, activity: 81, sessions: 7, idleMinutes: 30 },
      { date: "2026-09-30", minutes: 60, activity: null, sessions: 1, idleMinutes: 0 },
    ]);
    const rows = dailyLogCsvRows(log, summarizeLog(log));
    expect(DAILY_LOG_HEADERS).toEqual(["Date", "Weekday", "Hours (decimal)", "Hours (h:mm)", "Activity %", "Sessions", "Idle (decimal)", "Idle (h:mm)"]);
    expect(rows).toEqual([
      ["2026-09-28", "Mon", 7.53, "7:32", 81, 7, 0.5, "0:30"],
      ["2026-09-29", "Tue", 0, "0:00", "", 0, "", ""],
      ["2026-09-30", "Wed", 1, "1:00", "", 1, 0, "0:00"],
      ["Total", "", 8.53, "8:32", 81, 8, 0.5, "0:30"],
    ]);
  });

  it("an empty log still exports a zero Total row", () => {
    const rows = dailyLogCsvRows([], summarizeLog([]));
    expect(rows).toEqual([["Total", "", 0, "0:00", "", 0, "", ""]]);
  });

  it("the decimal Total is the sum of the rounded rows above it", () => {
    const log = buildDailyLog(keys("2026-09-28", 5), keys("2026-09-28", 5).map((date) => ({ date, minutes: 452 })));
    const rows = dailyLogCsvRows(log, summarizeLog(log));
    const body = rows.slice(0, -1).reduce((s, r) => s + Math.round(Number(r[2]) * 100), 0) / 100;
    expect(rows.at(-1)?.[2]).toBe(37.65);
    expect(rows.at(-1)?.[2]).toBe(body);
  });

  it("exports the timesheet matrix in decimal hours, totals adding up the rounded cells", () => {
    const m = buildTimesheetMatrix(["2026-09-28", "2026-09-29"], [
      { userId: "a", date: "2026-09-28", minutes: 90 },
      { userId: "b", date: "2026-09-29", minutes: 30 },
    ], (id) => id.toUpperCase());
    expect(matrixCsv(m)).toEqual({
      headers: ["Member", "2026-09-28 (Mon)", "2026-09-29 (Tue)", "Total (hours)"],
      rows: [
        ["A", 1.5, 0, 1.5],
        ["B", 0, 0.5, 0.5],
        ["Total", 1.5, 0.5, 2],
      ],
    });
    const thirds = buildTimesheetMatrix(["2026-09-28", "2026-09-29", "2026-09-30"], keys("2026-09-28", 3).map((date) => ({ userId: "a", date, minutes: 452, idleMinutes: 31 })), (id) => id);
    const csv = matrixCsv(thirds);
    expect(csv.headers.at(-1)).toBe("Idle (hours)");
    expect(csv.rows[0]).toEqual(["a", 7.53, 7.53, 7.53, 22.59, 1.55]);
    expect(csv.rows.at(-1)).toEqual(["Total", 7.53, 7.53, 7.53, 22.59, 1.55]);
  });
});

it("the target is a standard 8h day", () => {
  expect(TARGET_DAY_MINUTES).toBe(480);
});

/* ----------------------------- Periods ----------------------------- */

const ymd = (d: Date) => [d.getFullYear(), d.getMonth() + 1, d.getDate()];

describe("periods (browser-local)", () => {
  // Sunday 4 Oct 2026, mid-afternoon (local).
  const now = new Date(2026, 9, 4, 15, 30);

  it("the current week runs Monday to today; earlier weeks are whole", () => {
    expect(ymd(periodBounds("week", now).from)).toEqual([2026, 9, 28]);
    expect(ymd(periodBounds("week", now).to)).toEqual([2026, 10, 4]);
    const wed = new Date(2026, 9, 7, 9, 0);
    expect(periodSpan("week", wed).days).toEqual(["2026-10-05", "2026-10-06", "2026-10-07"]);
    expect(periodSpan("week", wed, -1).days).toEqual(keys("2026-09-28", 7));
  });

  it("7 and 30 days end today and step back by their own length", () => {
    expect(periodSpan("7d", now).days).toEqual(keys("2026-09-28", 7));
    expect(periodSpan("7d", now, -1).days).toEqual(keys("2026-09-21", 7));
    expect(periodSpan("30d", now).days).toEqual(keys("2026-09-05", 30));
    expect(periodSpan("30d", now, -1).days[29]).toBe("2026-09-04");
  });

  it("the current month runs to today; earlier months are whole, whatever their length", () => {
    expect(periodSpan("month", now).days).toEqual(keys("2026-10-01", 4));
    expect(periodSpan("month", now, -1).days).toEqual(keys("2026-09-01", 30));
    const march = new Date(2028, 2, 15);
    expect(periodSpan("month", march, -1).days).toHaveLength(29); // Feb 2028 is a leap month
  });

  it("never reaches into the future", () => {
    expect(periodSpan("week", now, 3).days).toEqual(periodSpan("week", now, 0).days);
    expect(periodSpan("7d", now, Number.NaN).days).toEqual(periodSpan("7d", now).days);
  });

  it("titles read naturally", () => {
    expect(periodTitle("week", now)).toBe("This week");
    expect(periodTitle("week", now, -1)).toBe("Last week");
    expect(periodTitle("week", now, -3)).toBe("3 weeks ago");
    expect(periodTitle("month", now)).toBe("This month");
    expect(periodTitle("month", now, -1)).toBe("Last month");
    expect(periodTitle("month", now, -3)).toBe("July 2026");
    expect(periodTitle("7d", now)).toBe("Last 7 days");
    expect(periodTitle("30d", now, -2)).toBe("30-day period");
  });

  it("a period in progress is compared with the same days of the one before", () => {
    const wed = new Date(2026, 9, 7, 9, 0);
    const cmp = comparisonSpan("week", wed, 0, 3);
    expect(cmp.days).toEqual(["2026-09-28", "2026-09-29", "2026-09-30"]);
    expect(cmp.from.toISOString()).toBe("2026-09-28T00:00:00.000Z");
    expect(cmp.to.toISOString()).toBe("2026-10-01T00:00:00.000Z");
    expect(comparisonSpan("7d", now, -1, 7).days).toEqual(keys("2026-09-14", 7));
    expect(comparisonSpan("7d", now, 0, 0).days).toEqual([]);
    expect(comparisonLabel("week", 0)).toBe("vs same days last week");
    expect(comparisonLabel("week", -2)).toBe("vs the week before");
    expect(comparisonLabel("month", 0)).toBe("vs same days last month");
    expect(comparisonLabel("7d", 0)).toBe("vs the 7 days before");
    expect(comparisonLabel("30d", 0)).toBe("vs the 30 days before");
  });

  it("changes are whole signed percents, and absent without a baseline", () => {
    expect(changeVs(120, 100)).toEqual({ percent: 20, direction: "up" });
    expect(changeVs(50, 100)).toEqual({ percent: -50, direction: "down" });
    expect(changeVs(100, 100)).toEqual({ percent: 0, direction: "flat" });
    expect(changeVs(100, 0)).toBeNull();
    expect(changeVs(100, null)).toBeNull();
    expect(changeVs(100, undefined)).toBeNull();
  });

  it("calendar weeks start on Monday and pad the edges", () => {
    // Thu 1 Oct … Sun 4 Oct 2026.
    expect(calendarWeeks(keys("2026-10-01", 4))).toEqual([[null, null, null, "2026-10-01", "2026-10-02", "2026-10-03", "2026-10-04"]]);
    const month = calendarWeeks(keys("2026-09-01", 30)); // Tue 1 … Wed 30 Sep
    expect(month).toHaveLength(5);
    expect(month[0][0]).toBeNull();
    expect(month[0][1]).toBe("2026-09-01");
    expect(month[4]).toEqual(["2026-09-28", "2026-09-29", "2026-09-30", null, null, null, null]);
    expect(month.every((w) => w.length === 7)).toBe(true);
    expect(calendarWeeks([])).toEqual([]);
  });

  it("formats hours three ways", () => {
    expect(formatHours(452, "duration")).toBe("7h 32m");
    expect(formatHours(452, "clock")).toBe("7:32");
    expect(formatHours(452, "decimal")).toBe("7.53");
    expect(formatHours(480, "decimal")).toBe("8.00");
    expect(formatHours(Number.NaN, "duration")).toBe("0m");
  });
});

describe("periods in the reporting zone", () => {
  // Sunday 4 Oct 2026, 09:30 UTC: 15:30 in Dhaka, 05:30 in New York.
  const now = new Date("2026-10-04T09:30:00Z");
  const preset = (p: PeriodPreset, offset = 0): PeriodSelection => ({ kind: "preset", preset: p, offset });

  it("a preset's days are whole local days, bounded by local midnights", () => {
    const dhaka = resolvePeriod(preset("7d"), now, "Asia/Dhaka");
    expect(dhaka.days).toEqual(keys("2026-09-28", 7));
    expect([dhaka.fromKey, dhaka.toKey]).toEqual(["2026-09-28", "2026-10-04"]);
    expect(dhaka.from.toISOString()).toBe("2026-09-27T18:00:00.000Z");
    expect(dhaka.to.toISOString()).toBe("2026-10-04T18:00:00.000Z");
    const ny = resolvePeriod(preset("7d"), now, "America/New_York");
    expect(ny.from.toISOString()).toBe("2026-09-28T04:00:00.000Z");
    expect(ny.to.toISOString()).toBe("2026-10-05T04:00:00.000Z");
  });

  it("'today' is read in the zone: a New Yorker's Sunday evening is already Monday in Dhaka", () => {
    const late = new Date("2026-10-04T23:00:00Z"); // Sun 19:00 in New York, Mon 05:00 in Dhaka
    expect(resolvePeriod(preset("week"), late, "America/New_York").days).toEqual(keys("2026-09-28", 7));
    expect(resolvePeriod(preset("week"), late, "Asia/Dhaka").days).toEqual(["2026-10-05"]);
  });

  it("a custom range is its own days, capped to the most recent the server accepts; an unreadable one falls back to 7 days", () => {
    const sept = resolvePeriod({ kind: "custom", from: "2026-09-01", to: "2026-09-30" }, now, "Asia/Dhaka");
    expect(sept.days).toHaveLength(30);
    expect(sept.from.toISOString()).toBe("2026-08-31T18:00:00.000Z");
    expect(sept.to.toISOString()).toBe("2026-09-30T18:00:00.000Z");
    const long = resolvePeriod({ kind: "custom", from: "2024-01-01", to: "2026-09-30" }, now, "UTC");
    expect(long.days).toHaveLength(MAX_REPORT_DAYS);
    expect([long.fromKey, long.toKey]).toEqual(["2025-09-30", "2026-09-30"]);
    expect(resolvePeriod({ kind: "custom", from: "nope", to: "2026-09-30" }, now, "UTC").days).toEqual(keys("2026-09-28", 7));
    expect(resolvePeriod({ kind: "custom", from: "2026-09-30", to: "2026-09-01" }, now, "UTC").days).toHaveLength(7);
  });

  it("a 31-day month is compared with all of a 28-day February — never with its own first days", () => {
    const april = new Date("2027-04-10T12:00:00Z");
    const march = resolvePeriod(preset("month", -1), april, "UTC");
    expect(march.days).toHaveLength(31);
    const cmp = comparisonFor(preset("month", -1), april, "UTC", march);
    expect([cmp.fromKey, cmp.toKey, cmp.days.length]).toEqual(["2027-02-01", "2027-02-28", 28]);
    expect(cmp.to.getTime()).toBe(march.from.getTime());
  });

  it("'this month' on the 31st meets all of a 30-day September, not the 1st of October", () => {
    const halloween = new Date("2026-10-31T12:00:00Z");
    const oct = resolvePeriod(preset("month"), halloween, "UTC");
    const cmp = comparisonFor(preset("month"), halloween, "UTC", oct);
    expect([cmp.fromKey, cmp.toKey]).toEqual(["2026-09-01", "2026-09-30"]);
  });

  it("a week in progress meets the same days of the week before; a custom range the days just before it", () => {
    const wed = new Date("2026-10-07T09:00:00Z");
    const week = resolvePeriod(preset("week"), wed, "UTC");
    expect(comparisonFor(preset("week"), wed, "UTC", week).days).toEqual(["2026-09-28", "2026-09-29", "2026-09-30"]);
    const custom: PeriodSelection = { kind: "custom", from: "2026-09-10", to: "2026-09-19" };
    const tenDays = resolvePeriod(custom, wed, "Asia/Dhaka");
    const before = comparisonFor(custom, wed, "Asia/Dhaka", tenDays);
    expect([before.fromKey, before.toKey]).toEqual(["2026-08-31", "2026-09-09"]);
    expect(before.to.getTime()).toBe(tenDays.from.getTime());
    expect(comparisonText(custom, tenDays.days.length)).toBe("vs the 10 days before");
    expect(comparisonText({ kind: "custom", from: "2026-09-10", to: "2026-09-10" }, 1)).toBe("vs the 1 day before");
    expect(comparisonText(preset("month"), 4)).toBe("vs same days last month");
  });

  it("never overlaps the period it is compared with, for any preset, step or zone", () => {
    const presets: PeriodPreset[] = ["week", "7d", "30d", "month"];
    const zones = ["UTC", "Asia/Dhaka", "America/New_York", "Pacific/Kiritimati"];
    for (let d = 0; d < 60; d += 3) {
      const instant = new Date(Date.UTC(2027, 0, 1 + d * 7, 13));
      for (const p of presets) {
        for (const offset of [0, -1, -2]) {
          for (const zone of zones) {
            const current = resolvePeriod(preset(p, offset), instant, zone);
            const cmp = comparisonFor(preset(p, offset), instant, zone, current);
            expect(cmp.days.length).toBeGreaterThan(0);
            expect(cmp.days.length).toBeLessThanOrEqual(current.days.length);
            expect(cmp.toKey < current.fromKey).toBe(true);
            expect(cmp.to.getTime()).toBeLessThanOrEqual(current.from.getTime());
          }
        }
      }
    }
  });

  it("names the selection and a month further back as the zone reads it", () => {
    expect(selectionTitle(preset("week"), now, "UTC")).toBe("This week");
    expect(selectionTitle({ kind: "custom", from: "2026-09-01", to: "2026-09-30" }, now, "UTC")).toBe("Custom range");
    expect(periodTitle("month", now, -3, "Asia/Dhaka")).toBe("July 2026");
    // On 31 Oct at 20:00 UTC it is already 1 Nov in Dhaka.
    const eve = new Date("2026-10-31T20:00:00Z");
    expect(periodTitle("month", eve, -2, "Asia/Dhaka")).toBe("September 2026");
    expect(periodTitle("month", eve, -2, "UTC")).toBe("August 2026");
  });

  it("an empty period has nothing to compare with", () => {
    const empty = { days: [], fromKey: "2026-10-01", toKey: "2026-10-01", from: new Date(0), to: new Date(0) };
    expect(comparisonFor(preset("7d"), now, "UTC", empty).days).toEqual([]);
  });
});

/* ----------------------------- What was worked on ----------------------------- */

describe("what was worked on (task notes)", () => {
  const blocks: NoteBlock[] = [
    { userId: "u1", day: "2026-09-28", minutes: 60, note: "Fix login bug" },
    { userId: "u1", day: "2026-09-28", minutes: 30, note: "  fix   LOGIN bug " }, // the same note, spelt differently
    { userId: "u2", day: "2026-09-29", minutes: 45, note: "Design review" },
    { userId: "u1", day: "2026-09-29", minutes: 20, note: null },
    { userId: "u2", day: "2026-09-29", minutes: 15, note: "" },
    { userId: "u2", day: "2026-09-28", minutes: 0, note: "Zero" },
  ];

  it("totals the time per note, most first, with the days and members behind it; no note comes last", () => {
    const rows = noteBreakdown(blocks);
    expect(rows).toEqual([
      { key: "fix login bug", note: "Fix login bug", minutes: 90, sessions: 2, days: ["2026-09-28"], members: ["u1"], share: 53 },
      { key: "design review", note: "Design review", minutes: 45, sessions: 1, days: ["2026-09-29"], members: ["u2"], share: 26 },
      { key: "", note: null, minutes: 35, sessions: 2, days: ["2026-09-29"], members: ["u1", "u2"], share: 21 },
    ]);
    expect(rows.reduce((s, r) => s + r.share, 0)).toBe(100);
    expect(noteBreakdown([])).toEqual([]);
  });

  it("groups the notes by day and by member, each section the sum of its notes", () => {
    expect(notesByDay(blocks).map((d) => [d.id, d.minutes, d.notes.map((n) => [n.note, n.minutes])])).toEqual([
      ["2026-09-28", 90, [["Fix login bug", 90]]],
      ["2026-09-29", 80, [["Design review", 45], [null, 35]]],
    ]);
    expect(notesByMember(blocks).map((m) => [m.id, m.minutes, m.notes.map((n) => [n.note, n.minutes])])).toEqual([
      ["u1", 110, [["Fix login bug", 90], [null, 20]]],
      ["u2", 60, [["Design review", 45], [null, 15]]],
    ]);
  });

  it("exports one CSV row per day, member and note, with a Total row that adds up", () => {
    expect(NOTES_CSV_HEADERS).toEqual(["Date", "Weekday", "Member", "Note", "Hours (decimal)", "Hours (h:mm)", "Sessions"]);
    expect(notesCsvRows(blocks, (id) => id.toUpperCase())).toEqual([
      ["2026-09-28", "Mon", "U1", "Fix login bug", 1.5, "1:30", 2],
      ["2026-09-29", "Tue", "U1", "", 0.33, "0:20", 1],
      ["2026-09-29", "Tue", "U2", "Design review", 0.75, "0:45", 1],
      ["2026-09-29", "Tue", "U2", "", 0.25, "0:15", 1],
      ["Total", "", "", "", 2.83, "2:50", 5],
    ]);
  });
});

/* ----------------------------- Highlights & summary ----------------------------- */

describe("highlights and the copied summary", () => {
  const days = keys("2026-09-28", 7); // Mon … Sun
  const fmt = (m: number) => formatHM(m);
  const projects = projectBreakdown(
    [
      { projectId: "p1", minutes: 360 },
      { projectId: "p2", minutes: 120 },
    ],
    (id) => ({ title: id === "p1" ? "Apollo" : "Zeus" }),
  );

  it("one person: busiest day, main project, full days and weekend work", () => {
    const log = buildDailyLog(days, [
      { date: "2026-09-28", minutes: 500, activity: 80 },
      { date: "2026-10-03", minutes: 60, activity: 50 },
    ]);
    const matrix = buildTimesheetMatrix(days, [], (id) => id);
    const out = buildHighlights({ summary: summarizeLog(log), projects, matrix, team: false, weekendDays: 2, fmt });
    expect(out.map((h) => h.kind)).toEqual(["busiest", "project", "fullDays", "weekend"]);
    expect(out[0]).toMatchObject({ value: "Mon, 28 Sep", detail: "8:20 tracked" });
    expect(out[1]).toMatchObject({ label: "Main project", value: "Apollo", detail: "75% of hours · 6:00" });
    expect(out[2]).toMatchObject({ value: "1 of 2", detail: "worked days reached 8 hours" });
    expect(out[3]).toMatchObject({ value: "1:00", detail: "11% of all hours" });
  });

  it("a team names its top contributor; no weekend in the period says nothing about weekends", () => {
    const weekdays = keys("2026-09-28", 5);
    const log = buildDailyLog(weekdays, [{ date: "2026-09-28", minutes: 300 }]);
    const matrix = buildTimesheetMatrix(
      weekdays,
      [
        { userId: "a", date: "2026-09-28", minutes: 200 },
        { userId: "b", date: "2026-09-28", minutes: 100 },
      ],
      (id) => (id === "a" ? "Rina" : "Omar"),
    );
    const out = buildHighlights({ summary: summarizeLog(log), projects: [], matrix, team: true, weekendDays: 0, fmt });
    expect(out.map((h) => h.kind)).toEqual(["busiest", "member"]);
    expect(out[1]).toMatchObject({ value: "Rina", detail: "3:20 over 1 day" });
  });

  it("weekends kept free read as None; an empty period has no highlights", () => {
    const log = buildDailyLog(days, [{ date: "2026-09-29", minutes: 120 }]);
    const matrix = buildTimesheetMatrix(days, [], (id) => id);
    const out = buildHighlights({ summary: summarizeLog(log), projects: [], matrix, team: false, weekendDays: 2, fmt });
    expect(out.find((h) => h.kind === "weekend")).toMatchObject({ value: "None" });
    expect(out.find((h) => h.kind === "fullDays")).toMatchObject({ value: "0 of 1", detail: "no day reached 8 hours" });
    const empty = summarizeLog(buildDailyLog(days, []));
    expect(buildHighlights({ summary: empty, projects, matrix, team: false, weekendDays: 2, fmt })).toEqual([]);
  });

  it("the copied summary reads as a short plain-text digest", () => {
    const log = buildDailyLog(days, [
      { date: "2026-09-28", minutes: 300, activity: 80 },
      { date: "2026-09-29", minutes: 180, activity: 60 },
    ]);
    const text = summaryText({
      workspace: "Acme Studio",
      period: "Mon, 28 Sep – Sun, 4 Oct 2026",
      scope: "All projects · All members",
      summary: summarizeLog(log),
      projects,
      change: { percent: 12, direction: "up" },
      comparison: "vs the 7 days before",
      perMember: null,
      fmt: (m) => formatHours(m, "duration"),
    });
    expect(text).toBe(
      [
        "Work Report — Acme Studio",
        "Mon, 28 Sep – Sun, 4 Oct 2026 · All projects · All members",
        "",
        "Total worked: 8h on 2 of 7 days",
        "Average: 4h per day worked",
        "Activity: 73% (Fair)",
        "Busiest day: Mon, 28 Sep (5h)",
        "Change: +12% vs the 7 days before",
        "",
        "By project:",
        "• Apollo — 6h (75%)",
        "• Zeus — 2h (25%)",
      ].join("\n"),
    );
  });

  it("says when activity wasn't measured, adds the idle time and what was worked on", () => {
    const log = buildDailyLog(days, [{ date: "2026-09-28", minutes: 480, activity: null, idleMinutes: 60 }]);
    const notes = noteBreakdown([
      { userId: "a", day: "2026-09-28", minutes: 300, note: "Checkout flow" },
      { userId: "a", day: "2026-09-28", minutes: 180, note: null },
    ]);
    const text = summaryText({
      workspace: "W",
      period: "P",
      scope: "S",
      summary: summarizeLog(log),
      projects: [],
      change: null,
      comparison: "",
      perMember: null,
      fmt: (m) => formatHours(m, "duration"),
      notes,
    });
    expect(text).toContain("Activity: not measured");
    expect(text).toContain("Idle: 1h (13% of tracked time)");
    expect(text.endsWith("Worked on:\n• Checkout flow — 5h")).toBe(true);
  });

  it("a team digest averages per member-day; a decrease has no plus sign; an empty period says so", () => {
    const log = buildDailyLog(days, [{ date: "2026-09-28", minutes: 300, activity: 80 }]);
    const team = summaryText({
      workspace: "W",
      period: "P",
      scope: "S",
      summary: summarizeLog(log),
      projects: [],
      change: { percent: -8, direction: "down" },
      comparison: "vs the week before",
      perMember: { minutes: 150, memberDays: 2 },
      fmt,
    });
    expect(team).toContain("Average: 2:30 per member per day worked");
    expect(team).toContain("Change: -8% vs the week before");
    expect(team).not.toContain("By project");
    const none = summaryText({
      workspace: "W",
      period: "P",
      scope: "S",
      summary: summarizeLog(buildDailyLog(days, [])),
      projects: [],
      change: null,
      comparison: "",
      perMember: null,
      fmt,
    });
    expect(none).toBe("Work Report — W\nP · S\n\nNo time was tracked in this period.");
  });
});
