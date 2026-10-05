// The dashboard charts' minutes axis: round ticks from zero that always cover
// the largest value — "0, 10h, 20h, 30h, 40h", never "8.3h, 16.7h, 25h".
import { describe, expect, it } from "vitest";
import fc from "fast-check";
import { minuteAxis } from "./charts";

describe("minuteAxis", () => {
  it("steps in round hours on a team's week", () => {
    expect(minuteAxis(2000)).toEqual({ ticks: [0, 600, 1200, 1800, 2400], top: 2400 });
    expect(minuteAxis(480)).toEqual({ ticks: [0, 120, 240, 360, 480], top: 480 });
  });

  it("steps in round minutes under an hour", () => {
    expect(minuteAxis(45)).toEqual({ ticks: [0, 15, 30, 45], top: 45 });
    expect(minuteAxis(61)).toEqual({ ticks: [0, 30, 60, 90], top: 90 });
  });

  it("draws a small axis for an empty chart, and treats nonsense as nothing", () => {
    expect(minuteAxis(0)).toEqual({ ticks: [0, 5], top: 5 });
    expect(minuteAxis(Number.NaN)).toEqual({ ticks: [0, 5], top: 5 });
    expect(minuteAxis(-30)).toEqual({ ticks: [0, 5], top: 5 });
  });

  it("always covers the value with at most five evenly spaced ticks from zero", () => {
    fc.assert(
      fc.property(fc.double({ min: 0, max: 1e7, noNaN: true }), (max) => {
        const { ticks, top } = minuteAxis(max);
        expect(ticks[0]).toBe(0);
        expect(ticks.length).toBeLessThanOrEqual(5);
        expect(top).toBeGreaterThanOrEqual(max);
        const step = ticks[1] - ticks[0];
        ticks.forEach((t, i) => expect(t).toBe(i * step));
      }),
    );
  });
});
