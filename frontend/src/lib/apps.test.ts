// App identity: one canonical name per app however an agent spelled it (the
// contract §8 alias table, mirrored by the backend's reporting service), a
// colour that follows the app rather than its rank and reads on both themes'
// cards, and the productivity category the Apps report and dashboard use.
import { describe, expect, it } from "vitest";
import fc from "fast-check";
import {
  APP_CATEGORIES,
  APP_CATEGORY_COLORS,
  APP_CATEGORY_LABELS,
  APP_PALETTE,
  OTHER_APP_COLOR,
  OTHER_APPS_NAME,
  UNKNOWN_APP_NAME,
  appCategory,
  appColor,
  appKey,
  canonicalAppName,
  friendlyAppName,
  isClassifiedApp,
} from "./apps";
import { categoryColor } from "./reports-data";

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

describe("canonicalAppName — the contract §8 alias table", () => {
  it.each([
    ["chrome.exe", "Google Chrome"],
    ["Google Chrome", "Google Chrome"],
    ["msedge.exe", "Microsoft Edge"],
    ["Code.exe", "Visual Studio Code"],
    ["Code", "Visual Studio Code"],
    ["firefox.exe", "Firefox"],
    ["slack.exe", "Slack"],
    ["teams.exe", "Microsoft Teams"],
    ["ms-teams.exe", "Microsoft Teams"],
    ["outlook.exe", "Outlook"],
    ["winword.exe", "Word"],
    ["excel.exe", "Excel"],
    ["powerpnt.exe", "PowerPoint"],
    ["explorer.exe", "File Explorer"],
    ["Finder", "Finder"],
    ["WindowsTerminal.exe", "Windows Terminal"],
    ["Terminal", "Terminal"],
    ["iTerm2", "iTerm2"],
    ["devenv.exe", "Visual Studio"],
    ["idea64.exe", "IntelliJ IDEA"],
    ["rider64.exe", "Rider"],
    ["figma.exe", "Figma"],
    ["Figma", "Figma"],
    ["zoom.exe", "Zoom"],
    ["zoom.us", "Zoom"],
    ["notepad.exe", "Notepad"],
    ["ApplicationFrameHost.exe", "Windows app"],
    ["LockApp.exe", "Lock screen"],
  ])("%s → %s", (raw, expected) => {
    expect(canonicalAppName(raw)).toBe(expected);
  });

  it("matches aliases and the .exe suffix case-insensitively, ignoring surrounding whitespace", () => {
    expect(canonicalAppName("CHROME.EXE")).toBe("Google Chrome");
    expect(canonicalAppName("  WindowsTerminal.exe ")).toBe("Windows Terminal");
    expect(canonicalAppName("code.Exe")).toBe("Visual Studio Code");
    expect(canonicalAppName("ITERM2")).toBe("iTerm2");
  });

  it("title-cases any other stem, keeping the capitals it already has", () => {
    expect(canonicalAppName("myapp.exe")).toBe("Myapp");
    expect(canonicalAppName("Antigravity IDE.exe")).toBe("Antigravity IDE");
    expect(canonicalAppName("antigravity ide.exe")).toBe("Antigravity Ide");
    expect(canonicalAppName("WhatsApp.exe")).toBe("WhatsApp");
    expect(canonicalAppName("OneDrive.exe")).toBe("OneDrive");
    expect(canonicalAppName("notepad++.exe")).toBe("Notepad++");
    expect(canonicalAppName("obs64.exe")).toBe("Obs64");
  });

  it("collapses runs of whitespace and strips every trailing .exe", () => {
    expect(canonicalAppName("my   tool.exe")).toBe("My Tool");
    expect(canonicalAppName("tool.exe.exe")).toBe("Tool");
    expect(canonicalAppName("tool .exe")).toBe("Tool");
  });

  it("treats names that collide with JavaScript built-ins as ordinary apps", () => {
    for (const raw of ["constructor", "toString.exe", "__proto__", "hasOwnProperty", "valueOf"]) {
      const name = canonicalAppName(raw);
      expect(typeof name).toBe("string");
      expect(name.toLowerCase()).toBe(raw.replace(/\.exe$/i, "").toLowerCase());
      expect(APP_PALETTE).toContain(appColor(raw));
      expect(appCategory(raw)).toBe("neutral");
      expect(isClassifiedApp(raw)).toBe(false);
    }
    expect(canonicalAppName("constructor")).toBe("Constructor");
  });

  it("is empty for a missing name", () => {
    expect(canonicalAppName("")).toBe("");
    expect(canonicalAppName("   ")).toBe("");
    expect(canonicalAppName(".exe")).toBe("");
    expect(canonicalAppName(null)).toBe("");
    expect(canonicalAppName(undefined)).toBe("");
  });

  it("leaves names the server already canonicalised exactly as they are", () => {
    for (const name of ["Windows app", "Lock screen", "iTerm2", "IntelliJ IDEA", "Visual Studio Code", "PowerPoint"]) {
      expect(canonicalAppName(name)).toBe(name);
    }
  });

  it("is idempotent for any input (canonicalising twice changes nothing)", () => {
    const known = fc.constantFrom("chrome.exe", "Code", "zoom.us", "ApplicationFrameHost.exe", "LockApp.exe", "iTerm2", "ß-tool.exe");
    fc.assert(
      fc.property(fc.oneof(fc.string(), fc.string({ unit: "grapheme" }), known), (raw) => {
        const once = canonicalAppName(raw);
        expect(canonicalAppName(once)).toBe(once);
      }),
      { numRuns: 300 },
    );
  });
});

describe("friendlyAppName / appKey", () => {
  it("shows the canonical name, or 'Unknown app' when there is none", () => {
    expect(friendlyAppName("msedge.exe")).toBe("Microsoft Edge");
    expect(friendlyAppName("")).toBe(UNKNOWN_APP_NAME);
    expect(friendlyAppName(null)).toBe("Unknown app");
    expect(friendlyAppName(UNKNOWN_APP_NAME)).toBe(UNKNOWN_APP_NAME);
    expect(friendlyAppName(OTHER_APPS_NAME)).toBe("Other apps");
  });

  it("groups every spelling of one app under one key", () => {
    expect(appKey("chrome.exe")).toBe("google chrome");
    expect(appKey("Google Chrome")).toBe(appKey("CHROME.EXE"));
    expect(appKey("MyTool.exe")).toBe(appKey("mytool.EXE"));
    expect(appKey("")).toBe("");
  });
});

describe("appColor", () => {
  it("is the same for every spelling of an app, on every call", () => {
    const chrome = appColor("chrome.exe");
    expect(appColor("Google Chrome")).toBe(chrome);
    expect(appColor("CHROME.EXE")).toBe(chrome);
    expect(appColor("my-internal-tool.exe")).toBe(appColor("My-internal-tool"));
  });

  it("always comes from the palette, and unnamed apps and the 'Other apps' bucket are neutral", () => {
    fc.assert(
      fc.property(fc.string({ minLength: 1 }), (raw) => {
        const color = appColor(raw);
        expect([...APP_PALETTE, OTHER_APP_COLOR]).toContain(color);
      }),
      { numRuns: 200 },
    );
    expect(appColor("")).toBe(OTHER_APP_COLOR);
    expect(appColor(undefined)).toBe(OTHER_APP_COLOR);
    expect(appColor(OTHER_APPS_NAME)).toBe(OTHER_APP_COLOR);
    expect(appColor(UNKNOWN_APP_NAME)).toBe(OTHER_APP_COLOR);
  });

  it("depends on the app, never on where it ranks in a list", () => {
    const apps = ["Code.exe", "chrome.exe", "slack.exe", "someapp.exe"];
    const forward = apps.map(appColor);
    const backward = [...apps].reverse().map(appColor).reverse();
    expect(backward).toEqual(forward);
  });

  it.each([
    ["a Windows developer", ["Code.exe", "teams.exe", "devenv.exe", "slack.exe", "WindowsTerminal.exe", "explorer.exe", "chrome.exe", "msedge.exe", "outlook.exe"]],
    ["an office worker", ["winword.exe", "teams.exe", "zoom.exe", "onenote.exe", "powerpnt.exe", "explorer.exe", "chrome.exe", "excel.exe", "msedge.exe", "outlook.exe"]],
    ["a Mac user", ["Code", "Notion", "zoom.us", "Slack", "Figma", "Spotify", "Google Chrome", "Terminal", "Finder", "Safari"]],
  ])("gives the apps %s typically runs side by side distinct colours", (_who, apps) => {
    const colors = apps.map(appColor);
    expect(new Set(colors).size).toBe(apps.length);
  });

  it("every palette colour (and the neutral) is readable on both the light and the dark card", () => {
    for (const color of [...APP_PALETTE, OTHER_APP_COLOR]) {
      expect(color).toMatch(/^#[0-9a-f]{6}$/);
      expect(contrast(color, "#ffffff")).toBeGreaterThanOrEqual(3);
      expect(contrast(color, "#1a2226")).toBeGreaterThanOrEqual(3);
    }
    expect(new Set(APP_PALETTE).size).toBe(APP_PALETTE.length);
  });
});

describe("appCategory / isClassifiedApp", () => {
  it.each([
    ["Visual Studio Code", "productive"],
    ["Terminal", "productive"],
    ["Figma", "productive"],
    ["Postman", "productive"],
    ["Linear", "productive"],
    ["Docker Desktop", "productive"],
    ["Google Chrome", "neutral"],
    ["Slack", "neutral"],
    ["Notion", "neutral"],
    ["Zoom", "neutral"],
    ["YouTube", "unproductive"],
    ["Spotify", "unproductive"],
    ["X (Twitter)", "unproductive"],
    ["X", "unproductive"],
  ] as const)("keeps the reports' classification of %s (%s)", (app, category) => {
    expect(appCategory(app)).toBe(category);
  });

  it("classifies what the agents actually send, through the canonical name", () => {
    expect(appCategory("Code.exe")).toBe("productive");
    expect(appCategory("WINWORD.EXE")).toBe("productive");
    expect(appCategory("WindowsTerminal.exe")).toBe("productive");
    expect(appCategory("msedge.exe")).toBe("neutral");
    expect(appCategory("zoom.us")).toBe("neutral");
    expect(appCategory("spotify.exe")).toBe("unproductive");
  });

  it("reads an app nobody classified as neutral, and says it is unclassified", () => {
    expect(appCategory("someinternaltool.exe")).toBe("neutral");
    expect(isClassifiedApp("someinternaltool.exe")).toBe(false);
    expect(isClassifiedApp("")).toBe(false);
    expect(isClassifiedApp("chrome.exe")).toBe(true);
    expect(isClassifiedApp("YouTube")).toBe(true);
  });
});

describe("category palette", () => {
  it("is the one the existing productivity charts render, so moving to it changes no colour", () => {
    expect(APP_CATEGORY_COLORS).toEqual(categoryColor);
  });

  it("labels every category", () => {
    expect(APP_CATEGORIES.map((c) => APP_CATEGORY_LABELS[c])).toEqual(["Productive", "Neutral", "Unproductive"]);
  });
});
