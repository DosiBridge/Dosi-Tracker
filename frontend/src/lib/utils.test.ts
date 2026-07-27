import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { cn, formatDuration, formatCompact, colorFromString, initials, scoreTone, timeAgo } from "./utils";

describe("cn (class merge)", () => {
  it("joins truthy classes and drops falsy ones", () => {
    expect(cn("a", false && "b", "c")).toBe("a c");
  });
  it("later tailwind utilities win over conflicting earlier ones", () => {
    expect(cn("p-2", "p-4")).toBe("p-4");
  });
});

describe("formatDuration", () => {
  it.each([
    [0, "0m"],
    [45, "45m"],
    [60, "1h"],
    [90, "1h 30m"],
    [125, "2h 5m"],
    [600, "10h"],
  ])("formats %i minutes as %s", (mins, expected) => {
    expect(formatDuration(mins)).toBe(expected);
  });
});

describe("formatCompact", () => {
  it("uses compact notation", () => {
    expect(formatCompact(1200)).toBe("1.2K");
    expect(formatCompact(3_400_000)).toBe("3.4M");
    expect(formatCompact(42)).toBe("42");
  });
});

describe("colorFromString", () => {
  it("is deterministic for the same input", () => {
    expect(colorFromString("Dosi")).toBe(colorFromString("Dosi"));
  });
  it("produces a valid HSL hue in [0,360)", () => {
    const m = colorFromString("anything").match(/^hsl\((\d+) 65% 55%\)$/);
    expect(m).not.toBeNull();
    const hue = Number(m![1]);
    expect(hue).toBeGreaterThanOrEqual(0);
    expect(hue).toBeLessThan(360);
  });
  it("honours custom saturation/lightness", () => {
    expect(colorFromString("x", 30, 40)).toMatch(/ 30% 40%\)$/);
  });
});

describe("initials", () => {
  it("takes the first letter of the first two words, uppercased", () => {
    expect(initials("Ada Lovelace")).toBe("AL");
    expect(initials("cher")).toBe("C");
    expect(initials("Jean  Luc  Picard")).toBe("JL"); // collapses extra spaces, max two
  });
  it("returns empty string for empty input", () => {
    expect(initials("")).toBe("");
  });
});

describe("scoreTone (boundaries)", () => {
  it.each([
    [100, "success"],
    [75, "success"],
    [74, "warning"],
    [50, "warning"],
    [49, "danger"],
    [0, "danger"],
  ])("score %i -> %s", (score, tone) => {
    expect(scoreTone(score)).toBe(tone);
  });
});

describe("timeAgo", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2024-06-01T12:00:00Z"));
  });
  afterEach(() => vi.useRealTimers());

  it.each([
    [20_000, "just now"], // rounds to 0 min
    [5 * 60_000, "5m ago"],
    [2 * 3_600_000, "2h ago"],
    [3 * 86_400_000, "3d ago"],
  ])("renders %i ms ago as %s", (deltaMs, expected) => {
    expect(timeAgo(new Date(Date.now() - deltaMs))).toBe(expected);
  });
});
