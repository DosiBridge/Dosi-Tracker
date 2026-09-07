import { describe, it, expect } from "vitest";
import {
  categoryColor,
  filterActivitiesByRange,
  rangeForKey,
  rangePresets,
  reportCatalog,
  type AppCategory,
  type RangeKey,
} from "./reports-data";
import { NOW } from "./mock-data";
import { buildActivity } from "@/test/factories";
import type { Activity } from "./types";

/* ----------------------------------------------------------------------------
 * rangeForKey pivots on the FROZEN demo clock (NOW) and uses local-calendar
 * setters (setDate/setHours), so a preset means "today/this month *for the
 * viewer*". Expected boundaries are therefore rebuilt from NOW's own local
 * calendar parts with the Date(y, m, d, …) constructor — an independent path to
 * the same instant that stays correct in any CI timezone.
 * -------------------------------------------------------------------------- */

const Y = NOW.getFullYear();
const M = NOW.getMonth();
const D = NOW.getDate();

/** A local wall-clock instant on day `day` of NOW's month (day may under/overflow). */
const localDay = (day: number, h = 0, mi = 0, s = 0, ms = 0) => new Date(Y, M, day, h, mi, s, ms);

/** NOW's local time-of-day, used by the rolling ("last N days") presets. */
const nowTimeOfDay = [NOW.getHours(), NOW.getMinutes(), NOW.getSeconds(), NOW.getMilliseconds()] as const;

describe("rangeForKey — exact boundaries against the frozen demo clock", () => {
  it("'today' runs from local midnight to the current instant", () => {
    const { from, to, label } = rangeForKey("today");
    expect(from.getTime()).toBe(localDay(D, 0, 0, 0, 0).getTime());
    expect(to.getTime()).toBe(NOW.getTime());
    expect(label).toBe("Today");

    // The lower bound is the *start* of the day, to the millisecond.
    expect([from.getHours(), from.getMinutes(), from.getSeconds(), from.getMilliseconds()]).toEqual([0, 0, 0, 0]);
    expect(from.getDate()).toBe(D);
  });

  it("'yesterday' covers the whole previous local day, 00:00:00.000 → 23:59:59.999", () => {
    const { from, to, label } = rangeForKey("yesterday");
    expect(from.getTime()).toBe(localDay(D - 1, 0, 0, 0, 0).getTime());
    expect(to.getTime()).toBe(localDay(D - 1, 23, 59, 59, 999).getTime());
    expect(label).toBe("Yesterday");

    // It must end exactly one millisecond before today starts — no gap, no overlap.
    expect(to.getTime()).toBe(rangeForKey("today").from.getTime() - 1);
    // …and it is strictly BEFORE today, never after it.
    expect(to.getTime()).toBeLessThan(NOW.getTime());
  });

  it("'7d' is a rolling window: exactly 7 calendar days back at the same time of day", () => {
    const { from, to, label } = rangeForKey("7d");
    expect(from.getTime()).toBe(localDay(D - 7, ...nowTimeOfDay).getTime());
    expect(to.getTime()).toBe(NOW.getTime());
    expect(label).toBe("Last 7 days");

    // Rolling, not calendar-aligned: the time of day is carried over, not zeroed.
    expect([from.getHours(), from.getMinutes()]).toEqual([NOW.getHours(), NOW.getMinutes()]);
  });

  it("'30d' reaches back 30 calendar days, four times further than '7d'", () => {
    const { from, to, label } = rangeForKey("30d");
    expect(from.getTime()).toBe(localDay(D - 30, ...nowTimeOfDay).getTime());
    expect(to.getTime()).toBe(NOW.getTime());
    expect(label).toBe("Last 30 days");
    expect(from.getTime()).toBeLessThan(rangeForKey("7d").from.getTime());
  });

  it("'month' starts on the 1st of the current month at midnight", () => {
    const { from, to, label } = rangeForKey("month");
    expect(from.getTime()).toBe(localDay(1, 0, 0, 0, 0).getTime());
    expect(to.getTime()).toBe(NOW.getTime());
    expect(label).toBe("This month");

    expect(from.getDate()).toBe(1);
    expect(from.getMonth()).toBe(M);
    expect([from.getHours(), from.getMinutes(), from.getSeconds(), from.getMilliseconds()]).toEqual([0, 0, 0, 0]);
  });

  it("'custom' falls back to a 7-day window but keeps its own label", () => {
    const { from, to, label } = rangeForKey("custom");
    expect(from.getTime()).toBe(localDay(D - 7, ...nowTimeOfDay).getTime());
    expect(to.getTime()).toBe(NOW.getTime());
    expect(label).toBe("Custom");
    // Same window as "7d", but the picker must not mislabel it.
    expect(label).not.toBe(rangeForKey("7d").label);
  });

  it("every preset yields a non-inverted window and a distinct label", () => {
    const keys: RangeKey[] = ["today", "yesterday", "7d", "30d", "month", "custom"];
    const labels = keys.map((k) => {
      const { from, to, label } = rangeForKey(k);
      expect(from.getTime()).toBeLessThanOrEqual(to.getTime());
      return label;
    });
    expect(new Set(labels).size).toBe(keys.length);
  });

  it("hands back fresh Dates so callers cannot corrupt the frozen clock", () => {
    const frozen = NOW.getTime();
    const r = rangeForKey("today");
    r.from.setFullYear(1999);
    r.to.setFullYear(1999);
    expect(NOW.getTime()).toBe(frozen);
    expect(rangeForKey("today").to.getTime()).toBe(frozen);
  });
});

/* ----------------------------- filterActivitiesByRange ----------------------------- */

describe("filterActivitiesByRange — inclusive on both ends", () => {
  const from = new Date("2026-06-10T00:00:00.000Z");
  const to = new Date("2026-06-12T23:59:59.999Z");

  const at = (iso: string, id: string): Activity => buildActivity({ id, endedAt: iso });

  const before = at("2026-06-09T23:59:59.999Z", "before");
  const atFrom = at("2026-06-10T00:00:00.000Z", "at-from");
  const middle = at("2026-06-11T12:00:00.000Z", "middle");
  const atTo = at("2026-06-12T23:59:59.999Z", "at-to");
  const after = at("2026-06-13T00:00:00.000Z", "after");
  const list = [before, atFrom, middle, atTo, after];

  const ids = (rows: Activity[]) => rows.map((a) => a.id);

  it("keeps rows landing exactly on either bound and drops the millisecond outside", () => {
    expect(ids(filterActivitiesByRange(list, from, to))).toEqual(["at-from", "middle", "at-to"]);
  });

  it("a row one millisecond before `from` is excluded", () => {
    expect(ids(filterActivitiesByRange([before, atFrom], from, to))).toEqual(["at-from"]);
  });

  it("a row one millisecond after `to` is excluded", () => {
    expect(ids(filterActivitiesByRange([atTo, after], from, to))).toEqual(["at-to"]);
  });

  it("a single-instant range matches only the activity ending on that instant", () => {
    const instant = new Date("2026-06-11T12:00:00.000Z");
    expect(ids(filterActivitiesByRange(list, instant, instant))).toEqual(["middle"]);
  });

  it("an inverted range (to before from) matches nothing", () => {
    expect(filterActivitiesByRange(list, to, from)).toEqual([]);
  });

  it("preserves input order and never mutates the caller's array", () => {
    const original = [...list];
    const out = filterActivitiesByRange(list, from, to);
    expect(list).toEqual(original);
    expect(out).not.toBe(list);
    expect(ids(out)).toEqual(ids(list.filter((a) => out.includes(a))));
  });
});

/* ----------------------------- Catalog / palette contracts ----------------------------- */

describe("categoryColor — the productivity palette the charts and legends render", () => {
  it("pins one distinct hex per category", () => {
    expect(categoryColor).toEqual({
      productive: "#22c55e",
      neutral: "#0ea5e9",
      unproductive: "#ef4444",
    });
  });

  it("every value is a full 6-digit hex, so `${color}1a` alpha suffixes stay valid CSS", () => {
    const categories: AppCategory[] = ["productive", "neutral", "unproductive"];
    for (const c of categories) expect(categoryColor[c]).toMatch(/^#[0-9a-f]{6}$/);
    expect(new Set(categories.map((c) => categoryColor[c])).size).toBe(3);
  });
});

describe("rangePresets — the date picker's option list", () => {
  it("offers every preset in display order with its human label", () => {
    expect(rangePresets).toEqual([
      { key: "today", label: "Today" },
      { key: "yesterday", label: "Yesterday" },
      { key: "7d", label: "Last 7 days" },
      { key: "30d", label: "Last 30 days" },
      { key: "month", label: "This month" },
      { key: "custom", label: "Custom" },
    ]);
  });

  it("each option's label is the label rangeForKey resolves for that key", () => {
    // date-range-picker.tsx shows the preset label on the trigger, then applies
    // rangeForKey(key) — the two must never disagree.
    expect(rangePresets).not.toHaveLength(0);
    for (const preset of rangePresets) {
      expect(rangeForKey(preset.key).label).toBe(preset.label);
    }
  });
});

describe("reportCatalog — the report hub + /reports/[slug] routing contract", () => {
  // Slugs mirror the component map in src/app/(dashboard)/reports/[slug]/page.tsx;
  // a slug that is not a key there renders notFound().
  const routableSlugs = ["time-activity", "productivity", "apps-urls", "projects", "attendance", "weekly", "payroll"];

  it("exposes exactly the slugs that have a report page, in hub order", () => {
    expect(reportCatalog.map((r) => r.slug)).toEqual(routableSlugs);
  });

  it("gives every slug a URL-safe, unique identifier", () => {
    for (const r of reportCatalog) expect(r.slug).toMatch(/^[a-z][a-z0-9-]*$/);
    expect(new Set(reportCatalog.map((r) => r.slug)).size).toBe(reportCatalog.length);
  });

  it("titles each report exactly as the card and page heading show it", () => {
    expect(Object.fromEntries(reportCatalog.map((r) => [r.slug, r.title]))).toEqual({
      "time-activity": "Time & Activity",
      productivity: "Productivity",
      "apps-urls": "Apps & Websites",
      projects: "Project Breakdown",
      attendance: "Attendance & Shifts",
      weekly: "Weekly Summary",
      payroll: "Payroll & Billing",
    });
  });

  it("gives every report a distinct, non-empty description for its card body", () => {
    for (const r of reportCatalog) {
      expect(r.description.trim().length).toBeGreaterThan(0);
      expect(r.title.trim().length).toBeGreaterThan(0);
    }
    expect(new Set(reportCatalog.map((r) => r.description)).size).toBe(reportCatalog.length);
    expect(new Set(reportCatalog.map((r) => r.title)).size).toBe(reportCatalog.length);
  });

  it("files every report under a category the hub actually renders a section for", () => {
    // reports/page.tsx iterates this exact list; an unlisted category hides the report.
    const sections = ["Time", "Productivity", "People", "Finance"];
    const counts = sections.map((s) => reportCatalog.filter((r) => r.category === s).length);
    expect(counts).toEqual([3, 2, 1, 1]);
    expect(counts.reduce((a, b) => a + b, 0)).toBe(reportCatalog.length);
  });

  it("names an icon that the hub's lucide registry can resolve (never the FileText fallback)", () => {
    // Keys of the `icons` map in reports/page.tsx.
    const registered = ["Clock", "Gauge", "AppWindow", "FolderKanban", "CalendarCheck", "CalendarDays", "DollarSign"];
    expect(reportCatalog.map((r) => r.icon)).toEqual([
      "Clock",
      "Gauge",
      "AppWindow",
      "FolderKanban",
      "CalendarCheck",
      "CalendarDays",
      "DollarSign",
    ]);
    for (const r of reportCatalog) expect(registered).toContain(r.icon);
  });
});
