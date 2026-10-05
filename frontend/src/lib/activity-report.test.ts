import { describe, expect, it } from "vitest";
import {
  DEFAULT_SESSION_DESCRIPTION,
  NO_SESSION_DETAILS,
  blockSeconds,
  clockSkewNote,
  demoSessionDetails,
  deviceSummary,
  endReasonTone,
  formatBlockLength,
  formatClock,
  formatClockShort,
  formatSessionDate,
  formatTimeRange,
  friendlyAppName,
  groupSessionsByDay,
  groupWindowsByApp,
  hasScreenshot,
  mainAppOf,
  notableEndReason,
  readSessionDetails,
  sessionDetailsById,
  sessionHeadline,
  shareOfBlock,
  summarizeTimeline,
  timelineSlots,
  timelineTickStep,
} from "./activity-report";
import type { WindowInfo } from "./types";

const win = (appName: string, seconds: number, extra: Partial<WindowInfo> = {}): WindowInfo => ({
  appName,
  windowTitle: `${appName} window`,
  seconds,
  ...extra,
});

describe("blockSeconds / clock formatting", () => {
  it("measures the block in whole seconds and never goes negative or NaN", () => {
    expect(blockSeconds({ startedAt: "2026-09-23T14:00:00.000Z", endedAt: "2026-09-23T14:04:58.400Z" })).toBe(298);
    expect(blockSeconds({ startedAt: "2026-09-23T14:05:00.000Z", endedAt: "2026-09-23T14:00:00.000Z" })).toBe(0);
    expect(blockSeconds({ startedAt: "garbage", endedAt: "2026-09-23T14:00:00.000Z" })).toBe(0);
  });

  it("formats clock times in the viewer's zone and survives bad input", () => {
    const iso = new Date(2026, 8, 23, 14, 7).toISOString(); // local 14:07
    expect(formatClockShort(iso)).toBe("2:07");
    expect(formatClockShort(new Date(2026, 8, 23, 0, 5).toISOString())).toBe("12:05");
    expect(formatClock(iso)).toMatch(/02:07\s?PM/);
    expect(formatClock("nope")).toBe("—");
    expect(formatClockShort("nope")).toBe("—");
    expect(formatTimeRange({ startedAt: iso, endedAt: iso })).toBe(`${formatClock(iso)} – ${formatClock(iso)}`);
  });
});

describe("groupWindowsByApp", () => {
  it("groups windows per app (case-insensitive), most-focused app and window first", () => {
    const groups = groupWindowsByApp([
      win("chrome.exe", 100, { windowTitle: "Docs", keyboardHits: 5, mouseClicks: 9 }),
      win("code.exe", 240, { windowTitle: "main.rs", keyboardHits: 310, mouseClicks: 12 }),
      win("Chrome.exe", 190, { windowTitle: "Mail", keyboardHits: 1 }),
    ]);
    expect(groups.map((g) => [g.appName, g.seconds, g.keyboardHits, g.mouseClicks])).toEqual([
      ["chrome.exe", 290, 6, 9],
      ["code.exe", 240, 310, 12],
    ]);
    expect(groups[0].windows.map((w) => w.windowTitle)).toEqual(["Mail", "Docs"]);
    expect(groups.every((g) => !g.credited)).toBe(true);
  });

  it("leaves input undefined (not 0) when no window reported it, and carries the legacy credit flag", () => {
    const [legacy] = groupWindowsByApp([win("Code", 1800, { credited: true })]);
    expect(legacy).toMatchObject({ appName: "Code", seconds: 1800, credited: true });
    expect(legacy.keyboardHits).toBeUndefined();
    expect(legacy.mouseClicks).toBeUndefined();
  });

  it("keeps the agent's order on ties and skips blank app names", () => {
    const groups = groupWindowsByApp([win("B", 0), win("A", 0), win("  ", 50)]);
    expect(groups.map((g) => g.appName)).toEqual(["B", "A"]);
    expect(groupWindowsByApp([])).toEqual([]);
  });

  it("mainAppOf names the app with the most summed focus", () => {
    expect(mainAppOf([win("code", 200), win("chrome", 150), win("chrome", 150)])).toBe("chrome");
    expect(mainAppOf([])).toBeUndefined();
  });

  it("shareOfBlock is a clamped whole percent", () => {
    expect(shareOfBlock(150, 300)).toBe(50);
    expect(shareOfBlock(400, 300)).toBe(100);
    expect(shareOfBlock(10, 0)).toBe(0);
    expect(shareOfBlock(0, 300)).toBe(0);
  });
});

describe("timeline slots", () => {
  const startedAt = "2026-09-23T14:00:00.000Z";

  it("returns [] without a timeline (legacy and demo rows)", () => {
    expect(timelineSlots({ startedAt })).toEqual([]);
    expect(timelineSlots({ startedAt, timeline: [] })).toEqual([]);
    expect(timelineSlots({ startedAt: "garbage", timeline: [{ minute: 0, keyboardHits: 1, mouseClicks: 0, active: true }] })).toEqual([]);
  });

  it("places each minute on the clock and fills a skipped minute as idle 'no data'", () => {
    const slots = timelineSlots({
      startedAt,
      timeline: [
        { minute: 0, keyboardHits: 12, mouseClicks: 3, active: true, appName: "code.exe" },
        { minute: 2, keyboardHits: 0, mouseClicks: 0, active: false },
      ],
    });
    expect(slots).toEqual([
      { minute: 0, keyboardHits: 12, mouseClicks: 3, active: true, appName: "code.exe", at: "2026-09-23T14:00:00.000Z", total: 15, missing: false },
      { minute: 1, keyboardHits: 0, mouseClicks: 0, active: false, at: "2026-09-23T14:01:00.000Z", total: 0, missing: true },
      { minute: 2, keyboardHits: 0, mouseClicks: 0, active: false, at: "2026-09-23T14:02:00.000Z", total: 0, missing: false },
    ]);
  });

  it("summarizes active minutes, totals and the busiest minute", () => {
    const slots = timelineSlots({
      startedAt,
      timeline: [
        { minute: 0, keyboardHits: 12, mouseClicks: 3, active: true },
        { minute: 1, keyboardHits: 40, mouseClicks: 2, active: true },
        { minute: 2, keyboardHits: 0, mouseClicks: 0, active: false },
        { minute: 3, keyboardHits: 0, mouseClicks: 0, active: true },
      ],
    });
    const summary = summarizeTimeline(slots);
    expect(summary).toMatchObject({ minutes: 4, activeMinutes: 3, keyboardHits: 52, mouseClicks: 5 });
    expect(summary.peak?.minute).toBe(1);
    expect(summarizeTimeline([])).toEqual({ minutes: 0, activeMinutes: 0, keyboardHits: 0, mouseClicks: 0, peak: undefined });
  });

  it("thins axis labels so a long block never crowds the axis", () => {
    expect(timelineTickStep(5)).toBe(1);
    expect(timelineTickStep(10)).toBe(2);
    expect(timelineTickStep(30)).toBe(5);
    expect(timelineTickStep(60)).toBe(10);
    expect(timelineTickStep(10_000)).toBe(60);
  });
});

describe("friendlyAppName / formatBlockLength", () => {
  it("names known executables the way people know them, and tidies the rest", () => {
    expect(friendlyAppName("msedge.exe")).toBe("Microsoft Edge");
    // The one canonical name every page and the backend's reports use (contract §8).
    expect(friendlyAppName("Code.EXE")).toBe("Visual Studio Code");
    expect(friendlyAppName("Google Chrome")).toBe("Google Chrome");
    expect(friendlyAppName("zoom.us")).toBe("Zoom");
    expect(friendlyAppName("  WindowsTerminal.exe ")).toBe("Windows Terminal");
    expect(friendlyAppName("Antigravity IDE.exe")).toBe("Antigravity IDE");
    expect(friendlyAppName("myapp.exe")).toBe("Myapp");
    expect(friendlyAppName("Rider")).toBe("Rider");
    expect(friendlyAppName("")).toBe("");
  });

  it("reads a block's length at a glance", () => {
    expect(formatBlockLength(0)).toBe("0m");
    expect(formatBlockLength(45)).toBe("45s");
    expect(formatBlockLength(298)).toBe("5m");
    expect(formatBlockLength(3_600)).toBe("1h");
    expect(formatBlockLength(3_900)).toBe("1h 5m");
    expect(formatBlockLength(Number.NaN)).toBe("0m");
  });
});

describe("sessionHeadline", () => {
  it("prefers a written description over the window title", () => {
    expect(sessionHeadline({ description: "Fix login bug", activeWindows: [win("code.exe", 60)] })).toBe("Fix login bug");
  });

  it("falls back to the main app's most-focused window, without the browser's own suffix", () => {
    const activeWindows = [
      win("msedge.exe", 300, { windowTitle: "WhatsApp - Personal - Microsoft\u200b Edge" }),
      win("code.exe", 120, { windowTitle: "main.rs" }),
    ];
    expect(sessionHeadline({ description: DEFAULT_SESSION_DESCRIPTION, activeWindows })).toBe("WhatsApp - Personal");
  });

  it("keeps the placeholder when there is nothing better", () => {
    expect(sessionHeadline({ description: DEFAULT_SESSION_DESCRIPTION, activeWindows: [] })).toBe(DEFAULT_SESSION_DESCRIPTION);
    expect(sessionHeadline({ description: "", activeWindows: [win("x.exe", 5, { windowTitle: "—" })] })).toBe(DEFAULT_SESSION_DESCRIPTION);
  });
});

describe("hasScreenshot", () => {
  it("counts a full capture or a thumbnail, not a webcam frame", () => {
    expect(hasScreenshot({ captures: [{ id: "1", kind: "thumb" }] })).toBe(true);
    expect(hasScreenshot({ captures: [{ id: "1", kind: "screen" }] })).toBe(true);
    expect(hasScreenshot({ captures: [{ id: "1", kind: "webcam" }] })).toBe(false);
    expect(hasScreenshot({})).toBe(false);
  });
});

describe("groupSessionsByDay", () => {
  const at = (y: number, mo: number, d: number, h: number, min = 0) => new Date(y, mo - 1, d, h, min).toISOString();
  const block = (start: string, minutes: number, productivity: number) => ({
    startedAt: start,
    endedAt: new Date(Date.parse(start) + minutes * 60_000).toISOString(),
    productivity,
  });
  const now = new Date(2026, 8, 30, 18, 0); // Wed 30 Sep 2026, local

  it("groups by local day in the given order, with duration-weighted activity", () => {
    const items = [
      block(at(2026, 9, 30, 17), 10, 80),
      block(at(2026, 9, 30, 16), 30, 40),
      block(at(2026, 9, 29, 23, 55), 10, 100),
      block(at(2026, 9, 27, 9), 5, 0),
    ];
    const days = groupSessionsByDay(items, now);
    expect(days.map((d) => d.label)).toEqual(["Today", "Yesterday", "Sun, 27 Sep"]);
    expect(days[0].items).toEqual([items[0], items[1]]);
    expect(days[0].seconds).toBe(40 * 60);
    expect(days[0].activity).toBe(50); // (10×80 + 30×40) / 40
    expect(days[1].items).toEqual([items[2]]); // a block that starts before midnight belongs to that day
    expect(days[2].activity).toBe(0);
  });

  it("keeps an oldest-first order too, and skips unusable timestamps", () => {
    const garbage = { startedAt: "garbage", endedAt: "garbage", productivity: 50 };
    const items = [block(at(2026, 9, 29, 8), 5, 50), garbage, block(at(2026, 9, 30, 8), 5, 50)];
    expect(groupSessionsByDay(items, now).map((d) => d.label)).toEqual(["Yesterday", "Today"]);
    expect(groupSessionsByDay([], now)).toEqual([]);
  });
});

describe("clock and day formatting in the reporting zone", () => {
  // 2026-09-23 18:30 UTC is 00:30 the next day in Dhaka (UTC+6) and 14:30 in New York (UTC−4).
  const iso = "2026-09-23T18:30:00.000Z";

  it("reads clock times in the zone it is given", () => {
    expect(formatClock(iso, "UTC")).toMatch(/06:30\s?PM/);
    expect(formatClock(iso, "Asia/Dhaka")).toMatch(/12:30\s?AM/);
    expect(formatClock(iso, "America/New_York")).toMatch(/02:30\s?PM/);
    expect(formatClockShort(iso, "Asia/Dhaka")).toBe("12:30");
    expect(formatClockShort(iso, "Asia/Kathmandu")).toBe("12:15"); // UTC+5:45
    expect(formatTimeRange({ startedAt: iso, endedAt: "2026-09-23T18:40:00.000Z" }, "UTC")).toMatch(/06:30\s?PM – 06:40\s?PM/);
    expect(formatClock("nope", "Asia/Dhaka")).toBe("—");
  });

  it("names the calendar day a block started in that zone", () => {
    expect(formatSessionDate(iso, "UTC")).toBe("Wed, 23 Sep");
    expect(formatSessionDate(iso, "Asia/Dhaka")).toBe("Thu, 24 Sep");
    expect(formatSessionDate(iso, "Asia/Dhaka", { year: true })).toBe("Thu, 24 Sep 2026");
    expect(formatSessionDate("garbage", "UTC")).toBe("—");
  });

  it("groups a block on the day it started in the zone, not in UTC", () => {
    const block = { startedAt: iso, endedAt: "2026-09-23T18:40:00.000Z", productivity: 60 };
    const now = new Date("2026-09-24T06:00:00.000Z");
    expect(groupSessionsByDay([block], now, { timeZone: "Asia/Dhaka" }).map((d) => [d.key, d.label])).toEqual([["2026-09-24", "Today"]]);
    expect(groupSessionsByDay([block], now, { timeZone: "UTC" }).map((d) => [d.key, d.label])).toEqual([["2026-09-23", "Yesterday"]]);
  });

  it("averages only the measured blocks, says when none was, and sums the reported idle time", () => {
    const now = new Date("2026-09-24T12:00:00.000Z");
    const measured = { id: "m", startedAt: "2026-09-24T09:00:00.000Z", endedAt: "2026-09-24T09:10:00.000Z", productivity: 80 };
    const unmeasured = { id: "u", startedAt: "2026-09-24T09:10:00.000Z", endedAt: "2026-09-24T09:40:00.000Z", productivity: 0 };
    const idle: Record<string, number | null> = { m: 120, u: 99_999 };
    const [day] = groupSessionsByDay([measured, unmeasured], now, {
      timeZone: "UTC",
      isMeasured: (b) => b.id !== "u",
      idleSecondsOf: (b) => idle[b.id],
    });
    expect(day.seconds).toBe(40 * 60);
    expect(day.activity).toBe(80); // the unmeasured 0 doesn't drag the day down
    expect(day.idleSeconds).toBe(120 + 30 * 60); // a reported idle time never exceeds its block
    const [none] = groupSessionsByDay([unmeasured], now, { timeZone: "UTC", isMeasured: () => false });
    expect(none.activity).toBeNull();
    expect(none.idleSeconds).toBe(0);
  });
});

describe("groupWindowsByApp — one app, whatever the platform calls it", () => {
  it("merges the Windows executable and the macOS name of the same app and names it canonically", () => {
    const groups = groupWindowsByApp([
      win("chrome.exe", 100, { windowTitle: "Docs" }),
      win("Google Chrome", 50, { windowTitle: "Mail" }),
      win("Code.exe", 120, { windowTitle: "main.rs" }),
    ]);
    expect(groups.map((g) => [g.appName, g.displayName, g.seconds])).toEqual([
      ["chrome.exe", "Google Chrome", 150],
      ["Code.exe", "Visual Studio Code", 120],
    ]);
  });
});

describe("readSessionDetails — what newer agents report about a block (contract §5)", () => {
  const full = {
    id: "a1",
    description: "  Fixing the login bug ",
    productivity: 72,
    idleSeconds: 95.6,
    endReason: "pause",
    platform: "Windows",
    osVersion: "Windows 11 Pro 23H2 (10.0.22631)",
    agentVersion: "0.2.0",
    deviceId: "d-1",
    deviceName: "DESKTOP-7Q2",
    timeZone: "Asia/Dhaka",
    utcOffsetMinutes: 360,
    clockSkewSeconds: -300,
    injectedInputEvents: 12,
    captures: [
      { id: "t", kind: "thumb", capturedAt: "2026-09-23T08:04:00Z", blurred: false },
      { id: "s", kind: "screen", capturedAt: "2026-09-23T08:04:30", blurred: true },
      { id: "w", kind: "webcam", capturedAt: "2026-09-23T08:10:00Z", blurred: false },
    ],
  };

  it("reads every field, pinning a zone-less capture time to UTC and preferring the screen capture", () => {
    expect(readSessionDetails(full)).toEqual({
      note: "Fixing the login bug",
      measured: true,
      idleSeconds: 96,
      endReason: "pause",
      platform: "windows",
      osVersion: "Windows 11 Pro 23H2 (10.0.22631)",
      agentVersion: "0.2.0",
      deviceId: "d-1",
      deviceName: "DESKTOP-7Q2",
      timeZone: "Asia/Dhaka",
      utcOffsetMinutes: 360,
      clockSkewSeconds: -300,
      injectedInputEvents: 12,
      capture: { capturedAt: "2026-09-23T08:04:30.000Z", blurred: true },
    });
  });

  it("reads activity % null as not measured — never as 0%", () => {
    expect(readSessionDetails({ ...full, productivity: null }).measured).toBe(false);
    expect(readSessionDetails({ id: "x" }).measured).toBe(false);
    expect(readSessionDetails({ ...full, productivity: 0 }).measured).toBe(true);
  });

  it("reads an older agent's block (and anything malformed) as not reported", () => {
    expect(readSessionDetails({ id: "old", productivity: 55, description: "", captures: [] })).toEqual({ ...NO_SESSION_DETAILS });
    const odd = readSessionDetails({
      ...full,
      idleSeconds: -5,
      endReason: "teleported",
      platform: "linux",
      utcOffsetMinutes: 2000,
      injectedInputEvents: "lots",
      captures: [{ id: "w", kind: "webcam" }, null],
    });
    expect(odd).toMatchObject({ idleSeconds: null, endReason: null, platform: null, utcOffsetMinutes: null, injectedInputEvents: null, capture: null });
    expect(readSessionDetails(null)).toEqual({ ...NO_SESSION_DETAILS, measured: false });
  });

  it("falls back to the thumbnail's capture time when there is no full capture", () => {
    expect(readSessionDetails({ ...full, captures: [full.captures[0]] }).capture).toEqual({
      capturedAt: "2026-09-23T08:04:00.000Z",
      blurred: false,
    });
  });

  it("indexes a list payload by row id, skipping rows without one", () => {
    const byId = sessionDetailsById([full, { description: "no id" }, null, { id: "b", productivity: null }]);
    expect([...byId.keys()]).toEqual(["a1", "b"]);
    expect(byId.get("b")?.measured).toBe(false);
    expect(sessionDetailsById("not a list").size).toBe(0);
  });
});

describe("demoSessionDetails — from the demo's own rows", () => {
  it("uses the description as the note and the inactive minutes as idle time", () => {
    const d = demoSessionDetails({
      startedAt: "2026-07-14T09:00:00.000Z",
      endedAt: "2026-07-14T09:10:00.000Z",
      productivity: 70,
      description: "Implementing new feature",
    });
    expect(d).toMatchObject({ note: "Implementing new feature", measured: true, idleSeconds: 3 * 60, deviceName: null, endReason: null });
  });

  it("treats the placeholder description as no note and never reports more idle than the block", () => {
    const d = demoSessionDetails({
      startedAt: "2026-07-14T09:00:00.000Z",
      endedAt: "2026-07-14T09:00:40.000Z",
      productivity: 0,
      description: DEFAULT_SESSION_DESCRIPTION,
    });
    expect(d.note).toBeNull();
    expect(d.idleSeconds).toBe(40);
  });
});

describe("end reasons, devices and clocks", () => {
  it("chips every end reason except a block that simply reached its interval", () => {
    expect(notableEndReason("interval")).toBeNull();
    expect(notableEndReason(null)).toBeNull();
    expect(notableEndReason("pause")).toBe("pause");
    expect(notableEndReason("shutdown")).toBe("shutdown");
    expect(endReasonTone("idle")).toBe("warning");
    expect(endReasonTone("break")).toBe("info");
    expect(endReasonTone("lock")).toBe("muted");
  });

  it("summarizes the device from whatever was reported", () => {
    expect(deviceSummary({ platform: "macos", osVersion: null, deviceName: "Ana's MacBook", agentVersion: "0.2.0" })).toBe(
      "macOS · Ana's MacBook · agent 0.2.0",
    );
    expect(deviceSummary({ platform: "windows", osVersion: "Windows 11 Pro", deviceName: null, agentVersion: null })).toBe("Windows 11 Pro");
    expect(deviceSummary({ platform: null, osVersion: null, deviceName: null, agentVersion: null })).toBeNull();
  });

  it("flags a device clock more than two minutes off, in the right direction", () => {
    expect(clockSkewNote(90)).toBeNull();
    expect(clockSkewNote(-120)).toBeNull();
    expect(clockSkewNote(180)).toBe("Device clock 3 min behind");
    expect(clockSkewNote(-600)).toBe("Device clock 10 min ahead");
    expect(clockSkewNote(3 * 3600)).toBe("Device clock 3 h behind");
    expect(clockSkewNote(null)).toBeNull();
  });
});
