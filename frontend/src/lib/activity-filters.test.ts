import { describe, it, expect } from "vitest";
import {
  applyActivityFilters,
  activityDateRange,
  countActiveFilters,
  activeFilterChips,
  activityApps,
  formatActivityRange,
  hasLocalFilters,
  memberOptions,
  projectOptions,
  defaultActivityFilters,
  type ActivityFilters,
} from "./activity-filters";
import type { Activity } from "./types";

function act(over: Partial<Activity> & Pick<Activity, "id">): Activity {
  return {
    userId: "u1",
    projectId: "p1",
    startedAt: "2024-06-01T09:00:00Z",
    endedAt: "2024-06-01T10:00:00Z",
    description: "work",
    productivity: 50,
    mouseClicks: 0,
    keyboardHits: 0,
    activeWindows: [],
    runningPrograms: [],
    screen: { app: "VSCode", kind: "editor", accent: "#000" },
    hasWebcam: false,
    online: false,
    ...over,
  };
}

const a1 = act({ id: "a1", userId: "u1", projectId: "p1", startedAt: "2024-06-01T09:00:00Z", endedAt: "2024-06-01T10:00:00Z", productivity: 90, mouseClicks: 100, keyboardHits: 50, screen: { app: "VSCode", kind: "editor", accent: "#000" }, online: true, hasWebcam: true, description: "writing tests" });
const a2 = act({ id: "a2", userId: "u2", projectId: "p1", startedAt: "2024-06-02T09:00:00Z", endedAt: "2024-06-02T10:00:00Z", productivity: 40, mouseClicks: 10, keyboardHits: 5, screen: { app: "Chrome", kind: "browser", accent: "#000" }, online: false, hasWebcam: false, description: "browsing docs" });
const a3 = act({ id: "a3", userId: "u1", projectId: "p2", startedAt: "2024-06-03T09:00:00Z", endedAt: "2024-06-03T10:00:00Z", productivity: 60, mouseClicks: 200, keyboardHits: 100, screen: { app: "Slack", kind: "chat", accent: "#000" }, online: true, hasWebcam: false, description: "chatting" });
const all = [a1, a2, a3];

// Base filter that lets everything through on date, so each test isolates ONE predicate.
const wide: ActivityFilters = {
  ...defaultActivityFilters,
  rangeKey: "custom",
  customFrom: "2000-01-01",
  customTo: "2100-01-01",
};
const ids = (list: Activity[]) => list.map((a) => a.id);
const utc = { timeZone: "UTC" };

describe("applyActivityFilters — predicates", () => {
  it("returns all rows when only a wide date range is applied", () => {
    expect(applyActivityFilters(all, wide)).toHaveLength(3);
  });
  it("filters by member", () => {
    expect(ids(applyActivityFilters(all, { ...wide, memberId: "u1" })).sort()).toEqual(["a1", "a3"]);
  });
  it("filters by project", () => {
    expect(ids(applyActivityFilters(all, { ...wide, projectId: "p1" })).sort()).toEqual(["a1", "a2"]);
  });
  it("filters by minimum productivity (inclusive)", () => {
    expect(ids(applyActivityFilters(all, { ...wide, minProductivity: 60 })).sort()).toEqual(["a1", "a3"]);
  });
  it("filters by maximum productivity (inclusive)", () => {
    expect(ids(applyActivityFilters(all, { ...wide, maxProductivity: 40 }))).toEqual(["a2"]);
  });
  it("filters by app (screen app)", () => {
    expect(ids(applyActivityFilters(all, { ...wide, app: "Chrome" }))).toEqual(["a2"]);
  });
  it("filters online only", () => {
    expect(ids(applyActivityFilters(all, { ...wide, onlineOnly: true })).sort()).toEqual(["a1", "a3"]);
  });
  it("filters webcam only", () => {
    expect(ids(applyActivityFilters(all, { ...wide, webcamOnly: true }))).toEqual(["a1"]);
  });
  it("filters by a free-text query against description", () => {
    expect(ids(applyActivityFilters(all, { ...wide, query: "brows" }))).toEqual(["a2"]);
  });
  it("excludes rows whose START falls outside a custom range (the server's rule)", () => {
    const narrow = { ...wide, customFrom: "2024-06-02", customTo: "2024-06-02" };
    expect(ids(applyActivityFilters(all, narrow, utc))).toEqual(["a2"]);
  });
});

describe("applyActivityFilters — sorting", () => {
  it("defaults to most recent first", () => {
    expect(ids(applyActivityFilters(all, wide))).toEqual(["a3", "a2", "a1"]);
  });
  it("oldest first", () => {
    expect(ids(applyActivityFilters(all, { ...wide, sort: "oldest" }))).toEqual(["a1", "a2", "a3"]);
  });
  it("productivity high to low", () => {
    expect(ids(applyActivityFilters(all, { ...wide, sort: "prod-high" }))).toEqual(["a1", "a3", "a2"]);
  });
  it("productivity low to high", () => {
    expect(ids(applyActivityFilters(all, { ...wide, sort: "prod-low" }))).toEqual(["a2", "a3", "a1"]);
  });
  it("most active first (clicks + keys)", () => {
    expect(ids(applyActivityFilters(all, { ...wide, sort: "active-high" }))).toEqual(["a3", "a1", "a2"]);
  });
});

describe("applyActivityFilters — live sessions come from presence, not from the row", () => {
  it("keeps the blocks the page says are running now, whatever their own flag", () => {
    const live = new Set(["a2"]);
    expect(ids(applyActivityFilters(all, { ...wide, onlineOnly: true }, { isLive: (a) => live.has(a.id) }))).toEqual(["a2"]);
    expect(applyActivityFilters(all, { ...wide, onlineOnly: true }, { isLive: () => false })).toEqual([]);
  });
});

describe("applyActivityFilters — blocks whose activity wasn't measured", () => {
  const unmeasured = act({ id: "n", startedAt: "2024-06-04T09:00:00Z", endedAt: "2024-06-04T10:00:00Z", productivity: 0 });
  const rows = [...all, unmeasured];
  const ctx = { isMeasured: (a: Activity) => a.id !== "n" };

  it("are kept while the activity window is untouched", () => {
    expect(ids(applyActivityFilters(rows, wide, ctx))).toContain("n");
  });

  it("never pass a narrowed activity window — not even one starting at 0%", () => {
    expect(ids(applyActivityFilters(rows, { ...wide, maxProductivity: 50 }, ctx))).toEqual(["a2"]);
    expect(ids(applyActivityFilters(rows, { ...wide, minProductivity: 0, maxProductivity: 95 }, ctx))).not.toContain("n");
  });

  it("sort after every measured block in both directions", () => {
    expect(ids(applyActivityFilters(rows, { ...wide, sort: "prod-high" }, ctx))).toEqual(["a1", "a3", "a2", "n"]);
    expect(ids(applyActivityFilters(rows, { ...wide, sort: "prod-low" }, ctx))).toEqual(["a2", "a3", "a1", "n"]);
  });
});

describe("applyActivityFilters — apps by their canonical name", () => {
  it("matches the Windows executable and the macOS name of the same app", () => {
    const win = act({ id: "win", screen: { app: "chrome.exe", kind: "browser", accent: "#000" } });
    const mac = act({ id: "mac", screen: { app: "Google Chrome", kind: "browser", accent: "#000" } });
    const other = act({ id: "other", screen: { app: "Code.exe", kind: "editor", accent: "#000" } });
    expect(ids(applyActivityFilters([win, mac, other], { ...wide, app: "Google Chrome" })).sort()).toEqual(["mac", "win"]);
  });

  it("finds an app by its canonical name in the search too", () => {
    const row = act({ id: "code", screen: { app: "Code.exe", kind: "editor", accent: "#000" } });
    expect(ids(applyActivityFilters([row], { ...wide, query: "visual studio" }))).toEqual(["code"]);
  });

  it("offers each app once, canonically named", () => {
    const rows = [
      act({ id: "x", screen: { app: "chrome.exe", kind: "browser", accent: "#000" }, activeWindows: [{ appName: "Google Chrome", windowTitle: "", seconds: 5 }] }),
      act({ id: "y", screen: { app: "Code.exe", kind: "editor", accent: "#000" }, runningPrograms: [{ appName: "slack.exe", windowTitle: "", seconds: 0 }] }),
    ];
    expect(activityApps(rows)).toEqual(["Google Chrome", "Slack", "Visual Studio Code"]);
  });
});

describe("activityDateRange — whole days of the reporting zone", () => {
  // Wed 23 Sep 2026, 01:30 in Dhaka (UTC+6) but still Tue 22 Sep 19:30 in UTC.
  const now = new Date("2026-09-22T19:30:00.000Z");
  const dhaka = { now, timeZone: "Asia/Dhaka" };
  const range = (over: Partial<ActivityFilters>, clock = dhaka) => activityDateRange({ ...defaultActivityFilters, ...over }, clock);

  it("reads Today and Yesterday as local calendar days, not UTC ones", () => {
    expect(range({ rangeKey: "today" })).toEqual({
      from: Date.parse("2026-09-22T18:00:00.000Z"),
      to: Date.parse("2026-09-23T18:00:00.000Z"),
      fromKey: "2026-09-23",
      toKey: "2026-09-23",
    });
    expect(range({ rangeKey: "yesterday" })).toMatchObject({ fromKey: "2026-09-22", toKey: "2026-09-22" });
    expect(range({ rangeKey: "today" }, { now, timeZone: "UTC" })).toMatchObject({ fromKey: "2026-09-22", toKey: "2026-09-22" });
  });

  it("makes the last 7 / 30 days today plus the days before, and this month its first day to today", () => {
    expect(range({ rangeKey: "7d" })).toMatchObject({ fromKey: "2026-09-17", toKey: "2026-09-23" });
    expect(range({ rangeKey: "30d" })).toMatchObject({ fromKey: "2026-08-25", toKey: "2026-09-23" });
    expect(range({ rangeKey: "month" })).toMatchObject({ fromKey: "2026-09-01", toKey: "2026-09-23" });
  });

  it("reads custom days in the zone, open on a side left empty, and falls back to 7 days with none", () => {
    expect(range({ rangeKey: "custom", customFrom: "2026-09-01", customTo: "2026-09-02" })).toEqual({
      from: Date.parse("2026-08-31T18:00:00.000Z"),
      to: Date.parse("2026-09-02T18:00:00.000Z"),
      fromKey: "2026-09-01",
      toKey: "2026-09-02",
    });
    expect(range({ rangeKey: "custom", customFrom: "2026-09-01" })).toMatchObject({ to: Number.POSITIVE_INFINITY, toKey: null });
    expect(range({ rangeKey: "custom", customTo: "2026-09-02" })).toMatchObject({ from: Number.NEGATIVE_INFINITY, fromKey: null });
    expect(range({ rangeKey: "custom" })).toMatchObject({ fromKey: "2026-09-17", toKey: "2026-09-23" });
  });

  it("keeps a block on the local day it started, so 'Today' and the day groups agree", () => {
    const earlyDhaka = act({ id: "early", startedAt: "2026-09-22T18:30:00.000Z", endedAt: "2026-09-22T18:40:00.000Z" }); // 00:30 local
    expect(ids(applyActivityFilters([earlyDhaka], { ...defaultActivityFilters, rangeKey: "today" }, dhaka))).toEqual(["early"]);
    expect(applyActivityFilters([earlyDhaka], { ...defaultActivityFilters, rangeKey: "yesterday" }, dhaka)).toEqual([]);
  });

  it("formats the days a range covers", () => {
    expect(formatActivityRange(range({ rangeKey: "7d" }))).toBe("17 Sep – 23 Sep 2026");
    expect(formatActivityRange(range({ rangeKey: "today" }))).toBe("23 Sep 2026");
    expect(formatActivityRange({ fromKey: "2026-09-01", toKey: null })).toBe("From 1 Sep 2026");
    expect(formatActivityRange({ fromKey: null, toKey: "2026-09-02" })).toBe("Until 2 Sep 2026");
    expect(formatActivityRange({ fromKey: null, toKey: null })).toBe("All time");
  });
});

describe("hasLocalFilters — what the server query doesn't already apply", () => {
  it("ignores the dates, member and project (the server filters those) and counts the rest", () => {
    expect(hasLocalFilters(defaultActivityFilters)).toBe(false);
    expect(hasLocalFilters({ ...defaultActivityFilters, rangeKey: "30d", memberId: "u1", projectId: "p1" })).toBe(false);
    expect(hasLocalFilters({ ...defaultActivityFilters, query: " x " })).toBe(true);
    expect(hasLocalFilters({ ...defaultActivityFilters, startTime: "09:00" })).toBe(true);
    expect(hasLocalFilters({ ...defaultActivityFilters, minProductivity: 10 })).toBe(true);
    expect(hasLocalFilters({ ...defaultActivityFilters, app: "Slack" })).toBe(true);
    expect(hasLocalFilters({ ...defaultActivityFilters, onlineOnly: true })).toBe(true);
    expect(hasLocalFilters({ ...defaultActivityFilters, webcamOnly: true })).toBe(true);
  });
});

describe("countActiveFilters", () => {
  it("is zero for the defaults", () => {
    expect(countActiveFilters(defaultActivityFilters)).toBe(0);
  });
  it("counts each changed dimension once", () => {
    expect(countActiveFilters({ ...defaultActivityFilters, memberId: "u1" })).toBe(1);
    expect(countActiveFilters({ ...defaultActivityFilters, onlineOnly: true, webcamOnly: true })).toBe(2);
    expect(
      countActiveFilters({ ...defaultActivityFilters, minProductivity: 20, maxProductivity: 80, app: "Chrome" })
    ).toBe(2); // productivity range = 1, app = 1
  });
});

describe("activeFilterChips", () => {
  it("emits chips only for non-default filters", () => {
    expect(activeFilterChips(defaultActivityFilters)).toEqual([]);
    const chips = activeFilterChips({ ...defaultActivityFilters, onlineOnly: true, webcamOnly: true });
    expect(chips.map((c) => c.key)).toEqual(["onlineOnly", "webcamOnly"]);
  });
  it("labels a productivity narrowing", () => {
    const chips = activeFilterChips({ ...defaultActivityFilters, minProductivity: 30, maxProductivity: 70 });
    expect(chips[0]).toMatchObject({ key: "productivity", label: "Activity: 30–70%" });
  });
  it("labels a preset date range via its human name", () => {
    const chips = activeFilterChips({ ...defaultActivityFilters, rangeKey: "30d" });
    expect(chips).toContainEqual({ key: "rangeKey", label: "Date: Last 30 days" });
  });
  it("labels a custom date range with its bounds", () => {
    const chips = activeFilterChips({ ...defaultActivityFilters, rangeKey: "custom", customFrom: "2024-06-01", customTo: "2024-06-30" });
    expect(chips).toContainEqual({ key: "rangeKey", label: "Date: 2024-06-01 → 2024-06-30" });
  });
  it("labels app and time-of-day filters exactly", () => {
    const chips = activeFilterChips({ ...defaultActivityFilters, app: "Chrome", startTime: "09:00", endTime: "17:00" });
    expect(chips).toContainEqual({ key: "app", label: "App: Chrome" });
    expect(chips).toContainEqual({ key: "time", label: "Time: 09:00–17:00" });
  });
  it("falls back to the raw id in a chip when the member/project lookup misses", () => {
    const chips = activeFilterChips({
      ...defaultActivityFilters,
      memberId: "no-such-member",
      projectId: "no-such-project",
    });
    expect(chips).toContainEqual({ key: "memberId", label: "Member: no-such-member" });
    expect(chips).toContainEqual({ key: "projectId", label: "Project: no-such-project" });
  });
  it("defaults an open-ended time filter to 00:00 and 23:59", () => {
    const fromOnly = activeFilterChips({ ...defaultActivityFilters, startTime: "08:30" });
    expect(fromOnly).toContainEqual({ key: "time", label: "Time: 08:30–23:59" });
    const toOnly = activeFilterChips({ ...defaultActivityFilters, endTime: "18:00" });
    expect(toOnly).toContainEqual({ key: "time", label: "Time: 00:00–18:00" });
  });
});

describe("applyActivityFilters — time-of-day window (start time, in the zone)", () => {
  // a1 starts at 09:00 UTC: 15:00 in Dhaka.
  it("keeps a row whose start is inside the window in the zone", () => {
    expect(ids(applyActivityFilters([a1], { ...wide, startTime: "08:59", endTime: "09:01" }, utc))).toEqual(["a1"]);
    expect(ids(applyActivityFilters([a1], { ...wide, startTime: "14:59", endTime: "15:01" }, { timeZone: "Asia/Dhaka" }))).toEqual(["a1"]);
  });
  it("drops a row before the lower bound", () => {
    expect(applyActivityFilters([a1], { ...wide, startTime: "09:02" }, utc)).toHaveLength(0);
  });
  it("drops a row after the upper bound", () => {
    expect(applyActivityFilters([a1], { ...wide, endTime: "08:58" }, utc)).toHaveLength(0);
  });
});

describe("option builders (from live tenant data)", () => {
  it("activityApps returns a sorted, de-duplicated, placeholder-free list", () => {
    const apps = activityApps();
    expect(apps.length).toBeGreaterThan(0);
    expect(apps).not.toContain("—");
    expect([...apps]).toEqual([...apps].sort());
    expect(new Set(apps).size).toBe(apps.length);
  });
  it("member/project option lists lead with an 'all' entry", () => {
    expect(memberOptions()[0]).toEqual({ id: "all", name: "All members" });
    expect(projectOptions()[0]).toEqual({ id: "all", title: "All projects" });
  });
});
