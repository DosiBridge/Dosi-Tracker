// Automated accessibility regression tests for analytics-charts:
// Verifies that every colour defined in PLATFORM_COLORS meets or exceeds the
// WCAG 2.1 non-text/large contrast ratio requirement (>= 3:1) against the
// actual light and dark card background design tokens parsed from globals.css.
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { PLATFORM_COLORS } from "./analytics-charts";

/** WCAG relative-luminance contrast of two #rrggbb colours. */
export function contrast(a: string, b: string): number {
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

export interface CardTokens {
  rootCard: string;
  darkCard: string;
}

/**
 * Extracts the real --card background tokens from globals.css:
 * - :root { ... --card: #ffffff; ... }
 * - .dark { ... --card: #1a2226; ... }
 *
 * Ensures compound selectors and media-query print overrides (e.g. .dark .work-report { --card: #ffffff; })
 * are not accidentally selected.
 */
export function extractCardTokens(css: string): CardTokens {
  const rootBlockMatch = css.match(/(?:^|[\n;}])\s*:root\s*\{([^}]+)\}/);
  if (!rootBlockMatch) {
    throw new Error("Cannot parse :root selector block from globals.css");
  }
  const rootCardMatch = rootBlockMatch[1].match(/--card\s*:\s*([^;]+);/);
  if (!rootCardMatch) {
    throw new Error("Cannot find '--card' CSS property definition inside :root block in globals.css");
  }
  const rootCard = rootCardMatch[1].trim();
  if (!/^#[0-9a-fA-F]{6}$/.test(rootCard)) {
    throw new Error(`Unrecognized or non-6-digit-hex --card color in :root: '${rootCard}'`);
  }

  const darkBlockMatch = css.match(/(?:^|[\n;}])\s*\.dark\s*\{([^}]+)\}/);
  if (!darkBlockMatch) {
    throw new Error("Cannot parse top-level .dark selector block from globals.css");
  }
  const darkCardMatch = darkBlockMatch[1].match(/--card\s*:\s*([^;]+);/);
  if (!darkCardMatch) {
    throw new Error("Cannot find '--card' CSS property definition inside .dark block in globals.css");
  }
  const darkCard = darkCardMatch[1].trim();
  if (!/^#[0-9a-fA-F]{6}$/.test(darkCard)) {
    throw new Error(`Unrecognized or non-6-digit-hex --card color in .dark: '${darkCard}'`);
  }

  return { rootCard, darkCard };
}

const CURRENT_DIR = path.dirname(fileURLToPath(import.meta.url));
const GLOBALS_CSS_PATH = path.resolve(CURRENT_DIR, "../../app/globals.css");
const rawGlobalsCss = readFileSync(GLOBALS_CSS_PATH, "utf8");
const actualCardTokens = extractCardTokens(rawGlobalsCss);

describe("a11y: PLATFORM_COLORS contrast verification", () => {
  it("a11y: resolves real :root and .dark --card design tokens directly from globals.css", () => {
    expect(actualCardTokens.rootCard).toBe("#ffffff");
    expect(actualCardTokens.darkCard).toBe("#1a2226");
  });

  it("a11y: guards against stale literals by deriving tokens dynamically from CSS content", () => {
    const mockCss = `
      :root { --card: #112233; }
      .dark { --card: #445566; }
    `;
    const derived = extractCardTokens(mockCss);
    expect(derived.rootCard).toBe("#112233");
    expect(derived.darkCard).toBe("#445566");
  });

  it("a11y: fails meaningfully if :root or .dark --card token is missing or malformed", () => {
    expect(() => extractCardTokens("body { color: red; }")).toThrowError("Cannot parse :root selector block");
    expect(() => extractCardTokens(":root { color: red; }")).toThrowError("Cannot find '--card' CSS property");
    expect(() => extractCardTokens(":root { --card: #ffffff; }")).toThrowError("Cannot parse top-level .dark selector block");
    expect(() => extractCardTokens(":root { --card: #ffffff; } .dark { color: blue; }")).toThrowError("Cannot find '--card' CSS property");
    expect(() => extractCardTokens(":root { --card: not-a-hex; } .dark { --card: #1a2226; }")).toThrowError("Unrecognized or non-6-digit-hex");
  });

  it("a11y: guards against accidentally picking the later print override .dark .work-report", () => {
    const mockPrintCss = `
      :root { --card: #ffffff; }
      .dark { --card: #1a2226; }
      @media print {
        .dark .work-report { --card: #ffffff; }
      }
    `;
    const derived = extractCardTokens(mockPrintCss);
    expect(derived.darkCard).toBe("#1a2226");
    expect(derived.darkCard).not.toBe("#ffffff");
  });

  it("a11y: defines keys for all supported platforms and unknown fallback", () => {
    expect(Object.keys(PLATFORM_COLORS).sort()).toEqual(["linux", "macos", "unknown", "windows"]);
  });

  const entries = Object.entries(PLATFORM_COLORS) as [keyof typeof PLATFORM_COLORS, string][];

  it.each(entries)(
    "a11y: platform colour %s (%s) has >= 3:1 contrast against actual CSS card backgrounds (%s, %s)",
    (platform, hex) => {
      expect(hex).toMatch(/^#[0-9a-f]{6}$/i);

      const lightRatio = contrast(hex, actualCardTokens.rootCard);
      const darkRatio = contrast(hex, actualCardTokens.darkCard);

      expect(
        lightRatio,
        `PLATFORM_COLORS.${platform} (${hex}) contrast on actual :root card (${actualCardTokens.rootCard}) is ${lightRatio.toFixed(3)}:1, must be >= 3:1`,
      ).toBeGreaterThanOrEqual(3);

      expect(
        darkRatio,
        `PLATFORM_COLORS.${platform} (${hex}) contrast on actual .dark card (${actualCardTokens.darkCard}) is ${darkRatio.toFixed(3)}:1, must be >= 3:1`,
      ).toBeGreaterThanOrEqual(3);
    },
  );
});
