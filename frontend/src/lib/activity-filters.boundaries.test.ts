// Boundary + branch behaviour of the activity filter engine.
//
// activity-filters.test.ts covers the happy path of each predicate; this file
// pins down the parts a "does it roughly filter?" test cannot see:
//   • the edges of the date range (whole days: first instant in, next day's
//     first instant out) and the INCLUSIVE time-of-day window,
//   • which range wins when a preset and stale custom dates are both present,
//   • that search matches EACH field independently and case-insensitively,
//   • the exact chip text and active-filter count for every single dimension,
//   • the sort comparators on inputs that are NOT already in the target order,
//   • the option builders, including their unknown-member fallback.
//
// A session belongs to the day and time it STARTED (the server's From/To
// rule), read in the reporting zone. The demo clock is frozen (NOW =
// 2026-07-14T15:30Z) and every date case names its zone, so nothing here
// depends on the wall clock or the zone the suite runs in.
import { describe, it, expect, beforeEach, afterAll } from "vitest";
import {
  activityApps,
  activeFilterChips,
  applyActivityFilters,
  countActiveFilters,
  defaultActivityFilters,
  memberOptions,
  projectOptions,
  type ActivityFilters,
} from "./activity-filters";
import { NOW } from "./mock-data";
import { hydrateLiveBackendData, projects } from "./tenant-data";
import type { Activity } from "./types";
import { buildActivity } from "@/test/factories";
import { resetPrototypeState } from "@/test/harness";

const DAY = 24 * 60 * 60 * 1000;
const ids = (list: Activity[]) => list.map((a) => a.id);

/** Passes every row on dates, so a test can isolate a single predicate. */
const wide: ActivityFilters = {
  ...defaultActivityFilters,
  rangeKey: "custom",
  customFrom: "2000-01-01",
  customTo: "2100-01-01",
};

// memberOptions()/activityApps() read the live tenant-data bindings, and two
// tests below re-point them via the real hydration entry point.
beforeEach(() => {
  resetPrototypeState();
});
afterAll(() => {
  resetPrototypeState();
});

/* ------------------------- date range: whole days of the zone ------------------------- */

/** A ten-minute block that starts at `startedAt`. */
const startingAt = (id: string, startedAt: string) =>
  buildActivity({ id, startedAt, endedAt: new Date(Date.parse(startedAt) + 10 * 60_000).toISOString() });

describe("applyActivityFilters — a custom date range covers whole days, both included", () => {
  const onFrom = startingAt("on-from", "2026-07-10T00:00:00.000Z");
  const justBefore = startingAt("just-before", "2026-07-09T23:59:59.999Z");
  const onTo = startingAt("on-to", "2026-07-10T23:59:59.000Z");
  const justAfter = startingAt("just-after", "2026-07-11T00:00:00.000Z");
  const singleDay = { ...wide, customFrom: "2026-07-10", customTo: "2026-07-10" };
  const utc = { timeZone: "UTC" };

  it("keeps a block that starts exactly at the first instant of the from-date", () => {
    expect(ids(applyActivityFilters([onFrom, justBefore], singleDay, utc))).toEqual(["on-from"]);
  });

  it("keeps a block that starts in the last second of the to-date, and drops one at the next midnight", () => {
    expect(ids(applyActivityFilters([onTo, justAfter], singleDay, utc))).toEqual(["on-to"]);
  });

  it("keeps a block that started on the day even when it ends after midnight (it's on the day it started)", () => {
    const overnight = buildActivity({ id: "overnight", startedAt: "2026-07-10T23:55:00.000Z", endedAt: "2026-07-11T00:05:00.000Z" });
    expect(ids(applyActivityFilters([overnight], singleDay, utc))).toEqual(["overnight"]);
    expect(applyActivityFilters([overnight], { ...wide, customFrom: "2026-07-11", customTo: "2026-07-11" }, utc)).toEqual([]);
  });

  it("reads the days in the reporting zone: a Dhaka day starts at 18:00 UTC the evening before", () => {
    const dhakaMorning = startingAt("dhaka-morning", "2026-07-09T18:30:00.000Z"); // 00:30 on 10 Jul in Dhaka
    expect(ids(applyActivityFilters([dhakaMorning], singleDay, { timeZone: "Asia/Dhaka" }))).toEqual(["dhaka-morning"]);
    expect(applyActivityFilters([dhakaMorning], singleDay, utc)).toEqual([]);
  });
});

/* ------------------------- time-of-day window: both ends inclusive ------------------------- */

describe("applyActivityFilters — the time-of-day window includes BOTH of its edges", () => {
  // Starts 09:20 UTC = 15:05 in Kathmandu (UTC+5:45): a zone with a quarter-hour offset.
  const row = startingAt("tod", "2026-07-10T09:20:00.000Z");
  const kathmandu = { timeZone: "Asia/Kathmandu" };

  it("keeps a block whose start time sits exactly ON the lower bound", () => {
    expect(ids(applyActivityFilters([row], { ...wide, startTime: "15:05" }, kathmandu))).toEqual(["tod"]);
  });

  it("keeps a block whose start time sits exactly ON the upper bound", () => {
    expect(ids(applyActivityFilters([row], { ...wide, endTime: "15:05" }, kathmandu))).toEqual(["tod"]);
  });

  it("drops a block one minute outside either bound", () => {
    expect(applyActivityFilters([row], { ...wide, startTime: "15:06" }, kathmandu)).toEqual([]);
    expect(applyActivityFilters([row], { ...wide, endTime: "15:04" }, kathmandu)).toEqual([]);
  });

  it("reads a block's clock time as minutes PAST the hour, not back from it", () => {
    // The bound is the top of the row's own hour. 15*60 + 5 clears it; 15*60 - 5 would not.
    expect(ids(applyActivityFilters([row], { ...wide, startTime: "15:00" }, kathmandu))).toEqual(["tod"]);
  });

  it("reads the start in the zone, not in UTC", () => {
    expect(ids(applyActivityFilters([row], { ...wide, startTime: "09:20", endTime: "09:20" }, { timeZone: "UTC" }))).toEqual(["tod"]);
    expect(applyActivityFilters([row], { ...wide, startTime: "09:20", endTime: "09:20" }, kathmandu)).toEqual([]);
  });
});

/* ------------------------- which range actually wins ------------------------- */

describe("applyActivityFilters — preset vs custom date range", () => {
  const recent = startingAt("recent", new Date(NOW.getTime() - DAY).toISOString());
  const ancient = startingAt("ancient", new Date(NOW.getTime() - 40 * DAY).toISOString());
  const both = [recent, ancient];

  it("really applies the default 7-day preset instead of letting everything through", () => {
    expect(ids(applyActivityFilters(both, defaultActivityFilters))).toEqual(["recent"]);
  });

  it("ignores leftover custom dates while a PRESET range is selected", () => {
    const f: ActivityFilters = {
      ...defaultActivityFilters,
      rangeKey: "30d",
      customFrom: "2000-01-01",
      customTo: "2100-01-01",
    };
    expect(ids(applyActivityFilters(both, f))).toEqual(["recent"]);
  });

  it("falls back to a preset window while 'custom' is selected but empty", () => {
    const f: ActivityFilters = { ...defaultActivityFilters, rangeKey: "custom" };
    expect(ids(applyActivityFilters(both, f))).toEqual(["recent"]);
  });

  it("treats a custom range with only a FROM date as open-ended forwards", () => {
    const f: ActivityFilters = { ...defaultActivityFilters, rangeKey: "custom", customFrom: "2000-01-01" };
    expect(ids(applyActivityFilters(both, f))).toEqual(["recent", "ancient"]);
  });

  it("treats a custom range with only a TO date as open-ended backwards", () => {
    const f: ActivityFilters = { ...defaultActivityFilters, rangeKey: "custom", customTo: "2100-01-01" };
    expect(ids(applyActivityFilters(both, f))).toEqual(["recent", "ancient"]);
  });
});

/* ------------------------- free-text search ------------------------- */

describe("applyActivityFilters — search matches each field on its own, case-insensitively", () => {
  // Every row is deliberately boring except in ONE searchable field, so a hit
  // proves that field alone is enough (the search is an OR, not an AND).
  const plain = {
    description: "Untitled block",
    screen: { app: "Terminal", kind: "terminal" as const, accent: "#22c55e" },
    activeWindows: [],
    runningPrograms: [],
    userId: "u1", // Ayesha Rahman
    projectId: "p1", // Dosi Web Platform
  };
  const byDescription = buildActivity({ ...plain, id: "q-description", description: "Polishing dark mode tokens" });
  const byScreenApp = buildActivity({ ...plain, id: "q-screen-app", screen: { app: "Figma", kind: "design", accent: "#ec4899" } });
  const byWindowApp = buildActivity({ ...plain, id: "q-window-app", activeWindows: [{ appName: "Postman", windowTitle: "Collections", seconds: 90 }] });
  const byWindowTitle = buildActivity({ ...plain, id: "q-window-title", activeWindows: [{ appName: "Notion", windowTitle: "Sprint Retro", seconds: 90 }] });
  const byMember = buildActivity({ ...plain, id: "q-member", userId: "u5" }); // Sadia Akter
  const byProject = buildActivity({ ...plain, id: "q-project", projectId: "p2" }); // Mobile App Revamp
  const orphan = buildActivity({ ...plain, id: "q-orphan", userId: "u-gone", projectId: "p-gone" });
  const corpus = [byDescription, byScreenApp, byWindowApp, byWindowTitle, byMember, byProject, orphan];

  const hits = (query: string) => ids(applyActivityFilters(corpus, { ...wide, query })).sort();

  it("matches the task description", () => expect(hits("POLISHING")).toEqual(["q-description"]));
  it("matches the foreground app", () => expect(hits("FIGMA")).toEqual(["q-screen-app"]));
  it("matches an active window's app name", () => expect(hits("POSTMAN")).toEqual(["q-window-app"]));
  it("matches an active window's title", () => expect(hits("SPRINT")).toEqual(["q-window-title"]));
  it("matches the member's name", () => expect(hits("SADIA")).toEqual(["q-member"]));
  it("matches the project's title", () => expect(hits("MOBILE APP")).toEqual(["q-project"]));

  it("ignores whitespace the user typed around the term", () => {
    expect(hits("   polishing   ")).toEqual(["q-description"]);
  });

  it("returns nothing for a term no field contains — even on rows whose member/project is unknown", () => {
    expect(hits("no-field-contains-this")).toEqual([]);
  });
});

/* ------------------------- app filter across the window lists ------------------------- */

describe("applyActivityFilters — the app filter looks past the foreground screen", () => {
  const foreground = buildActivity({
    id: "foreground",
    screen: { app: "Terminal", kind: "terminal", accent: "#22c55e" },
    activeWindows: [{ appName: "Figma", windowTitle: "Hero", seconds: 120 }],
    runningPrograms: [{ appName: "Zoom", windowTitle: "—", seconds: 0 }],
  });
  const background = buildActivity({
    id: "background",
    screen: { app: "Notion", kind: "docs", accent: "#64748b" },
    activeWindows: [{ appName: "Slack", windowTitle: "#design", seconds: 60 }],
    runningPrograms: [{ appName: "Postman", windowTitle: "—", seconds: 0 }],
  });
  const both = [foreground, background];

  it("matches an app that only appears as an ACTIVE WINDOW", () => {
    expect(ids(applyActivityFilters(both, { ...wide, app: "Figma" }))).toEqual(["foreground"]);
  });

  it("matches an app that only appears as a RUNNING PROGRAM", () => {
    expect(ids(applyActivityFilters(both, { ...wide, app: "Postman" }))).toEqual(["background"]);
  });

  it("matches nothing when the app is in none of the three lists", () => {
    expect(applyActivityFilters(both, { ...wide, app: "Spotify" })).toEqual([]);
  });
});

/* ------------------------- sorting an input that is NOT already sorted ------------------------- */

describe("applyActivityFilters — comparators must actually reorder", () => {
  it("oldest-first re-orders a list that does not arrive oldest-first", () => {
    const middle = buildActivity({ id: "middle", endedAt: "2026-07-10T10:00:00.000Z" });
    const earliest = buildActivity({ id: "earliest", endedAt: "2026-07-01T10:00:00.000Z" });
    const latest = buildActivity({ id: "latest", endedAt: "2026-07-13T10:00:00.000Z" });
    expect(ids(applyActivityFilters([middle, earliest, latest], { ...wide, sort: "oldest" }))).toEqual([
      "earliest",
      "middle",
      "latest",
    ]);
  });

  it("most-active ranks on clicks PLUS keystrokes, so keyboard work can outrank mouse work", () => {
    const keyboardHeavy = buildActivity({ id: "keyboard-heavy", mouseClicks: 20, keyboardHits: 900 }); // 920
    const mouseHeavy = buildActivity({ id: "mouse-heavy", mouseClicks: 700, keyboardHits: 10 }); // 710
    const quiet = buildActivity({ id: "quiet", mouseClicks: 30, keyboardHits: 60 }); // 90
    expect(
      ids(applyActivityFilters([keyboardHeavy, mouseHeavy, quiet], { ...wide, sort: "active-high" }))
    ).toEqual(["keyboard-heavy", "mouse-heavy", "quiet"]);
  });
});

/* ------------------------- the "N active" badge ------------------------- */

describe("countActiveFilters — every dimension counts on its own", () => {
  const only = (over: Partial<ActivityFilters>) => countActiveFilters({ ...defaultActivityFilters, ...over });

  it("counts a project narrowing", () => {
    expect(only({ projectId: "p2" })).toBe(1);
  });

  it("counts a preset date change", () => {
    expect(only({ rangeKey: "30d" })).toBe(1);
  });

  it("counts custom bounds even while the preset key is untouched", () => {
    expect(only({ customFrom: "2026-07-01" })).toBe(1);
    expect(only({ customTo: "2026-07-31" })).toBe(1);
  });

  it("counts a one-sided time window", () => {
    expect(only({ startTime: "09:00" })).toBe(1);
    expect(only({ endTime: "17:00" })).toBe(1);
  });

  it("counts a one-sided productivity window", () => {
    expect(only({ minProductivity: 20 })).toBe(1);
    expect(only({ maxProductivity: 80 })).toBe(1);
  });
});

/* ------------------------- the chip row ------------------------- */

describe("activeFilterChips — exact text, one chip per active dimension", () => {
  const chips = (over: Partial<ActivityFilters>) => activeFilterChips({ ...defaultActivityFilters, ...over });

  it("shows no date chip while 'custom' is selected but no bound has been entered", () => {
    expect(chips({ rangeKey: "custom" })).toEqual([]);
  });

  it("marks the missing side of a half-open custom range with an ellipsis", () => {
    expect(chips({ rangeKey: "custom", customFrom: "2026-07-01" })).toEqual([
      { key: "rangeKey", label: "Date: 2026-07-01 → …" },
    ]);
    expect(chips({ rangeKey: "custom", customTo: "2026-07-31" })).toEqual([
      { key: "rangeKey", label: "Date: … → 2026-07-31" },
    ]);
  });

  it("names each date preset the same way the preset buttons do", () => {
    expect(chips({ rangeKey: "today" })).toEqual([{ key: "rangeKey", label: "Date: Today" }]);
    expect(chips({ rangeKey: "yesterday" })).toEqual([{ key: "rangeKey", label: "Date: Yesterday" }]);
    expect(chips({ rangeKey: "month" })).toEqual([{ key: "rangeKey", label: "Date: This month" }]);
  });

  it("shows a productivity chip when only ONE end of the range has moved", () => {
    expect(chips({ minProductivity: 30 })).toEqual([{ key: "productivity", label: "Activity: 30–100%" }]);
    expect(chips({ maxProductivity: 70 })).toEqual([{ key: "productivity", label: "Activity: 0–70%" }]);
  });

  it("spells out the two toggle chips", () => {
    expect(chips({ onlineOnly: true })).toEqual([{ key: "onlineOnly", label: "Live only" }]);
    expect(chips({ webcamOnly: true })).toEqual([{ key: "webcamOnly", label: "With webcam" }]);
  });
});

/* ------------------------- option builders ------------------------- */

describe("filter option builders (driven by the live tenant data)", () => {
  it("offers every app seen on screen, in an active window OR in a running program — never a placeholder", () => {
    hydrateLiveBackendData(projects, [
      buildActivity({
        id: "apps",
        screen: { app: "Terminal", kind: "terminal", accent: "#22c55e" },
        activeWindows: [{ appName: "Figma", windowTitle: "Hero", seconds: 120 }],
        runningPrograms: [
          { appName: "Docker Desktop", windowTitle: "—", seconds: 0 },
          { appName: "—", windowTitle: "—", seconds: 0 },
          { appName: "", windowTitle: "—", seconds: 0 },
        ],
      }),
    ]);

    expect(activityApps()).toEqual(["Docker Desktop", "Figma", "Terminal"]);
  });

  it("lists each tracked member once, in first-seen order, falling back to the raw id", () => {
    hydrateLiveBackendData(projects, [
      buildActivity({ id: "m1", userId: "u2" }),
      buildActivity({ id: "m2", userId: "u1" }),
      buildActivity({ id: "m3", userId: "u2" }), // repeat — must not appear twice
      buildActivity({ id: "m4", userId: "u-not-in-this-workspace" }),
    ]);

    expect(memberOptions()).toEqual([
      { id: "all", name: "All members" },
      { id: "u2", name: "Tanvir Hasan" },
      { id: "u1", name: "Ayesha Rahman" },
      { id: "u-not-in-this-workspace", name: "u-not-in-this-workspace" },
    ]);
  });

  it("lists every project of the workspace, archived ones included", () => {
    expect(projectOptions()).toEqual([
      { id: "all", title: "All projects" },
      { id: "p1", title: "Dosi Web Platform" },
      { id: "p2", title: "Mobile App Revamp" },
      { id: "p3", title: "Acme Corp CRM" },
      { id: "p4", title: "Marketing Website" },
      { id: "p5", title: "Data Pipeline" }, // archived, but still selectable
      { id: "p6", title: "Design System" },
    ]);
  });
});
