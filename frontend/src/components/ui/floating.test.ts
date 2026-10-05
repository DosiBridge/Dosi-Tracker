// Where a floating panel goes: below its trigger when there is room, flipped
// above when there isn't, lined up with the trigger's edge, slid inside the
// viewport on a phone, and capped so a long menu scrolls inside the screen.
import { describe, expect, it } from "vitest";
import fc from "fast-check";
import { computeFloatingPosition, floatingOrigin, type FloatingAlign, type FloatingSide } from "./floating";

const DESKTOP = { width: 1440, height: 900 };
const PHONE = { width: 360, height: 640 };

/** A 40px-tall trigger at (left, top), `width` wide. */
const button = (left: number, top: number, width = 120) => ({ left, top, width, height: 40 });

describe("computeFloatingPosition", () => {
  it("opens below the trigger, on its start edge, 8px away", () => {
    const at = computeFloatingPosition({ anchor: button(600, 100), floating: { width: 224, height: 180 }, viewport: DESKTOP });
    expect(at).toMatchObject({ side: "bottom", align: "start", top: 148, left: 600 });
  });

  it("lines the panel up with the trigger's end or centre", () => {
    const end = computeFloatingPosition({ anchor: button(600, 100), floating: { width: 224, height: 180 }, viewport: DESKTOP, align: "end" });
    expect(end.left).toBe(600 + 120 - 224);
    const center = computeFloatingPosition({ anchor: button(600, 100), floating: { width: 224, height: 180 }, viewport: DESKTOP, align: "center" });
    expect(center.left).toBe(600 + 60 - 112);
  });

  it("flips above a trigger near the bottom of the screen", () => {
    const at = computeFloatingPosition({ anchor: button(600, 800), floating: { width: 224, height: 300 }, viewport: DESKTOP });
    expect(at.side).toBe("top");
    expect(at.top).toBe(800 - 8 - 300);
  });

  it("flips below when asked for the top but there is no room above", () => {
    const at = computeFloatingPosition({ anchor: button(600, 30), floating: { width: 224, height: 200 }, viewport: DESKTOP, side: "top" });
    expect(at).toMatchObject({ side: "bottom", top: 78 });
  });

  it("stays on the roomier side when neither fits, and caps the height so the rest scrolls", () => {
    const at = computeFloatingPosition({ anchor: button(20, 300), floating: { width: 224, height: 2000 }, viewport: PHONE });
    expect(at.side).toBe("bottom");
    expect(at.maxHeight).toBe(640 - 340 - 8 - 8);
    expect(at.top).toBe(348);
  });

  it("never caps a panel below a usable height: it overlaps the trigger instead of shrinking to a sliver", () => {
    // A landscape phone: only 84px of room either side of a tall trigger.
    const at = computeFloatingPosition({ anchor: { left: 20, top: 100, width: 120, height: 160 }, floating: { width: 224, height: 400 }, viewport: { width: 740, height: 360 } });
    expect(at.maxHeight).toBe(160);
    expect(at.top + at.maxHeight).toBeLessThanOrEqual(360 - 8);
    expect(at.top).toBeGreaterThanOrEqual(8);
  });

  it("swaps an end-aligned panel to the start edge when it would run off the left (a trigger by the sidebar)", () => {
    const at = computeFloatingPosition({ anchor: button(16, 100, 90), floating: { width: 224, height: 180 }, viewport: PHONE, align: "end" });
    expect(at).toMatchObject({ align: "start", left: 16 });
  });

  it("swaps a start-aligned panel to the end edge near the right of the screen", () => {
    const at = computeFloatingPosition({ anchor: button(1300, 100), floating: { width: 288, height: 180 }, viewport: DESKTOP });
    expect(at).toMatchObject({ align: "end", left: 1300 + 120 - 288 });
  });

  it("slides the panel inside the viewport when neither edge fits", () => {
    const at = computeFloatingPosition({ anchor: button(150, 100, 60), floating: { width: 300, height: 180 }, viewport: PHONE });
    expect(at.left).toBe(360 - 8 - 300);
    const centered = computeFloatingPosition({ anchor: button(0, 100, 40), floating: { width: 300, height: 180 }, viewport: PHONE, align: "center" });
    expect(centered).toMatchObject({ align: "center", left: 8 });
  });

  it("caps a panel wider than the phone to the screen, minus the padding", () => {
    const at = computeFloatingPosition({ anchor: button(200, 100), floating: { width: 600, height: 180 }, viewport: PHONE });
    expect(at.maxWidth).toBe(344);
    expect(at.left).toBe(8);
  });

  it("honours a custom gap and edge padding", () => {
    const at = computeFloatingPosition({ anchor: button(0, 100), floating: { width: 200, height: 100 }, viewport: DESKTOP, offset: 4, padding: 16 });
    expect(at).toMatchObject({ top: 144, left: 16, maxWidth: 1440 - 32 });
  });

  it("turns unreadable measurements into numbers, never NaN", () => {
    const at = computeFloatingPosition({
      anchor: { left: Number.NaN, top: Number.POSITIVE_INFINITY, width: -5, height: Number.NaN },
      floating: { width: Number.NaN, height: -1 },
      viewport: { width: 0, height: Number.NaN },
    });
    for (const value of [at.top, at.left, at.maxHeight, at.maxWidth]) expect(Number.isFinite(value)).toBe(true);
  });

  it("keeps any panel that fits the screen entirely inside it", () => {
    const sides = fc.constantFrom<FloatingSide>("top", "bottom");
    const aligns = fc.constantFrom<FloatingAlign>("start", "center", "end");
    fc.assert(
      fc.property(
        fc.integer({ min: 320, max: 2560 }),
        fc.integer({ min: 480, max: 1600 }),
        fc.record({ x: fc.double({ min: 0, max: 1, noNaN: true }), y: fc.double({ min: 0, max: 1, noNaN: true }) }),
        fc.integer({ min: 24, max: 240 }),
        fc.integer({ min: 40, max: 900 }),
        fc.integer({ min: 20, max: 2400 }),
        sides,
        aligns,
        (vw, vh, spot, triggerWidth, panelWidth, panelHeight, side, align) => {
          const anchor = { left: spot.x * (vw - triggerWidth), top: spot.y * (vh - 40), width: triggerWidth, height: 40 };
          const at = computeFloatingPosition({ anchor, floating: { width: panelWidth, height: panelHeight }, viewport: { width: vw, height: vh }, side, align });
          const width = Math.min(panelWidth, at.maxWidth);
          const height = Math.min(panelHeight, at.maxHeight);
          expect(at.left).toBeGreaterThanOrEqual(8);
          expect(at.left + width).toBeLessThanOrEqual(vw - 8 + 0.5);
          expect(at.top).toBeGreaterThanOrEqual(8);
          expect(at.top + height).toBeLessThanOrEqual(vh - 8 + 0.5);
          expect(at.maxHeight).toBeLessThanOrEqual(vh - 16);
        },
      ),
      { numRuns: 400 },
    );
  });
});

describe("floatingOrigin", () => {
  it("grows the panel out of the trigger's corner", () => {
    expect(floatingOrigin({ side: "bottom", align: "end" })).toBe("top right");
    expect(floatingOrigin({ side: "top", align: "start" })).toBe("bottom left");
    expect(floatingOrigin({ side: "bottom", align: "center" })).toBe("top center");
  });
});
