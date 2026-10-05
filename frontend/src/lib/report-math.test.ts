import { describe, it, expect } from "vitest";
import {
  ALL,
  MAX_PERIOD_DAYS,
  activeDaysByUser,
  activityBreakdown,
  appGroupOf,
  appGroupTotals,
  appUsageFromBlocks,
  asArray,
  attendanceFromBlocks,
  attendanceRows,
  averageClock,
  compareVersions,
  dayBars,
  decimalHoursTotal,
  displayedTotal,
  elapsedDays,
  formatClock,
  formatMoney,
  formatRate,
  grantedPoliciesOf,
  idleFromActivity,
  latestAgentVersion,
  legacyPayrollLines,
  memberName,
  mergeDevices,
  payrollByMember,
  payrollLinesFromBlocks,
  payrollTotals,
  periodTitleFor,
  projectCosts,
  ratesByMember,
  readAppUsage,
  readAttendance,
  readDaily,
  readMemberDays,
  readPayroll,
  readSummary,
  reportAccessFor,
  reportQuery,
  resolveReportPeriod,
  safePercent,
  serverDayKey,
  sumMoney,
  summarizeBlocks,
  toCents,
  topProjectByMember,
  utcInstant,
  withRoster,
  type AttendanceLine,
  type DailyFigures,
  type MemberRate,
  type ReportBlock,
} from "./report-math";
import { appColor } from "./apps";
import { canEditActivityNote } from "./api-contract";
import { formatHours } from "./work-report";

/* A Tuesday afternoon (UTC): 21:30 in Dhaka (UTC+6, no DST), 11:30 in New York (EDT, UTC−4). */
const NOW = new Date("2026-07-14T15:30:00.000Z");

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
    expect([...none.keys()]).toEqual(["From", "To"]);
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
    expect(safePercent(5, -1)).toBe(0);
    expect(safePercent(Number.NaN, 10)).toBe(0);
    expect(safePercent(1, Number.POSITIVE_INFINITY)).toBe(0);
  });

  it("memberName prefers the roster name and falls back to a short id label", () => {
    expect(memberName("0f7c9a2e-1111-2222-3333-444455556666", { name: "Rina Das" })).toBe("Rina Das");
    expect(memberName("0f7c9a2e-1111-2222-3333-444455556666", { name: "   " })).toBe("Member 0f7c9a2e");
    expect(memberName("abc")).toBe("Member abc");
  });
});

/* ----------------------------- Periods ----------------------------- */

describe("resolveReportPeriod — whole days of the reporting zone", () => {
  it("'week' runs Monday up to today, bounded by local midnights (Dhaka, UTC+6)", () => {
    const p = resolveReportPeriod({ preset: "week", now: NOW, timeZone: "Asia/Dhaka" });
    expect(p.days).toEqual(["2026-07-13", "2026-07-14"]);
    expect(p.from.toISOString()).toBe("2026-07-12T18:00:00.000Z");
    expect(p.to.toISOString()).toBe("2026-07-14T18:00:00.000Z");
    expect(p).toMatchObject({ fromKey: "2026-07-13", toKey: "2026-07-14", today: "2026-07-14", title: "This week", label: "13 Jul – 14 Jul 2026", custom: false });
  });

  it("'7d' is exactly seven whole days ending today — never a partial first day", () => {
    const p = resolveReportPeriod({ preset: "7d", now: NOW, timeZone: "America/New_York" });
    expect(p.days).toHaveLength(7);
    expect(p.days[0]).toBe("2026-07-08");
    expect(p.from.toISOString()).toBe("2026-07-08T04:00:00.000Z");
    expect(p.to.toISOString()).toBe("2026-07-15T04:00:00.000Z");
    expect(p.title).toBe("Last 7 days");
  });

  it("steps back through whole months, naming them after the first two", () => {
    const last = resolveReportPeriod({ preset: "month", offset: -1, now: NOW, timeZone: "Asia/Dhaka" });
    expect(last.days).toHaveLength(30);
    expect([last.fromKey, last.toKey, last.title]).toEqual(["2026-06-01", "2026-06-30", "Last month"]);
    expect(last.from.toISOString()).toBe("2026-05-31T18:00:00.000Z");
    expect(resolveReportPeriod({ preset: "month", offset: -2, now: NOW, timeZone: "Asia/Dhaka" }).title).toBe("May 2026");
    // There is no future to report: a positive offset is the current period.
    expect(resolveReportPeriod({ preset: "month", offset: 3, now: NOW, timeZone: "UTC" }).title).toBe("This month");
  });

  it("a custom range is its own days, titled by its dates", () => {
    const p = resolveReportPeriod({ preset: "custom", range: { from: "2026-06-01", to: "2026-06-15" }, now: NOW, timeZone: "UTC" });
    expect(p.days).toHaveLength(15);
    expect(p).toMatchObject({ custom: true, title: "1 Jun – 15 Jun 2026", label: "1 Jun – 15 Jun 2026" });
    expect(p.from.toISOString()).toBe("2026-06-01T00:00:00.000Z");
    expect(p.to.toISOString()).toBe("2026-06-16T00:00:00.000Z");
  });

  it("keeps the most recent days of a range longer than the server accepts", () => {
    const p = resolveReportPeriod({ preset: "custom", range: { from: "2025-01-01", to: "2026-07-14" }, now: NOW, timeZone: "UTC" });
    expect(p.days).toHaveLength(MAX_PERIOD_DAYS);
    expect(p.toKey).toBe("2026-07-14");
  });

  it("falls back to the last 7 days when the custom range is unusable", () => {
    const p = resolveReportPeriod({ preset: "custom", range: { from: "2026-07-10", to: "2026-07-01" }, now: NOW, timeZone: "UTC" });
    expect(p.custom).toBe(false);
    expect(p.days).toHaveLength(7);
  });

  it("elapsedDays leaves out the days still ahead", () => {
    expect(elapsedDays({ days: ["2026-07-13", "2026-07-14", "2026-07-15"], today: "2026-07-14" })).toEqual(["2026-07-13", "2026-07-14"]);
  });

  it("periodTitleFor reads like the Work Report's titles", () => {
    expect(periodTitleFor("week", -1, "2026-07-06")).toBe("Last week");
    expect(periodTitleFor("week", -3, "2026-06-22")).toBe("3 weeks ago");
    expect(periodTitleFor("30d", -1, "2026-05-16")).toBe("30-day period");
    expect(periodTitleFor("7d", -2, "2026-06-24")).toBe("7-day period");
  });
});

describe("serverDayKey / utcInstant", () => {
  it("reads a zone-less server date as the day it names", () => {
    expect(serverDayKey("2026-07-14T00:00:00")).toBe("2026-07-14");
    expect(serverDayKey("2026-07-14T00:00:00Z")).toBe("2026-07-14");
    expect(serverDayKey("2026-02-30T00:00:00")).toBeNull();
    expect(serverDayKey(42)).toBeNull();
  });

  it("reads an instant without a zone suffix as UTC", () => {
    expect(utcInstant("2026-07-14T10:00:00")).toBe(Date.parse("2026-07-14T10:00:00Z"));
    expect(utcInstant("2026-07-14T10:00:00+02:00")).toBe(Date.parse("2026-07-14T08:00:00Z"));
    expect(utcInstant("never")).toBeNull();
    expect(utcInstant("")).toBeNull();
  });
});

/* ----------------------------- Hours ----------------------------- */

describe("displayedTotal — a Total row always adds up", () => {
  it("decimal hours: the sum of the rounded rows, not the rounded sum", () => {
    const days = Array<number>(5).fill(452); // 7h 32m each → 7.53
    expect(formatHours(displayedTotal(days, "decimal"), "decimal")).toBe("37.65");
    // Rounding the total from minutes would have read 37.67.
    expect(formatHours(days.reduce((s, m) => s + m, 0), "decimal")).toBe("37.67");
    expect(decimalHoursTotal(days)).toBe(37.65);
  });

  it("minutes: whole minutes per row, then summed", () => {
    expect(displayedTotal([10.4, 10.4], "duration")).toBe(20);
    expect(displayedTotal([10.4, 10.4], "clock")).toBe(20);
    expect(displayedTotal([Number.NaN, -5, "30"], "duration")).toBe(30);
  });
});

/* ----------------------------- Activity & idle ----------------------------- */

describe("activityBreakdown", () => {
  it("splits tracked time into active, idle and other", () => {
    expect(activityBreakdown(600, 80, 100)).toEqual({ tracked: 600, active: 480, idle: 100, other: 20, activity: 80 });
  });

  it("keeps the measured idle minutes; the derived active time gives way", () => {
    expect(activityBreakdown(600, 80, 200)).toEqual({ tracked: 600, active: 400, idle: 200, other: 0, activity: 80 });
  });

  it("never invents a figure that wasn't measured or reported", () => {
    expect(activityBreakdown(600, null, 100)).toEqual({ tracked: 600, active: null, idle: 100, other: 500, activity: null });
    expect(activityBreakdown(600, 75, undefined)).toEqual({ tracked: 600, active: 450, idle: null, other: 150, activity: 75 });
  });

  it("survives garbage without NaN", () => {
    expect(activityBreakdown(Number.NaN, 50, null)).toEqual({ tracked: 0, active: 0, idle: null, other: 0, activity: 50 });
    expect(activityBreakdown("90", "50", "30")).toEqual({ tracked: 90, active: 45, idle: 30, other: 15, activity: 50 });
    expect(activityBreakdown(100, 140, -5)).toEqual({ tracked: 100, active: 100, idle: 0, other: 0, activity: 100 });
  });

  it("idleFromActivity: the minute buckets a block's activity % leaves without input", () => {
    expect(idleFromActivity(60, 75)).toBe(15);
    expect(idleFromActivity(60, null)).toBe(0);
    expect(idleFromActivity(Number.NaN, 50)).toBe(0);
  });
});

/* ----------------------------- Reporting shapes ----------------------------- */

describe("readSummary", () => {
  it("reads an older server's summary without inventing the new fields", () => {
    const s = readSummary({
      totalActivities: 3,
      totalTrackedMinutes: 120.5,
      averageProductivity: 0,
      perUser: [{ userId: "a", activityCount: 3, trackedMinutes: 120.5, averageProductivity: 0 }],
      perProject: [{ projectId: "p", activityCount: 3, trackedMinutes: 120.5, averageProductivity: 0 }],
    });
    expect(s).toMatchObject({ activityCount: 3, trackedMinutes: 120.5, idleMinutes: null, averageProductivity: 0, perUserProject: null });
    expect(s.perUser[0]).toEqual({ userId: "a", name: null, activityCount: 3, trackedMinutes: 120.5, idleMinutes: null, averageProductivity: 0 });
    expect(s.perProject[0]).toMatchObject({ projectId: "p", title: null, isArchived: null });
  });

  it("reads the contract-v2 fields", () => {
    const s = readSummary({
      totalActivities: 1,
      totalTrackedMinutes: 5,
      totalIdleMinutes: 1,
      averageProductivity: null,
      perUser: [{ userId: "a", userName: "rina", name: "Rina Das", activityCount: 1, trackedMinutes: 5, idleMinutes: 1, averageProductivity: null }],
      perProject: [{ projectId: "p", projectTitle: "Apollo", isArchived: true, activityCount: 1, trackedMinutes: 5, idleMinutes: 1, averageProductivity: null }],
      perUserProject: [{ userId: "a", projectId: "p", activityCount: 1, trackedMinutes: 5, idleMinutes: 1, averageProductivity: 50, amount: 0, hourlyRate: 0 }, { userId: 7 }],
    });
    expect(s.averageProductivity).toBeNull();
    expect(s.idleMinutes).toBe(1);
    expect(s.perUser[0].name).toBe("Rina Das");
    expect(s.perProject[0]).toMatchObject({ title: "Apollo", isArchived: true });
    expect(s.perUserProject).toEqual([{ userId: "a", projectId: "p", activityCount: 1, trackedMinutes: 5, idleMinutes: 1, averageProductivity: 50 }]);
  });

  it("reads garbage as an empty summary", () => {
    expect(readSummary(null)).toEqual({ activityCount: 0, trackedMinutes: 0, idleMinutes: null, averageProductivity: null, perUser: [], perProject: [], perUserProject: null });
    expect(readSummary({ perUser: [null, { userId: 3 }, "x"] }).perUser).toEqual([]);
  });
});

describe("readDaily / readMemberDays", () => {
  it("keys each row by its day and skips unreadable rows", () => {
    expect(
      readDaily([
        { date: "2026-07-14T00:00:00", activityCount: 2, trackedMinutes: 30, idleMinutes: 3, averageProductivity: 70 },
        { date: "nope", trackedMinutes: 10 },
        null,
      ]),
    ).toEqual([{ date: "2026-07-14", activityCount: 2, trackedMinutes: 30, idleMinutes: 3, averageProductivity: 70 }]);
  });

  it("reads member-days with the server's name", () => {
    expect(readMemberDays([{ userId: "a", userName: "rina", date: "2026-07-14T00:00:00Z", trackedMinutes: 30, activityCount: 2 }, { date: "2026-07-14" }])).toEqual([
      { userId: "a", name: "rina", date: "2026-07-14", trackedMinutes: 30, activityCount: 2, idleMinutes: null },
    ]);
  });
});

describe("summarizeBlocks — the demo's /reporting/* shapes", () => {
  const blocks: ReportBlock[] = [
    // Ends 00:30 on 14 Jul in Dhaka although it is still 13 Jul in UTC.
    { userId: "a", projectId: "p1", startedAt: "2026-07-13T18:00:00Z", endedAt: "2026-07-13T18:30:00Z", productivity: 80 },
    // Input not measured: counted in hours, left out of the activity average.
    { userId: "a", projectId: "p2", startedAt: "2026-07-14T03:00:00Z", endedAt: "2026-07-14T04:00:00Z", productivity: null },
    { userId: "b", projectId: "p1", startedAt: "2026-07-12T10:00:00Z", endedAt: "2026-07-12T10:20:00Z", productivity: 50, idleMinutes: 15 },
  ];
  const { summary, daily, memberDays } = summarizeBlocks(blocks, "Asia/Dhaka");

  it("totals hours and weights activity over measured minutes only", () => {
    expect(summary).toMatchObject({ activityCount: 3, trackedMinutes: 110, idleMinutes: 21, averageProductivity: 68 });
  });

  it("breaks the totals down per member, per project and per member × project", () => {
    expect(summary.perUser.find((u) => u.userId === "a")).toMatchObject({ trackedMinutes: 90, idleMinutes: 6, averageProductivity: 80, activityCount: 2 });
    expect(summary.perProject.find((p) => p.projectId === "p2")).toMatchObject({ trackedMinutes: 60, idleMinutes: 0, averageProductivity: null });
    expect(summary.perUserProject).toHaveLength(3);
    expect(topProjectByMember(summary.perUserProject ?? []).get("a")).toEqual({ projectId: "p2", minutes: 60 });
  });

  it("books each block to the zone's day of its end", () => {
    expect(daily.map((d) => [d.date, d.trackedMinutes])).toEqual([
      ["2026-07-12", 20],
      ["2026-07-14", 90],
    ]);
    expect(memberDays.find((d) => d.userId === "a")).toMatchObject({ date: "2026-07-14", trackedMinutes: 90, activityCount: 2 });
  });
});

describe("dayBars", () => {
  const rows: DailyFigures[] = [
    { date: "2026-07-12", activityCount: 2, trackedMinutes: 120, idleMinutes: 30, averageProductivity: 50 },
    { date: "2026-07-14", activityCount: 1, trackedMinutes: 10, idleMinutes: null, averageProductivity: null },
    { date: "2026-07-15", activityCount: 0, trackedMinutes: 0, idleMinutes: 0, averageProductivity: null },
  ];
  const bars = dayBars(["2026-07-11", "2026-07-12", "2026-07-13"], rows);

  it("has a bar for every day, and keeps a day outside the period that carries time", () => {
    expect(bars.map((b) => b.date)).toEqual(["2026-07-11", "2026-07-12", "2026-07-13", "2026-07-14"]);
    expect(bars.map((b) => b.weekend)).toEqual([true, true, false, false]);
  });

  it("splits each day and leaves unmeasured figures empty", () => {
    expect(bars[1]).toMatchObject({ minutes: 120, active: 60, idle: 30, other: 30, activity: 50, sessions: 2 });
    expect(bars[3]).toMatchObject({ minutes: 10, active: null, idle: null, other: 10, activity: null });
    expect(bars[0]).toMatchObject({ minutes: 0, active: null, idle: null, other: 0, sessions: 0 });
  });
});

describe("activeDaysByUser", () => {
  it("counts distinct days with time per member", () => {
    const days = activeDaysByUser([
      { userId: "a", date: "2030-01-01T00:00:00Z" },
      { userId: "a", date: "2030-01-01T00:00:00" },
      { userId: "a", date: "2030-01-02", trackedMinutes: 0 },
      { userId: "b", date: "2030-01-02", trackedMinutes: 5 },
      null as unknown as { userId: string; date: string },
    ]);
    expect(days.get("a")).toBe(1);
    expect(days.get("b")).toBe(1);
  });
});

/* ----------------------------- Money & payroll ----------------------------- */

describe("money", () => {
  it("always shows two decimals in one locale", () => {
    expect(formatMoney(1234.5)).toBe("$1,234.50");
    expect(formatMoney(1234)).toBe("$1,234.00");
    expect(formatMoney(-0.001)).toBe("$0.00");
    expect(formatMoney(Number.NaN)).toBe("$0.00");
    expect(formatMoney(10, "EUR")).toBe("€10.00");
    expect(formatMoney(10, "not-a-currency")).toBe("$10.00");
    expect(formatRate(40)).toBe("$40.00/hr");
  });

  it("adds amounts in whole cents", () => {
    expect(sumMoney([0.1, 0.2])).toBe(0.3);
    expect(sumMoney([0.1, 0.2, 0.7])).toBe(1);
    expect(toCents(Number.POSITIVE_INFINITY)).toBe(0);
  });
});

describe("ratesByMember", () => {
  it("treats missing/invalid rates as unpriced and keeps the higher duplicate", () => {
    const rates = ratesByMember([
      { userId: "a", hourlyRate: 40 },
      { userId: "b", hourlyRate: null },
      { userId: "d", hourlyRate: -5 },
      { userId: "a", hourlyRate: 30 },
      null as unknown as MemberRate,
    ]);
    expect([...rates]).toEqual([
      ["a", 40],
      ["b", 0],
      ["d", 0],
    ]);
  });
});

describe("payroll", () => {
  const lines = readPayroll({
    rows: [
      { userId: "a", userName: "rina", projectId: "p1", projectTitle: "Apollo", activityCount: 4, trackedMinutes: 120, amount: 100.004, hourlyRate: 50 },
      { userId: "a", projectId: "p2", trackedMinutes: 60, amount: 20, hourlyRate: 20 },
      { userId: "b", projectId: "p1", trackedMinutes: 30, amount: 0, hourlyRate: 0 },
      { projectId: "orphan" },
      null,
    ],
    totalMinutes: 210,
    totalAmount: 120,
  });

  it("reads the server's lines, amounts to the cent", () => {
    expect(lines).toHaveLength(3);
    expect(lines[0]).toEqual({ userId: "a", userName: "rina", projectId: "p1", projectTitle: "Apollo", minutes: 120, amount: 100, rate: 50, activityCount: 4 });
    expect(readPayroll([{ userId: "x", projectId: "y", trackedMinutes: 6, amount: 1, hourlyRate: 10 }])).toHaveLength(1);
    expect(readPayroll(null)).toEqual([]);
  });

  it("pays a member the sum of their lines; the rate is per paid hour", () => {
    const [a, b] = payrollByMember(lines);
    expect(a).toMatchObject({ userId: "a", userName: "rina", minutes: 180, amount: 120, rate: 40, unpricedMinutes: 0 });
    expect(a.lines.map((l) => l.projectId)).toEqual(["p1", "p2"]);
    expect(b).toMatchObject({ userId: "b", minutes: 30, amount: 0, rate: 0, unpricedMinutes: 30 });
  });

  it("totals are the cent-exact sum of the rows", () => {
    expect(payrollTotals(lines)).toEqual({ minutes: 210, amount: 120, unpricedMinutes: 30, members: 2, rate: 40 });
    const pennies = [0.1, 0.2, 0.7].map((amount, i) => ({ ...lines[0], projectId: `p${i}`, amount }));
    expect(payrollTotals(pennies).amount).toBe(1);
    expect(projectCosts(lines)).toEqual(new Map([["p1", 100], ["p2", 20]]));
  });

  it("legacy pricing: each project's hours at that project's current rate — not the highest rate × all hours", () => {
    const legacy = legacyPayrollLines([
      { projectId: "p1", projectTitle: "Apollo", perUser: [{ userId: "a", trackedMinutes: 120 }, { userId: "c", trackedMinutes: 0 }], rates: new Map([["a", 50]]) },
      { projectId: "p2", perUser: [{ userId: "a", trackedMinutes: 60 }], rates: new Map() },
    ]);
    expect(legacy.map((l) => [l.projectId, l.minutes, l.amount, l.rate])).toEqual([
      ["p1", 120, 100, 50],
      ["p2", 60, 0, 0],
    ]);
    expect(payrollTotals(legacy).amount).toBe(100);
  });

  it("prices blocks with a rate lookup (the demo's payroll)", () => {
    const priced = payrollLinesFromBlocks(
      [
        { userId: "a", projectId: "p1", startedAt: "2026-07-14T09:00:00Z", endedAt: "2026-07-14T10:40:00Z" },
        { userId: "a", projectId: "p1", startedAt: "2026-07-14T11:00:00Z", endedAt: "2026-07-14T11:00:00Z" },
        { userId: "b", projectId: "p1", startedAt: "2026-07-14T09:00:00Z", endedAt: "2026-07-14T09:30:00Z" },
      ],
      (userId) => (userId === "a" ? 33.33 : Number.NaN),
    );
    expect(priced).toEqual([
      { userId: "a", userName: null, projectId: "p1", projectTitle: null, minutes: 100, amount: 55.55, rate: 33.33, activityCount: 2 },
      { userId: "b", userName: null, projectId: "p1", projectTitle: null, minutes: 30, amount: 0, rate: 0, activityCount: 1 },
    ]);
  });
});

/* ----------------------------- Apps ----------------------------- */

describe("apps", () => {
  it("files an app under its category; unknown apps are unclassified, not 'neutral'", () => {
    expect(appGroupOf("Code.exe")).toBe("productive");
    expect(appGroupOf("chrome.exe")).toBe("neutral");
    expect(appGroupOf("YouTube")).toBe("unproductive");
    expect(appGroupOf("mytool.exe")).toBe("unclassified");
    expect(appGroupOf(null)).toBe("unclassified");
  });

  it("folds an older server's process names into one app per canonical name", () => {
    const lines = readAppUsage([
      { appName: "chrome.exe", trackedMinutes: 30, activityCount: 3, userCount: 2 },
      { appName: "Chrome.exe", trackedMinutes: 10, activityCount: 1, userCount: 1 },
      { appName: "code.exe", displayName: "Visual Studio Code", trackedMinutes: 60, idleMinutes: 5, activityCount: 4, userCount: 1 },
      { appName: "TextInputHost.exe", trackedMinutes: 0, activityCount: 9, userCount: 3 },
      null,
    ]);
    expect(lines.map((l) => [l.name, l.minutes, l.group])).toEqual([
      ["Visual Studio Code", 60, "productive"],
      ["Google Chrome", 40, "neutral"],
      ["TextInputHost", 0, "unclassified"],
    ]);
    expect(lines[1]).toMatchObject({ idleMinutes: null, activityCount: 4, userCount: 2, color: appColor("chrome.exe") });
    expect(lines[0].idleMinutes).toBe(5);
  });

  it("credits blocks as the server does: per-window time, scaled to the block; a legacy block to its focused app", () => {
    const lines = appUsageFromBlocks([
      {
        userId: "a",
        startedAt: "2026-07-14T10:00:00Z",
        endedAt: "2026-07-14T10:10:00Z",
        activeWindows: [
          { appName: "Code.exe", seconds: 480 },
          { appName: "chrome.exe", seconds: 240 },
        ],
        runningPrograms: [{ appName: "slack.exe" }],
      },
      { userId: "b", startedAt: "2026-07-14T11:00:00Z", endedAt: "2026-07-14T11:30:00Z", activeWindows: [{ appName: "Figma", seconds: 0 }] },
    ]);
    expect(lines.map((l) => [l.name, l.minutes, l.activityCount, l.userCount])).toEqual([
      ["Figma", 30, 1, 1],
      ["Visual Studio Code", 6.67, 1, 1],
      ["Google Chrome", 3.33, 1, 1],
      ["Slack", 0, 1, 1],
    ]);
    const totals = appGroupTotals(lines);
    expect(totals.productive).toBeCloseTo(36.67, 5);
    expect(totals.neutral).toBeCloseTo(3.33, 5);
    expect(totals.total).toBeCloseTo(40, 5);
  });
});

/* ----------------------------- Attendance ----------------------------- */

describe("attendance", () => {
  // Sat 11, Sun 12, Mon 13, Tue 14, Wed 15 July 2026.
  const days = ["2026-07-11", "2026-07-12", "2026-07-13", "2026-07-14", "2026-07-15"];

  it("reads the contract-v2 report (and tells an older one apart)", () => {
    const v2 = readAttendance({
      days: ["2026-07-13T00:00:00"],
      rows: [
        {
          userId: "a",
          userName: "rina",
          daysPresent: 2,
          daysAbsent: 1,
          weekendDaysWorked: 1,
          workedMinutes: 600,
          idleMinutes: 30,
          averageStartMinutes: 545,
          averageEndMinutes: 1030,
          presentDays: ["2026-07-13T00:00:00", "2026-07-14T00:00:00", "2026-07-11T00:00:00", "garbage"],
        },
      ],
    });
    expect(v2.detailed).toBe(true);
    expect(v2.lines[0]).toMatchObject({ userId: "a", name: "rina", workedMinutes: 600, idleMinutes: 30, averageStart: 545, averageEnd: 1030, weekendDaysReported: 1 });
    expect([...v2.lines[0].presentDays].sort()).toEqual(["2026-07-11", "2026-07-13", "2026-07-14"]);

    const old = readAttendance({ days: [], rows: [{ userId: "b", daysPresent: 1, daysAbsent: 0, workedMinutes: 60, presentDays: ["2026-07-13T00:00:00Z"] }] });
    expect(old.detailed).toBe(false);
    expect(old.lines[0]).toMatchObject({ idleMinutes: null, averageStart: null, weekendDaysReported: null });
  });

  it("present + absent equals the working days; weekend work is counted apart", () => {
    const line: AttendanceLine = {
      userId: "a",
      name: null,
      presentDays: new Set(["2026-07-11", "2026-07-13", "2026-07-14"]),
      workedMinutes: 600.4,
      idleMinutes: null,
      averageStart: 545,
      averageEnd: 1030,
      weekendDaysReported: null,
    };
    const [row, absentee] = attendanceRows(withRoster([line], [{ userId: "a" }, { userId: "b", name: "Omar" }]), days);
    expect(row).toMatchObject({ present: 2, absent: 1, weekendDays: 1, workedMinutes: 600 });
    // A member who tracked nothing is still on the sheet: absent every working day.
    expect(absentee).toMatchObject({ userId: "b", name: "Omar", present: 0, absent: 3, weekendDays: 0, workedMinutes: 0 });
  });

  it("derives presence, start and finish from blocks in the zone", () => {
    const lines = attendanceFromBlocks(
      [
        { userId: "a", projectId: "p", startedAt: "2026-07-13T03:00:00Z", endedAt: "2026-07-13T04:00:00Z", productivity: 100 },
        { userId: "a", projectId: "p", startedAt: "2026-07-13T10:00:00Z", endedAt: "2026-07-13T11:30:00Z", productivity: 100 },
        { userId: "a", projectId: "p", startedAt: "2026-07-14T02:30:00Z", endedAt: "2026-07-14T03:00:00Z", productivity: 50 },
      ],
      "Asia/Dhaka",
    );
    expect(lines).toHaveLength(1);
    // 09:00 and 08:30 local → 08:45; 17:30 and 09:00 → 13:15.
    expect(lines[0]).toMatchObject({ workedMinutes: 180, idleMinutes: 15, averageStart: 525, averageEnd: 795 });
    expect([...lines[0].presentDays].sort()).toEqual(["2026-07-13", "2026-07-14"]);
  });

  it("averages the team's start and finish by days present", () => {
    const base = { name: null, presentDays: new Set<string>(), workedMinutes: 0, idleMinutes: null };
    expect(
      averageClock([
        { ...base, userId: "a", present: 3, absent: 0, weekendDays: 0, averageStart: 540, averageEnd: 1020 },
        { ...base, userId: "b", present: 1, absent: 2, weekendDays: 0, averageStart: 600, averageEnd: 1080 },
        { ...base, userId: "c", present: 0, absent: 3, weekendDays: 0, averageStart: null, averageEnd: null },
      ]),
    ).toEqual({ start: 555, end: 1035 });
    expect(averageClock([])).toEqual({ start: null, end: null });
  });

  it("formats a clock time, past midnight too", () => {
    expect(formatClock(545)).toBe("09:05");
    expect(formatClock(0)).toBe("00:00");
    expect(formatClock(1470)).toBe("00:30 (+1)");
    expect(formatClock(null)).toBe("—");
    expect(formatClock(-5)).toBe("—");
  });
});

/* ----------------------------- Devices ----------------------------- */

describe("devices", () => {
  const now = new Date("2026-07-14T11:00:00Z");
  const devices = [
    { userId: "a", userName: "rina", deviceId: "D1", deviceName: "RINA-PC", platform: "windows", osVersion: "Windows 11", agentVersion: "0.2.0", lastActivityAt: "2026-07-14T10:00:00", trackedMinutes: 300 },
    { userId: "a", deviceId: null, trackedMinutes: 60, lastActivityAt: "2026-07-10T10:00:00Z" },
    { userId: "b", deviceId: "D2", platform: "MacOS", agentVersion: "0.3.0", trackedMinutes: 100, lastActivityAt: "2026-07-14T09:00:00Z" },
  ];
  const presence = [
    { userId: "a", deviceId: "d1", deviceName: "RINA-LAPTOP", platform: "windows", agentVersion: "0.3.0", status: "tracking", lastSeenAt: "2026-07-14T10:59:00Z", projectTitle: "Apollo", currentApp: "Visual Studio Code" },
    { userId: "c", deviceId: "D3", status: "idle", lastSeenAt: "2026-07-14T09:00:00Z" },
  ];

  it("merges the period's devices with live presence, newest agent facts first", () => {
    const lines = mergeDevices(devices, presence, now);
    expect(lines.map((l) => l.key)).toEqual(["a:d1", "b:d2", "a:legacy", "c:d3"]);
    expect(lines[0]).toMatchObject({ deviceName: "RINA-LAPTOP", agentVersion: "0.3.0", trackedMinutes: 300, lastActivityAt: Date.parse("2026-07-14T10:00:00Z") });
    expect(lines[0].presence).toMatchObject({ status: "tracking", online: true, projectTitle: "Apollo" });
    expect(lines[1]).toMatchObject({ platform: "macos", presence: null });
    expect(lines[2]).toMatchObject({ deviceId: null, trackedMinutes: 60 });
    // Running but tracked nothing in the period; last heard from 2h ago.
    expect(lines[3]).toMatchObject({ trackedMinutes: 0, presence: { online: false, status: "idle" } });
  });

  it("leaves out presence-only devices for a report narrowed to a project", () => {
    expect(mergeDevices(devices, presence, now, { knownOnly: true }).map((l) => l.key)).toEqual(["a:d1", "b:d2", "a:legacy"]);
    expect(mergeDevices(null, null, now)).toEqual([]);
  });

  it("compares agent versions numerically", () => {
    expect(compareVersions("0.10.0", "0.9.3")).toBe(1);
    expect(compareVersions("1.2", "1.2.0")).toBe(0);
    expect(compareVersions("0.2.0-beta", "0.2.0")).toBe(0);
    expect(compareVersions("v1", "1.0.1")).toBe(-1);
    expect(latestAgentVersion([{ agentVersion: "0.2.0" }, { agentVersion: null }, { agentVersion: "0.10.1" }, { agentVersion: "dev" }])).toBe("0.10.1");
    expect(latestAgentVersion([])).toBeNull();
  });
});

/* ----------------------------- Access ----------------------------- */

describe("report access", () => {
  it("reads the granted policies of the application configuration", () => {
    const policies = { "Tracker.Team.Manage": true };
    expect(grantedPoliciesOf({ auth: { grantedPolicies: policies } })).toBe(policies);
    expect(grantedPoliciesOf({})).toBeNull();
    expect(grantedPoliciesOf(null)).toBeNull();
    expect(grantedPoliciesOf({ auth: { grantedPolicies: [] } })).toBeNull();
  });

  it("follows the permissions when known, the role otherwise", () => {
    expect(reportAccessFor("admin", { "Tracker.Activities.ViewAll": true })).toEqual({ viewAll: true, manageTeam: false });
    expect(reportAccessFor("admin", { "Tracker.Team.Manage": true })).toEqual({ viewAll: false, manageTeam: true });
    expect(reportAccessFor("owner", {})).toEqual({ viewAll: false, manageTeam: false });
    expect(reportAccessFor("owner", null)).toEqual({ viewAll: true, manageTeam: true });
    expect(reportAccessFor("worker", undefined)).toEqual({ viewAll: false, manageTeam: false });
  });

  it("gates note edits on the Team.Manage grant, not the derived admin role", () => {
    const now = new Date("2026-10-05T12:00:00Z");
    const other = { isOwner: false, startedAt: "2026-10-05T09:00:00Z", now };
    const stale = { isOwner: true, startedAt: "2026-08-01T09:00:00Z", now };
    // A live "admin" by ViewAll alone (a supervisor) may not edit others' or stale notes.
    const supervisor = reportAccessFor("admin", { "Tracker.Activities.ViewAll": true }).manageTeam;
    expect(canEditActivityNote({ ...other, canManageTeam: supervisor })).toBe(false);
    expect(canEditActivityNote({ ...stale, canManageTeam: supervisor })).toBe(false);
    const manager = reportAccessFor("admin", { "Tracker.Team.Manage": true }).manageTeam;
    expect(canEditActivityNote({ ...other, canManageTeam: manager })).toBe(true);
  });
});
