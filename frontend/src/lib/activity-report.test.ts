import { describe, expect, it } from "vitest";
import {
  blockSeconds,
  formatClock,
  formatClockShort,
  formatTimeRange,
  groupWindowsByApp,
  mainAppOf,
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
