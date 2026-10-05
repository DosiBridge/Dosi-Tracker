/* ============================================================================
 * FLOATING PANELS — where a menu or popover goes so it stays on screen.
 *
 * Pure geometry (no DOM, no React): given the trigger's box, the panel's
 * natural size and the viewport, pick the side with room (flip), slide the
 * panel along that side until it is inside the viewport (shift), and cap its
 * height and width so a long menu scrolls inside the screen instead of past
 * its edge. Coordinates are viewport pixels, as getBoundingClientRect gives
 * them, for a `position: fixed` panel. The selectors below are shared with
 * the dialog focus trap, so both agree on what is focusable and what is a layer.
 * ========================================================================== */

/** Elements a keyboard user can Tab to. */
export const FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

/**
 * Marks a floating panel (menu, popover) portaled to <body>. A dialog's focus
 * trap leaves Tab to a panel opened from inside it, which manages its own.
 */
export const FLOATING_LAYER_ATTRIBUTE = "data-floating-layer";
export const FLOATING_LAYER_SELECTOR = `[${FLOATING_LAYER_ATTRIBUTE}]`;

/** Which side of the trigger the panel opens on. */
export type FloatingSide = "top" | "bottom";

/** Which edge of the trigger the panel lines up with. */
export type FloatingAlign = "start" | "center" | "end";

export interface FloatingRect {
  top: number;
  left: number;
  width: number;
  height: number;
}

export interface FloatingInput {
  /** The trigger's box in viewport coordinates. */
  anchor: FloatingRect;
  /** The panel's natural size (before any cap). */
  floating: { width: number; height: number };
  viewport: { width: number; height: number };
  /** Preferred side (default "bottom"); the other one is used when it has more room. */
  side?: FloatingSide;
  /** Preferred alignment (default "start"); start and end swap when only the other fits. */
  align?: FloatingAlign;
  /** Gap between the trigger and the panel, px (default 8). */
  offset?: number;
  /** Closest the panel may come to the viewport's edges, px (default 8). */
  padding?: number;
}

export interface FloatingPosition {
  top: number;
  left: number;
  /** The side actually used. */
  side: FloatingSide;
  /** The alignment actually used. */
  align: FloatingAlign;
  /** Tallest the panel may be; taller content scrolls inside it. */
  maxHeight: number;
  /** Widest the panel may be (the viewport minus its padding). */
  maxWidth: number;
}

/** Below this, a panel squeezed into a short gap would be unusable: it may overlap the trigger instead. */
const MIN_USEFUL_HEIGHT = 160;

function clamp(value: number, min: number, max: number): number {
  // When the range is empty (a panel wider than the viewport) the start edge wins.
  return Math.max(min, Math.min(value, max));
}

function finite(value: number, fallback = 0): number {
  return Number.isFinite(value) ? value : fallback;
}

/** Where a floating panel goes: flipped to the side with room, shifted inside the viewport, capped to fit. */
export function computeFloatingPosition(input: FloatingInput): FloatingPosition {
  const offset = Math.max(0, finite(input.offset ?? 8));
  const padding = Math.max(0, finite(input.padding ?? 8));
  const viewport = { width: Math.max(0, finite(input.viewport.width)), height: Math.max(0, finite(input.viewport.height)) };
  const anchor = {
    top: finite(input.anchor.top),
    left: finite(input.anchor.left),
    width: Math.max(0, finite(input.anchor.width)),
    height: Math.max(0, finite(input.anchor.height)),
  };
  const natural = { width: Math.max(0, finite(input.floating.width)), height: Math.max(0, finite(input.floating.height)) };

  /* Vertical: the preferred side unless the panel doesn't fit there and the other side has more room. */
  const room = (side: FloatingSide) =>
    side === "bottom" ? viewport.height - (anchor.top + anchor.height) - offset - padding : anchor.top - offset - padding;
  const preferred: FloatingSide = input.side ?? "bottom";
  const other: FloatingSide = preferred === "bottom" ? "top" : "bottom";
  const side = natural.height > room(preferred) && room(other) > room(preferred) ? other : preferred;

  const screenHeight = Math.max(0, viewport.height - 2 * padding);
  const maxHeight = Math.floor(Math.min(screenHeight, Math.max(room(side), Math.min(natural.height, MIN_USEFUL_HEIGHT))));
  const height = Math.min(natural.height, maxHeight);
  const wanted = side === "bottom" ? anchor.top + anchor.height + offset : anchor.top - offset - height;
  const top = clamp(wanted, padding, viewport.height - padding - height);

  /* Horizontal: the preferred edge, else the opposite edge, else slid inside the viewport. */
  const maxWidth = Math.floor(Math.max(0, viewport.width - 2 * padding));
  const width = Math.min(natural.width, maxWidth);
  const leftFor = (align: FloatingAlign) =>
    align === "start"
      ? anchor.left
      : align === "end"
        ? anchor.left + anchor.width - width
        : anchor.left + anchor.width / 2 - width / 2;
  const fits = (left: number) => left >= padding && left + width <= viewport.width - padding;

  let align: FloatingAlign = input.align ?? "start";
  if (!fits(leftFor(align)) && align !== "center") {
    const swapped: FloatingAlign = align === "start" ? "end" : "start";
    if (fits(leftFor(swapped))) align = swapped;
  }
  const left = clamp(leftFor(align), padding, viewport.width - padding - width);

  return { top: Math.round(top), left: Math.round(left), side, align, maxHeight, maxWidth };
}

/** The CSS transform-origin that makes a panel grow out of its trigger ("top right" for a bottom-end panel). */
export function floatingOrigin(position: Pick<FloatingPosition, "side" | "align">): string {
  const vertical = position.side === "bottom" ? "top" : "bottom";
  const horizontal = position.align === "start" ? "left" : position.align === "end" ? "right" : "center";
  return `${vertical} ${horizontal}`;
}
