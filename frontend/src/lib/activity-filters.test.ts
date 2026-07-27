import { describe, it, expect } from "vitest";
import {
  applyActivityFilters,
  countActiveFilters,
  activeFilterChips,
  activityApps,
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

const a1 = act({ id: "a1", userId: "u1", projectId: "p1", endedAt: "2024-06-01T10:00:00Z", productivity: 90, mouseClicks: 100, keyboardHits: 50, screen: { app: "VSCode", kind: "editor", accent: "#000" }, online: true, hasWebcam: true, description: "writing tests" });
const a2 = act({ id: "a2", userId: "u2", projectId: "p1", endedAt: "2024-06-02T10:00:00Z", productivity: 40, mouseClicks: 10, keyboardHits: 5, screen: { app: "Chrome", kind: "browser", accent: "#000" }, online: false, hasWebcam: false, description: "browsing docs" });
const a3 = act({ id: "a3", userId: "u1", projectId: "p2", endedAt: "2024-06-03T10:00:00Z", productivity: 60, mouseClicks: 200, keyboardHits: 100, screen: { app: "Slack", kind: "chat", accent: "#000" }, online: true, hasWebcam: false, description: "chatting" });
const all = [a1, a2, a3];

// Base filter that lets everything through on date, so each test isolates ONE predicate.
const wide: ActivityFilters = {
  ...defaultActivityFilters,
  rangeKey: "custom",
  customFrom: "2000-01-01",
  customTo: "2100-01-01",
};
const ids = (list: Activity[]) => list.map((a) => a.id);

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
  it("excludes rows whose ended date falls outside a custom range", () => {
    const narrow = { ...wide, customFrom: "2024-06-02", customTo: "2024-06-02" };
    expect(ids(applyActivityFilters(all, narrow))).toEqual(["a2"]);
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
    expect(chips[0]).toMatchObject({ key: "productivity", label: "Productivity: 30–70%" });
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

describe("applyActivityFilters — time-of-day window (local, tz-robust)", () => {
  // minutesOfDay uses the LOCAL clock, so derive the window from the fixture's own local time
  // rather than hard-coding it — keeps the test correct in any CI timezone.
  const local = new Date(a1.endedAt);
  const localMin = local.getHours() * 60 + local.getMinutes();
  const hhmm = (min: number) => {
    const m = (min + 1440) % 1440;
    return `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
  };

  it("keeps a row whose local time is inside the window", () => {
    const f = { ...wide, startTime: hhmm(localMin - 1), endTime: hhmm(localMin + 1) };
    expect(ids(applyActivityFilters([a1], f))).toEqual(["a1"]);
  });
  it("drops a row before the lower bound", () => {
    const f = { ...wide, startTime: hhmm(localMin + 2) };
    expect(applyActivityFilters([a1], f)).toHaveLength(0);
  });
  it("drops a row after the upper bound", () => {
    const f = { ...wide, endTime: hhmm(localMin - 2) };
    expect(applyActivityFilters([a1], f)).toHaveLength(0);
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
