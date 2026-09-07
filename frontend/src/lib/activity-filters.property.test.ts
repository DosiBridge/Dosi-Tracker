import { describe, it, expect } from "vitest";
import fc from "fast-check";
import {
  applyActivityFilters,
  defaultActivityFilters,
  type ActivityFilters,
  type ActivitySort,
} from "./activity-filters";
import type { RangeKey } from "./reports-data";
import type { Activity } from "./types";
import { NOW } from "./mock-data";
import { buildActivity } from "@/test/factories";

/* ----------------------------- Arbitraries ----------------------------- */

const userPool = ["u1", "u2", "u9-unknown"];
const projectPool = ["p1", "p2", "p9-unknown"];
const appPool = ["Visual Studio Code", "Google Chrome", "Slack"];

const MIN = 60 * 1000;

// Activities spread across the 40 days before the frozen demo clock (2026-07-14T15:30Z),
// so preset ranges (today/7d/30d/month) all have both in-range and out-of-range items.
const activityArb: fc.Arbitrary<Activity> = fc
  .record({
    userId: fc.constantFrom(...userPool),
    projectId: fc.constantFrom(...projectPool),
    minutesAgo: fc.integer({ min: 0, max: 40 * 24 * 60 }),
    productivity: fc.integer({ min: 0, max: 100 }),
    mouseClicks: fc.integer({ min: 0, max: 1000 }),
    keyboardHits: fc.integer({ min: 0, max: 5000 }),
    online: fc.boolean(),
    hasWebcam: fc.boolean(),
    app: fc.constantFrom(...appPool),
    description: fc.constantFrom("Writing tests", "Fixing bug", "রিপোর্ট লিখছি", "Design review"),
  })
  .map((r) =>
    buildActivity({
      userId: r.userId,
      projectId: r.projectId,
      endedAt: new Date(NOW.getTime() - r.minutesAgo * MIN).toISOString(),
      startedAt: new Date(NOW.getTime() - (r.minutesAgo + 10) * MIN).toISOString(),
      productivity: r.productivity,
      mouseClicks: r.mouseClicks,
      keyboardHits: r.keyboardHits,
      online: r.online,
      hasWebcam: r.hasWebcam,
      screen: { app: r.app, kind: "editor", accent: "#000" },
      description: r.description,
      activeWindows: [],
      runningPrograms: [],
    })
  );

const listArb = fc.array(activityArb, { maxLength: 25 });

const hhmmArb = fc
  .tuple(fc.integer({ min: 0, max: 23 }), fc.integer({ min: 0, max: 59 }))
  .map(([h, m]) => `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`);

const dateStrArb = fc.oneof(
  fc.constant(""),
  fc.integer({ min: 0, max: 45 }).map((d) => new Date(NOW.getTime() - d * 24 * 60 * MIN).toISOString().slice(0, 10))
);

const filtersArb: fc.Arbitrary<ActivityFilters> = fc.record({
  query: fc.constantFrom("", "test", "chrome", "fix", "রিপোর্ট"),
  memberId: fc.constantFrom("all", ...userPool),
  projectId: fc.constantFrom("all", ...projectPool),
  rangeKey: fc.constantFrom<RangeKey>("today", "yesterday", "7d", "30d", "month", "custom"),
  customFrom: dateStrArb,
  customTo: dateStrArb,
  startTime: fc.oneof(fc.constant(""), hhmmArb),
  endTime: fc.oneof(fc.constant(""), hhmmArb),
  minProductivity: fc.integer({ min: 0, max: 100 }),
  maxProductivity: fc.integer({ min: 0, max: 100 }),
  app: fc.constantFrom("all", ...appPool),
  onlineOnly: fc.boolean(),
  webcamOnly: fc.boolean(),
  sort: fc.constantFrom<ActivitySort>("recent", "oldest", "prod-high", "prod-low", "active-high"),
});

// A base filter that passes everything on dates, used to isolate single dimensions.
const wide: ActivityFilters = {
  ...defaultActivityFilters,
  rangeKey: "custom",
  customFrom: "2000-01-01",
  customTo: "2100-01-01",
};

const ids = (list: Activity[]) => list.map((a) => a.id);
const idSet = (list: Activity[]) => new Set(ids(list));

/* ----------------------------- Properties ----------------------------- */

describe("applyActivityFilters — structural invariants (property-based)", () => {
  it("never invents rows: every output item comes from the input, at most once", () => {
    fc.assert(
      fc.property(listArb, filtersArb, (list, f) => {
        const out = applyActivityFilters(list, f);
        expect(out.length).toBeLessThanOrEqual(list.length);
        const inputIds = idSet(list);
        for (const a of out) expect(inputIds.has(a.id)).toBe(true);
        // factory ids are unique, so no row may be duplicated by filtering/sorting
        expect(idSet(out).size).toBe(out.length);
      })
    );
  });

  it("is idempotent: filtering an already-filtered list changes nothing (ids AND order)", () => {
    fc.assert(
      fc.property(listArb, filtersArb, (list, f) => {
        const once = applyActivityFilters(list, f);
        const twice = applyActivityFilters(once, f);
        expect(ids(twice)).toEqual(ids(once));
      })
    );
  });

  it("does not mutate the input list", () => {
    fc.assert(
      fc.property(listArb, filtersArb, (list, f) => {
        const before = ids(list);
        applyActivityFilters(list, f);
        expect(ids(list)).toEqual(before);
      })
    );
  });

  it("returns empty for empty input under every filter combination", () => {
    fc.assert(
      fc.property(filtersArb, (f) => {
        expect(applyActivityFilters([], f)).toEqual([]);
      })
    );
  });

  it("is deterministic: the same input and filter always yield the same order", () => {
    fc.assert(
      fc.property(listArb, filtersArb, (list, f) => {
        expect(ids(applyActivityFilters(list, f))).toEqual(ids(applyActivityFilters(list, f)));
      })
    );
  });
});

describe("applyActivityFilters — combined filters mean intersection", () => {
  it("member ∧ app ∧ online returns exactly the intersection of the single-dimension results", () => {
    fc.assert(
      fc.property(
        listArb,
        fc.constantFrom(...userPool),
        fc.constantFrom(...appPool),
        fc.boolean(),
        (list, member, app, onlineOnly) => {
          const byMember = idSet(applyActivityFilters(list, { ...wide, memberId: member }));
          const byApp = idSet(applyActivityFilters(list, { ...wide, app }));
          const byOnline = idSet(applyActivityFilters(list, { ...wide, onlineOnly }));
          const combined = applyActivityFilters(list, { ...wide, memberId: member, app, onlineOnly });
          const expected = [...byMember].filter((id) => byApp.has(id) && byOnline.has(id)).sort();
          expect(ids(combined).sort()).toEqual(expected);
        }
      )
    );
  });

  it("an inverted productivity window (min > max) matches nothing", () => {
    fc.assert(
      fc.property(
        listArb,
        fc.integer({ min: 0, max: 99 }).chain((max) =>
          fc.tuple(fc.constant(max), fc.integer({ min: max + 1, max: 100 }))
        ),
        (list, [max, min]) => {
          expect(applyActivityFilters(list, { ...wide, minProductivity: min, maxProductivity: max })).toEqual([]);
        }
      )
    );
  });
});

describe("applyActivityFilters — sort contract holds for every mode on any input", () => {
  const endMs = (a: Activity) => new Date(a.endedAt).getTime();
  const activityScore = (a: Activity) => a.mouseClicks + a.keyboardHits;

  const orderings: Record<ActivitySort, (a: Activity, b: Activity) => boolean> = {
    recent: (a, b) => endMs(a) >= endMs(b),
    oldest: (a, b) => endMs(a) <= endMs(b),
    "prod-high": (a, b) => a.productivity >= b.productivity,
    "prod-low": (a, b) => a.productivity <= b.productivity,
    "active-high": (a, b) => activityScore(a) >= activityScore(b),
  };

  it("every adjacent pair of the output respects the selected ordering", () => {
    fc.assert(
      fc.property(listArb, filtersArb, (list, f) => {
        const out = applyActivityFilters(list, f);
        const inOrder = orderings[f.sort];
        for (let i = 1; i < out.length; i++) {
          expect(inOrder(out[i - 1], out[i])).toBe(true);
        }
      })
    );
  });
});
