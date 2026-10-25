// Boundary + branch behaviour of the report figures.
//
// report-math.test.ts covers each report's happy path; this file pins down
// what a well-formed payload never exercises:
//   • figures that are present but unreadable (they read as unknown, never 0),
//     and offsets or month keys that name nothing,
//   • blocks the demo derivations can't place, given in any order,
//   • rows a server splits in parts — a day, an app, a device or a heartbeat
//     reported twice — and how the parts fold together,
//   • the order of ties (projects, pay lines, members, apps, devices), so two
//     renders of the same data list it the same way,
//   • versions that are missing parts or digits.
import { describe, expect, it } from "vitest";
import {
  appUsageFromBlocks,
  attendanceFromBlocks,
  compareVersions,
  dayBars,
  mergeDevices,
  payrollByMember,
  periodTitleFor,
  readAppUsage,
  readAttendance,
  readSummary,
  resolveReportPeriod,
  summarizeBlocks,
  topProjectByMember,
  withRoster,
  type DailyFigures,
  type MemberProjectFigures,
  type PayrollLine,
  type ReportBlock,
} from "./report-math";

/* A Tuesday afternoon (UTC). */
const NOW = new Date("2026-07-14T15:30:00.000Z");
const ms = (iso: string) => Date.parse(iso);

/* ----------------------------- Figures & periods ----------------------------- */

describe("figures that are present but unreadable", () => {
  it("an idle time or activity that can't be read is unknown, never 0 — while readable numeric text is read", () => {
    const summary = readSummary({ totalTrackedMinutes: "90", totalIdleMinutes: "n/a", averageProductivity: "high", perUser: [], perProject: [] });
    expect(summary).toMatchObject({ trackedMinutes: 90, idleMinutes: null, averageProductivity: null });
  });
});

describe("period titles and offsets that name nothing", () => {
  it("an unreadable offset is the current period", () => {
    expect(periodTitleFor("week", Number.NaN, "2026-07-13")).toBe("This week");
    const stepped = resolveReportPeriod({ preset: "week", offset: Number.NaN, now: NOW, timeZone: "UTC" });
    expect(stepped).toEqual(resolveReportPeriod({ preset: "week", offset: 0, now: NOW, timeZone: "UTC" }));
    expect(stepped.title).toBe("This week");
  });

  it("a month further back whose key names no month is titled 'Month'", () => {
    expect(periodTitleFor("month", -3, "2026-13-01")).toBe("Month");
  });
});

/* ----------------------------- Demo derivations ----------------------------- */

describe("summarizeBlocks — blocks it can't place, in any order", () => {
  const block = (startedAt: string, endedAt: string, over: Partial<ReportBlock> = {}): ReportBlock => ({
    userId: "u1",
    projectId: "p1",
    startedAt,
    endedAt,
    productivity: 50,
    ...over,
  });

  it("skips blocks without a readable end or ids; a block whose start is unreadable is a session without time", () => {
    const { summary } = summarizeBlocks(
      [
        block("2026-07-14T09:00:00Z", "2026-07-14T10:00:00Z"),
        block("garbage", "2026-07-14T11:00:00Z"),
        block("2026-07-14T12:00:00Z", "garbage"),
        block("2026-07-14T13:00:00Z", "2026-07-14T14:00:00Z", { userId: 7 as unknown as string }),
        block("2026-07-14T13:00:00Z", "2026-07-14T14:00:00Z", { projectId: null as unknown as string }),
      ],
      "UTC",
    );
    expect(summary).toMatchObject({ activityCount: 2, trackedMinutes: 60 });
    expect(summary.perUser).toEqual([expect.objectContaining({ userId: "u1", activityCount: 2, trackedMinutes: 60 })]);
  });

  it("keeps the daily series oldest first whatever order the blocks come in", () => {
    const { daily } = summarizeBlocks(
      [
        block("2026-07-13T09:00:00Z", "2026-07-13T09:30:00Z"),
        block("2026-07-15T09:00:00Z", "2026-07-15T09:45:00Z"),
        block("2026-07-14T09:00:00Z", "2026-07-14T10:00:00Z"),
        block("2026-07-12T09:00:00Z", "2026-07-12T09:15:00Z"),
      ],
      "UTC",
    );
    expect(daily.map((d) => [d.date, d.trackedMinutes])).toEqual([
      ["2026-07-12", 15],
      ["2026-07-13", 30],
      ["2026-07-14", 60],
      ["2026-07-15", 45],
    ]);
  });
});

describe("dayBars — a day reported in parts", () => {
  it("sums the parts; when only one part knows its idle time, the other's counts as none", () => {
    const rows: DailyFigures[] = [
      { date: "2026-07-14", activityCount: 1, trackedMinutes: 60, idleMinutes: 10, averageProductivity: 50 },
      { date: "2026-07-14", activityCount: 2, trackedMinutes: 30, idleMinutes: null, averageProductivity: null },
    ];
    const [bar] = dayBars(["2026-07-14"], rows);
    // Activity is weighted over the measured part only; idle is the known part.
    expect(bar).toMatchObject({ minutes: 90, sessions: 3, idle: 10, active: 45, other: 35, activity: 50 });
  });
});

describe("topProjectByMember — ties", () => {
  const row = (userId: string, projectId: string, trackedMinutes: number): MemberProjectFigures => ({
    userId,
    projectId,
    trackedMinutes,
    activityCount: 1,
    idleMinutes: null,
    averageProductivity: null,
  });

  it("ignores rows without time, and a tie goes to the earlier project id in whatever order the rows come", () => {
    const top = topProjectByMember([row("u1", "p2", 60), row("u1", "p1", 60), row("u1", "p3", 60), row("u2", "p9", 0), row("u2", "p8", 15)]);
    expect(top.get("u1")).toEqual({ projectId: "p1", minutes: 60 });
    expect(top.get("u2")).toEqual({ projectId: "p8", minutes: 15 });
  });
});

/* ----------------------------- Payroll ----------------------------- */

describe("payroll — ties", () => {
  const line = (userId: string, projectId: string, minutes: number, amount: number, rate = 0): PayrollLine => ({
    userId,
    userName: null,
    projectId,
    projectTitle: null,
    minutes,
    amount,
    rate,
    activityCount: 1,
  });

  it("lines that pay the same are ordered by time; members with the same pay and time by id", () => {
    const members = payrollByMember([line("u2", "p2", 0, 0), line("u2", "p1", 60, 0), line("u1", "p1", 30, 0), line("u1", "p2", 30, 0)]);
    expect(members.map((m) => [m.userId, m.minutes, m.amount, m.unpricedMinutes])).toEqual([
      ["u1", 60, 0, 60],
      ["u2", 60, 0, 60],
    ]);
    expect(members[1].lines.map((l) => l.projectId)).toEqual(["p1", "p2"]);
  });
});

/* ----------------------------- Apps ----------------------------- */

describe("apps — rows folded together", () => {
  it("readAppUsage adds up idle time when only some of an app's rows report it, and lists apps with equal time by name", () => {
    const lines = readAppUsage([
      { appName: "chrome.exe", trackedMinutes: 30, idleMinutes: null, activityCount: 1, userCount: 1 },
      { appName: "Chrome.exe", trackedMinutes: 20, idleMinutes: 5, activityCount: 2, userCount: 2 },
      { appName: "slack.exe", trackedMinutes: 25, idleMinutes: 4, activityCount: 1, userCount: 1 },
      { appName: "Slack", trackedMinutes: 25, idleMinutes: null, activityCount: 1, userCount: 3 },
      { appName: "Zoom", trackedMinutes: 50, activityCount: 1, userCount: 1 },
    ]);
    expect(lines.map((l) => [l.name, l.minutes, l.idleMinutes, l.activityCount, l.userCount])).toEqual([
      ["Google Chrome", 50, 5, 3, 2],
      ["Slack", 50, 4, 2, 3],
      ["Zoom", 50, null, 1, 1],
    ]);
  });

  it("appUsageFromBlocks: a block without focused windows credits no time, though its running programs count toward reach", () => {
    const lines = appUsageFromBlocks([
      { userId: "u1", startedAt: "2026-07-14T09:00:00Z", endedAt: "2026-07-14T09:10:00Z", activeWindows: null, runningPrograms: [{ appName: "Spotify" }, { appName: "  " }, {}] },
      { userId: "u2", startedAt: "2026-07-14T09:00:00Z", endedAt: "2026-07-14T09:10:00Z", runningPrograms: [{ appName: "spotify.exe" }] },
    ]);
    expect(lines.map((l) => [l.name, l.minutes, l.activityCount, l.userCount])).toEqual([["Spotify", 0, 2, 2]]);
  });
});

/* ----------------------------- Attendance ----------------------------- */

describe("attendance — what can't be read", () => {
  it("readAttendance reads anything that isn't a report as no rows", () => {
    expect(readAttendance(null)).toEqual({ lines: [], detailed: false });
    expect(readAttendance("rows")).toEqual({ lines: [], detailed: false });
  });

  it("withRoster lists a roster member the source left out, unnamed when the roster has no name", () => {
    expect(withRoster([], [{ userId: "u9" }])).toEqual([expect.objectContaining({ userId: "u9", name: null, workedMinutes: 0, averageStart: null })]);
  });

  it("attendanceFromBlocks skips a block it can't place, and keeps a block's own idle minutes over the activity estimate", () => {
    const lines = attendanceFromBlocks(
      [
        { userId: "u1", projectId: "p1", startedAt: "2026-07-14T09:00:00Z", endedAt: "2026-07-14T10:00:00Z", productivity: 90, idleMinutes: 12 },
        { userId: "u1", projectId: "p1", startedAt: "garbage", endedAt: "2026-07-14T11:00:00Z", productivity: 90 },
      ],
      "UTC",
    );
    // From the activity alone the hour would read 6 idle minutes; the block measured 12.
    expect(lines).toEqual([expect.objectContaining({ userId: "u1", workedMinutes: 60, idleMinutes: 12, averageStart: 540, averageEnd: 600 })]);
  });
});

/* ----------------------------- Devices ----------------------------- */

describe("mergeDevices — a device or a heartbeat reported more than once", () => {
  it("adds up a device listed twice, keeps its latest readable activity, and skips rows without a member", () => {
    const lines = mergeDevices(
      [
        { userId: "u1", deviceId: "D1", trackedMinutes: 30, lastActivityAt: "2026-07-14T08:00:00Z" },
        { userId: "u1", deviceId: "d1", trackedMinutes: 20, lastActivityAt: "2026-07-14T09:00:00Z" }, // later: wins
        { userId: "u1", deviceId: "D1", trackedMinutes: 10, lastActivityAt: "2026-07-14T07:00:00Z" }, // earlier: ignored
        { userId: "u1", deviceId: "D1", trackedMinutes: 5, lastActivityAt: "soon" }, // unreadable: ignored
        { userId: "u2", deviceId: "D2", trackedMinutes: 15 }, // no activity time yet
        { userId: "u2", deviceId: "D2", trackedMinutes: 15, lastActivityAt: "2026-07-14T06:00:00Z" },
        { deviceId: "D3", trackedMinutes: 99 },
      ],
      null,
      NOW,
    );
    expect(lines.map((l) => [l.key, l.trackedMinutes, l.lastActivityAt])).toEqual([
      ["u1:d1", 65, ms("2026-07-14T09:00:00Z")],
      ["u2:d2", 30, ms("2026-07-14T06:00:00Z")],
    ]);
  });

  it("keeps a device's newest heartbeat: an older or unreadable one never replaces it, and a beat without a status reads as stopped", () => {
    const lines = mergeDevices(
      null,
      [
        { userId: "u1", deviceId: "D1", status: "tracking", lastSeenAt: "2026-07-14T15:29:00Z" },
        { userId: "u1", deviceId: "D1", status: "idle", lastSeenAt: "2026-07-14T15:00:00Z" }, // older
        { userId: "u1", deviceId: "D1", status: "paused", lastSeenAt: "garbage" }, // unreadable
        { userId: "u2", deviceId: "D2", status: "paused", lastSeenAt: "garbage" },
        { userId: "u2", deviceId: "D2", status: "idle", lastSeenAt: "2026-07-14T15:20:00Z" }, // readable beats unreadable
        { userId: "u3", deviceId: "D3", lastSeenAt: "2026-07-14T15:10:00Z" },
        { userId: "u4", deviceId: "  ", status: "tracking", lastSeenAt: "2026-07-14T15:29:00Z" }, // no device
        { deviceId: "D5", status: "tracking", lastSeenAt: "2026-07-14T15:29:00Z" }, // no member
      ],
      NOW,
    );
    expect(lines.map((l) => [l.key, l.presence?.status, l.presence?.online, l.presence?.lastSeenAt])).toEqual([
      ["u1:d1", "tracking", true, ms("2026-07-14T15:29:00Z")],
      ["u2:d2", "idle", false, ms("2026-07-14T15:20:00Z")],
      ["u3:d3", "stopped", false, ms("2026-07-14T15:10:00Z")],
    ]);
  });

  it("orders devices online first, then by time tracked, then by latest activity, then by key", () => {
    const lines = mergeDevices(
      [
        { userId: "u1", deviceId: "B", trackedMinutes: 30, lastActivityAt: "2026-07-14T08:00:00Z" },
        { userId: "u1", deviceId: "C", trackedMinutes: 30 },
        { userId: "u1", deviceId: "A", trackedMinutes: 30, lastActivityAt: "2026-07-14T09:00:00Z" },
        { userId: "u1", deviceId: "E", trackedMinutes: 30 },
        { userId: "u1", deviceId: "D", trackedMinutes: 60 },
      ],
      null,
      NOW,
    );
    expect(lines.map((l) => l.key)).toEqual(["u1:d", "u1:a", "u1:b", "u1:c", "u1:e"]);
  });
});

describe("compareVersions — versions missing parts or digits", () => {
  it("a version without digits reads as 0.0.0, and a missing part counts as 0 on either side", () => {
    expect(compareVersions("dev", "0.0.1")).toBe(-1);
    expect(compareVersions("dev", "0")).toBe(0);
    expect(compareVersions("1.2", "1.2.1")).toBe(-1);
    expect(compareVersions("1.2.1", "1.2")).toBe(1);
  });
});
