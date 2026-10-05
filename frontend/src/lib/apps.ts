/* ============================================================================
 * APP IDENTITY — the one name, colour and category an application has.
 *
 * Agents report apps the way their platform sees them: the Windows agent sends
 * the executable ("chrome.exe", "Code.exe"), the macOS agent the app's
 * localized name ("Google Chrome", "Code", "zoom.us"). Every screen that shows
 * an app — activity cards, Work Analytics, the Apps report, the dashboard, CSV
 * exports — goes through this module, so one app reads as one name with one
 * colour everywhere (contract §8).
 *
 * The backend applies the SAME mapping when it groups app usage
 * (AppUsageRowDto.displayName), so the alias table below mirrors the
 * reporting service's: change one, change the other in the same release.
 *
 * Colour follows the app, never its rank: the common apps hold fixed palette
 * slots (chosen so the apps one person typically runs side by side differ),
 * and any other name hashes into the same palette. Ten colours cannot keep
 * every pair of apps apart, so colour is a secondary cue — app charts always
 * name the app next to its swatch.
 * ========================================================================== */

export type AppCategory = "productive" | "neutral" | "unproductive";

export const APP_CATEGORIES: readonly AppCategory[] = ["productive", "neutral", "unproductive"];

export const APP_CATEGORY_LABELS: Record<AppCategory, string> = {
  productive: "Productive",
  neutral: "Neutral",
  unproductive: "Unproductive",
};

/** The productivity palette the category charts and legends render (unchanged from the reports). */
export const APP_CATEGORY_COLORS: Record<AppCategory, string> = {
  productive: "#22c55e",
  neutral: "#0ea5e9",
  unproductive: "#ef4444",
};

/** Shown for a block whose agent reported no app name. */
export const UNKNOWN_APP_NAME = "Unknown app";

/** The bucket charts fold their long tail into. */
export const OTHER_APPS_NAME = "Other apps";

/**
 * App colours. Each clears 3:1 against BOTH the light card (#ffffff) and the
 * dark card (#1a2226), so one value works in either theme; lightness
 * alternates between neighbouring hues to keep them apart.
 */
export const APP_PALETTE: readonly string[] = [
  "#2d6fd4", // 0 blue
  "#937cf1", // 1 lavender
  "#9f4cb1", // 2 purple
  "#dd5d96", // 3 pink
  "#c44132", // 4 red
  "#d47a05", // 5 orange
  "#977e05", // 6 gold
  "#308b15", // 7 green
  "#05a883", // 8 teal
  "#0aa0d2", // 9 sky
];

/** Neutral slate for "Other apps" and unnamed apps (also 3:1 on both cards). */
export const OTHER_APP_COLOR = "#768390";

/* ----------------------------- Canonical names ----------------------------- */

/**
 * Case-insensitive aliases, keyed by the lower-cased name with ".exe"
 * stripped (contract §8). The second group maps every canonical name to
 * itself, so canonicalising a name the server already canonicalised (its
 * `displayName`) is a no-op — "Windows app" must not become "Windows App".
 * (A Map, like the tables below: an app may be called "constructor".)
 */
const ALIASES = new Map<string, string>(Object.entries({
  chrome: "Google Chrome",
  "google chrome": "Google Chrome",
  msedge: "Microsoft Edge",
  code: "Visual Studio Code",
  firefox: "Firefox",
  slack: "Slack",
  teams: "Microsoft Teams",
  "ms-teams": "Microsoft Teams",
  outlook: "Outlook",
  winword: "Word",
  excel: "Excel",
  powerpnt: "PowerPoint",
  explorer: "File Explorer",
  finder: "Finder",
  windowsterminal: "Windows Terminal",
  terminal: "Terminal",
  iterm2: "iTerm2",
  devenv: "Visual Studio",
  idea64: "IntelliJ IDEA",
  rider64: "Rider",
  figma: "Figma",
  zoom: "Zoom",
  "zoom.us": "Zoom",
  notepad: "Notepad",
  applicationframehost: "Windows app",
  lockapp: "Lock screen",

  "microsoft edge": "Microsoft Edge",
  "visual studio code": "Visual Studio Code",
  "microsoft teams": "Microsoft Teams",
  word: "Word",
  powerpoint: "PowerPoint",
  "file explorer": "File Explorer",
  "windows terminal": "Windows Terminal",
  "visual studio": "Visual Studio",
  "intellij idea": "IntelliJ IDEA",
  rider: "Rider",
  "windows app": "Windows app",
  "lock screen": "Lock screen",
  "unknown app": UNKNOWN_APP_NAME,
  "other apps": OTHER_APPS_NAME,
}));

const EXE_SUFFIX = /\.exe$/i;

/** Trim and collapse whitespace runs to single spaces. */
function tidy(value: string): string {
  return value.trim().replace(/\s+/g, " ");
}

/** Upper-case the first character of each word and keep the rest ("myApp tool" → "MyApp Tool"). */
function titleCase(stem: string): string {
  return stem
    .split(" ")
    .map((word) => {
      const first = word.charAt(0);
      const upper = first.toUpperCase();
      // A character that upper-cases to several ("ß" → "SS") is kept, as .NET's char upper-casing does.
      return (upper.length === 1 ? upper : first) + word.slice(1);
    })
    .join(" ");
}

/**
 * The canonical name of an app as agents report it (contract §8): ".exe"
 * stripped, the alias table applied case-insensitively, otherwise the stem
 * title-cased. "" when there is no name at all. Idempotent: the canonical
 * name of a canonical name is itself.
 */
export function canonicalAppName(raw: string | null | undefined): string {
  if (typeof raw !== "string") return "";
  let stem = tidy(raw);
  while (EXE_SUFFIX.test(stem)) stem = tidy(stem.replace(EXE_SUFFIX, ""));
  if (!stem) return "";
  return ALIASES.get(stem.toLowerCase()) ?? titleCase(stem);
}

/** The name to show for an app: its canonical name, or "Unknown app" when it has none. */
export function friendlyAppName(raw: string | null | undefined): string {
  return canonicalAppName(raw) || UNKNOWN_APP_NAME;
}

/**
 * The key to group app rows by: the canonical name, lower-cased, so
 * "MyTool.exe" and "mytool.exe" (Windows names ignore case) are one app.
 */
export function appKey(raw: string | null | undefined): string {
  return canonicalAppName(raw).toLowerCase();
}

/* ----------------------------- Colours ----------------------------- */

/**
 * Fixed slots for the apps people most often run together, so a typical
 * developer's, office worker's or Mac user's top apps never share a colour.
 */
const PINNED_SLOTS = new Map<string, number>(Object.entries({
  "visual studio code": 0,
  word: 0,
  "microsoft teams": 1,
  notion: 1,
  linear: 1,
  discord: 1,
  "visual studio": 2,
  zoom: 2,
  rider: 2,
  "intellij idea": 2,
  slack: 3,
  onenote: 3,
  "windows terminal": 4,
  powerpoint: 4,
  figma: 4,
  youtube: 4,
  opera: 4,
  powershell: 4,
  cmd: 4,
  firefox: 5,
  "file explorer": 5,
  postman: 5,
  brave: 5,
  spotify: 5,
  "google chrome": 6,
  excel: 7,
  terminal: 7,
  iterm2: 7,
  whatsapp: 7,
  "microsoft edge": 8,
  finder: 8,
  outlook: 9,
  safari: 9,
  "docker desktop": 9,
  telegram: 9,
}));

/** 32-bit FNV-1a: small, fast and stable across sessions and browsers. */
function fnv1a(text: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

/** The app's colour: the same for every spelling of the same app, on every page. */
export function appColor(name: string | null | undefined): string {
  const key = appKey(name);
  if (!key || key === UNKNOWN_APP_NAME.toLowerCase() || key === OTHER_APPS_NAME.toLowerCase()) return OTHER_APP_COLOR;
  const slot = PINNED_SLOTS.get(key) ?? fnv1a(key) % APP_PALETTE.length;
  return APP_PALETTE[slot];
}

/* ----------------------------- Categories ----------------------------- */

/** Known apps by what they are usually used for. Anything else reads as neutral. */
const CATEGORIES = new Map<string, AppCategory>(Object.entries({
  // Building, writing and designing.
  "visual studio code": "productive",
  "visual studio": "productive",
  "intellij idea": "productive",
  rider: "productive",
  "windows terminal": "productive",
  terminal: "productive",
  iterm2: "productive",
  powershell: "productive",
  pwsh: "productive",
  cmd: "productive",
  figma: "productive",
  postman: "productive",
  linear: "productive",
  "docker desktop": "productive",
  word: "productive",
  excel: "productive",
  powerpoint: "productive",
  onenote: "productive",
  notepad: "productive",
  // Communication, browsing and the system itself.
  "google chrome": "neutral",
  "microsoft edge": "neutral",
  firefox: "neutral",
  safari: "neutral",
  brave: "neutral",
  opera: "neutral",
  slack: "neutral",
  "microsoft teams": "neutral",
  zoom: "neutral",
  outlook: "neutral",
  notion: "neutral",
  discord: "neutral",
  whatsapp: "neutral",
  telegram: "neutral",
  "file explorer": "neutral",
  finder: "neutral",
  "windows app": "neutral",
  "lock screen": "neutral",
  // Entertainment.
  youtube: "unproductive",
  spotify: "unproductive",
  netflix: "unproductive",
  steam: "unproductive",
  tiktok: "unproductive",
  instagram: "unproductive",
  facebook: "unproductive",
  x: "unproductive",
  "x (twitter)": "unproductive",
} satisfies Record<string, AppCategory>));

/** The app's category; apps nobody has classified are neutral. */
export function appCategory(name: string | null | undefined): AppCategory {
  return CATEGORIES.get(appKey(name)) ?? "neutral";
}

/**
 * Whether the app has a known classification. A report should not call time
 * "productive" or "unproductive" on the strength of the neutral default alone.
 */
export function isClassifiedApp(name: string | null | undefined): boolean {
  return CATEGORIES.has(appKey(name));
}
