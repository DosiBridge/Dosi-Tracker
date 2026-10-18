// Automated accessibility regression tests for analytics-charts:
// Verifies that every colour defined in PLATFORM_COLORS meets or exceeds the
// WCAG 2.1 non-text/large contrast ratio requirement (>= 3:1) against both
// light and dark theme card background tokens (--card in globals.css).
import { describe, expect, it } from "vitest";
import { PLATFORM_COLORS } from "./analytics-charts";

/** WCAG relative-luminance contrast of two #rrggbb colours. */
function contrast(a: string, b: string): number {
  const luminance = (hex: string) => {
    const [r, g, bl] = [1, 3, 5].map((i) => {
      const c = parseInt(hex.slice(i, i + 2), 16) / 255;
      return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
  };
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

// Actual card background design tokens from src/app/globals.css:
// :root  --card: #ffffff
// .dark  --card: #1a2226
const LIGHT_CARD_BG = "#ffffff";
const DARK_CARD_BG = "#1a2226";

describe("PLATFORM_COLORS a11y contrast", () => {
  it("defines keys for all supported platforms and unknown fallback", () => {
    expect(Object.keys(PLATFORM_COLORS).sort()).toEqual(["linux", "macos", "unknown", "windows"]);
  });

  const entries = Object.entries(PLATFORM_COLORS) as [keyof typeof PLATFORM_COLORS, string][];

  it.each(entries)(
    "platform colour %s (%s) has >= 3:1 contrast against light card (%s) and dark card (%s)",
    (platform, hex) => {
      expect(hex).toMatch(/^#[0-9a-f]{6}$/i);

      const lightRatio = contrast(hex, LIGHT_CARD_BG);
      const darkRatio = contrast(hex, DARK_CARD_BG);

      expect(
        lightRatio,
        `PLATFORM_COLORS.${platform} (${hex}) contrast on light card (${LIGHT_CARD_BG}) is ${lightRatio.toFixed(3)}:1, must be >= 3:1`,
      ).toBeGreaterThanOrEqual(3);

      expect(
        darkRatio,
        `PLATFORM_COLORS.${platform} (${hex}) contrast on dark card (${DARK_CARD_BG}) is ${darkRatio.toFixed(3)}:1, must be >= 3:1`,
      ).toBeGreaterThanOrEqual(3);
    },
  );
});
