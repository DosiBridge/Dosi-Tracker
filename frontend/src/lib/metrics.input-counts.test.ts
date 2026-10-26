import { describe, expect, it } from "vitest";
import { INPUT_COUNT_METRIC, addCounts, formatCount, inputCountOf, serverCount, sumCounts } from "@/lib/metrics";

// Keyboard and mouse counts are null when not measured (consent off, a session
// that can't count input such as Wayland) and 0 only when measured as zero
// (contract §1, API-05). These helpers are what every table, chart and export
// reads them through.

describe("inputCountOf — a count off the wire", () => {
  it.each([
    [undefined, null],
    [null, null],
    ["", null],
    ["   ", null],
    ["x", null],
    [Number.NaN, null],
    [Number.POSITIVE_INFINITY, null],
  ])("reads %j as not measured", (raw, expected) => {
    expect(inputCountOf(raw)).toBe(expected);
  });

  it("keeps a measured zero, clamps negatives and rounds", () => {
    expect(inputCountOf(0)).toBe(0);
    expect(inputCountOf("0")).toBe(0);
    expect(inputCountOf(-4)).toBe(0);
    expect(inputCountOf(12.6)).toBe(13);
    expect(inputCountOf(" 42 ")).toBe(42);
  });
});

describe("addCounts / sumCounts — only measured values add up", () => {
  it("is null only when nothing was measured", () => {
    expect(addCounts(null, null)).toBeNull();
    expect(addCounts(undefined, null)).toBeNull();
    expect(addCounts(null, 0)).toBe(0);
    expect(addCounts(3, null)).toBe(3);
    expect(addCounts(3, 4)).toBe(7);
    expect(addCounts(Number.NaN, 2)).toBe(2);
  });

  it("sums a set, skipping the unmeasured members", () => {
    expect(sumCounts([])).toBeNull();
    expect(sumCounts([null, undefined, null])).toBeNull();
    expect(sumCounts([null, 0, null])).toBe(0);
    expect(sumCounts([5, null, 7])).toBe(12);
  });
});

describe("formatCount — what a cell shows", () => {
  it("shows the number, or a dash (or the given words) when not measured", () => {
    expect(formatCount(1204)).toBe("1,204");
    expect(formatCount(0)).toBe("0");
    expect(formatCount(null)).toBe(INPUT_COUNT_METRIC.placeholder);
    expect(formatCount(undefined)).toBe("—");
    expect(formatCount(null, String, INPUT_COUNT_METRIC.notMeasured)).toBe("Not measured");
    expect(formatCount(7, (n) => `${n} keys`)).toBe("7 keys");
  });
});

describe("serverCount — a server total read with how many blocks measured it", () => {
  it("is not measured when no block measured it, whatever the total says", () => {
    expect(serverCount(0, 0)).toBeNull();
    expect(serverCount(15, 0)).toBeNull();
  });

  it("keeps a measured total, zero included", () => {
    expect(serverCount(0, 3)).toBe(0);
    expect(serverCount(15, 2)).toBe(15);
  });

  it("keeps an older server's number when it doesn't say how many measured", () => {
    expect(serverCount(9, undefined)).toBe(9);
    expect(serverCount(0, null)).toBe(0);
    expect(serverCount(undefined, undefined)).toBeNull();
  });
});
