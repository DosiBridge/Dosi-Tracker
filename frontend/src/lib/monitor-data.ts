import { NOW } from "./mock-data";
import { projects, userById } from "./tenant-data";
import type { ScreenMock } from "./types";
import type { AppCategory } from "./reports-data";

/* ----------------------------- Types ----------------------------- */

export type SegmentType = "work" | "meeting" | "break" | "idle";

export interface BrowserTab {
  id: string;
  title: string;
  domain: string;
  url: string;
  minutes: number;
  category: AppCategory;
  color: string; // favicon / brand color
}

export interface DaySegment {
  id: string;
  type: SegmentType;
  startMin: number; // minutes from midnight
  endMin: number;
  minutes: number;
  app: string;
  windowTitle: string;
  projectId: string | null;
  productivity: number; // 0 for idle/break
  mouseClicks: number;
  keyboardHits: number;
  screen: ScreenMock;
  /** Open browser tabs — only present when `app` is a web browser. */
  tabs?: BrowserTab[];
}

/* ----------------------------- Browser tabs ----------------------------- */

export const BROWSERS = ["Google Chrome", "Microsoft Edge", "Firefox", "Safari", "Brave"];

interface SiteDef {
  domain: string;
  title: string;
  category: AppCategory;
  color: string;
}

const SITE_CATALOG: SiteDef[] = [
  { domain: "github.com", title: "Pull requests · dosi-tracker", category: "productive", color: "#24292f" },
  { domain: "stackoverflow.com", title: "How to debug an event tap — Stack Overflow", category: "productive", color: "#f48024" },
  { domain: "developer.mozilla.org", title: "Array.prototype.reduce() - MDN", category: "productive", color: "#0b7dc3" },
  { domain: "abp.io", title: "Multi-Tenancy | ABP Framework Docs", category: "productive", color: "#e5177b" },
  { domain: "nextjs.org", title: "App Router — Next.js Docs", category: "productive", color: "#111827" },
  { domain: "localhost:3000", title: "Dosi-Tracker — Dashboard", category: "productive", color: "#6d5efc" },
  { domain: "figma.com", title: "Design System – Figma", category: "productive", color: "#a259ff" },
  { domain: "chatgpt.com", title: "ChatGPT", category: "productive", color: "#10a37f" },
  { domain: "mail.google.com", title: "Inbox (12) - Gmail", category: "neutral", color: "#ea4335" },
  { domain: "calendar.google.com", title: "Google Calendar — Week", category: "neutral", color: "#4285f4" },
  { domain: "docs.google.com", title: "Product Spec — Google Docs", category: "neutral", color: "#1a73e8" },
  { domain: "notion.so", title: "Sprint 24 Planning — Notion", category: "neutral", color: "#111111" },
  { domain: "linkedin.com", title: "LinkedIn — Feed", category: "neutral", color: "#0a66c2" },
  { domain: "news.ycombinator.com", title: "Hacker News", category: "neutral", color: "#ff6600" },
  { domain: "youtube.com", title: "Lo-fi beats to code to — YouTube", category: "unproductive", color: "#ff0000" },
  { domain: "x.com", title: "Home / X", category: "unproductive", color: "#111111" },
  { domain: "reddit.com", title: "r/programming", category: "unproductive", color: "#ff4500" },
];

function genTabs(app: string, minutes: number, seedKey: string): BrowserTab[] {
  const rand = mulberry32(hashStr(`${seedKey}:${app}`));
  const count = Math.max(2, Math.min(6, Math.round(minutes / 9) + 1));
  const pool = [...SITE_CATALOG];
  const chosen: SiteDef[] = [];
  for (let i = 0; i < count && pool.length; i++) {
    chosen.push(pool.splice(Math.floor(rand() * pool.length), 1)[0]);
  }
  const weights = chosen.map(() => 0.4 + rand());
  const wsum = weights.reduce((a, b) => a + b, 0) || 1;
  return chosen
    .map((s, i) => ({
      id: `${seedKey}-t${i}`,
      title: s.title,
      domain: s.domain,
      url: `https://${s.domain}/`,
      minutes: Math.max(1, Math.round((minutes * weights[i]) / wsum)),
      category: s.category,
      color: s.color,
    }))
    .sort((a, b) => b.minutes - a.minutes);
}

export interface BrowserSite {
  domain: string;
  title: string;
  minutes: number;
  visits: number;
  category: AppCategory;
  color: string;
}

export interface BrowserOverview {
  totalMinutes: number;
  distinctSites: number;
  tabInstances: number;
  sites: BrowserSite[];
  byCategory: Record<AppCategory, number>;
}

/** Roll up every browser segment's tabs into a per-day browsing overview. */
export function browserSummary(segments: DaySegment[]): BrowserOverview {
  const map = new Map<string, BrowserSite>();
  const byCategory: Record<AppCategory, number> = { productive: 0, neutral: 0, unproductive: 0 };
  let totalMinutes = 0;
  let tabInstances = 0;

  for (const s of segments) {
    if (!s.tabs) continue;
    for (const t of s.tabs) {
      tabInstances++;
      totalMinutes += t.minutes;
      byCategory[t.category] += t.minutes;
      const cur = map.get(t.domain) ?? { domain: t.domain, title: t.title, minutes: 0, visits: 0, category: t.category, color: t.color };
      cur.minutes += t.minutes;
      cur.visits += 1;
      map.set(t.domain, cur);
    }
  }
  return {
    totalMinutes,
    distinctSites: map.size,
    tabInstances,
    sites: [...map.values()].sort((a, b) => b.minutes - a.minutes),
    byCategory,
  };
}

export interface DaySummary {
  tracked: number; // work + meeting minutes
  idle: number; // idle + break minutes
  productive: number;
  productivityAvg: number;
  firstMin: number | null;
  lastMin: number | null;
  longestFocus: number;
  meetings: number;
  breakMinutes: number;
  screenshots: number;
}

/* ----------------------------- App metadata ----------------------------- */

const appMeta: Record<
  string,
  { kind: ScreenMock["kind"]; accent: string; category: AppCategory; titles: string[] }
> = {
  "Visual Studio Code": { kind: "editor", accent: "#3b82f6", category: "productive", titles: ["dashboard.tsx", "api.rs", "ProjectAppService.cs", "main.swift", "reports-data.ts"] },
  Terminal: { kind: "terminal", accent: "#22c55e", category: "productive", titles: ["cargo run --release", "npm run dev", "git rebase -i", "dotnet build"] },
  Figma: { kind: "design", accent: "#ec4899", category: "productive", titles: ["Dashboard v3", "Design System", "Mobile Flows", "Marketing Hero"] },
  Postman: { kind: "browser", accent: "#f59e0b", category: "productive", titles: ["POST /api/activities", "Auth flow", "GET /projects"] },
  Linear: { kind: "docs", accent: "#8b7dff", category: "productive", titles: ["Sprint 24", "Bug triage", "Roadmap"] },
  "Docker Desktop": { kind: "terminal", accent: "#0ea5e9", category: "productive", titles: ["postgres:16", "Containers", "Logs"] },
  "Google Chrome": { kind: "browser", accent: "#f97316", category: "neutral", titles: ["ABP Framework docs", "Next.js docs", "Stack Overflow", "localhost:3000"] },
  Slack: { kind: "chat", accent: "#a855f7", category: "neutral", titles: ["#engineering", "#dosi-standup", "Direct messages"] },
  Notion: { kind: "docs", accent: "#64748b", category: "neutral", titles: ["Architecture RFC", "Meeting notes", "Release checklist"] },
  Zoom: { kind: "chat", accent: "#38bdf8", category: "neutral", titles: ["Daily standup", "Sprint planning", "Design review", "1:1"] },
  YouTube: { kind: "browser", accent: "#ef4444", category: "unproductive", titles: ["Tech talk", "Music", "Tutorial"] },
};

const appsByDesignation: Record<string, string[]> = {
  "Frontend Developer": ["Visual Studio Code", "Google Chrome", "Figma", "Slack", "Terminal", "Linear"],
  "Backend Developer": ["Visual Studio Code", "Terminal", "Postman", "Docker Desktop", "Google Chrome", "Slack"],
  "QA Engineer": ["Google Chrome", "Postman", "Visual Studio Code", "Slack", "Notion"],
  "UI/UX Designer": ["Figma", "Google Chrome", "Slack", "Notion", "YouTube"],
  "DevOps Engineer": ["Terminal", "Docker Desktop", "Visual Studio Code", "Google Chrome", "Slack"],
  "Engineering Lead": ["Visual Studio Code", "Linear", "Slack", "Google Chrome", "Notion", "Terminal"],
};
const defaultApps = ["Visual Studio Code", "Google Chrome", "Slack", "Terminal"];

/* ----------------------------- Seeded RNG ----------------------------- */

function hashStr(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}
function mulberry32(seed: number) {
  return function () {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/* ----------------------------- Day list ----------------------------- */

const DAY = 24 * 60 * 60 * 1000;

/** Last N days (most recent first) as selectable options, deterministic from NOW. */
export function availableDays(count = 7) {
  const out: { iso: string; label: string; weekday: string; isToday: boolean }[] = [];
  for (let d = 0; d < count; d++) {
    const date = new Date(NOW.getTime() - d * DAY);
    const iso = date.toISOString().slice(0, 10);
    out.push({
      iso,
      label: date.toLocaleDateString("en", { month: "short", day: "numeric" }),
      weekday: date.toLocaleDateString("en", { weekday: "short" }),
      isToday: d === 0,
    });
  }
  return out;
}

function dayIndexFromIso(iso: string): number {
  const today = NOW.toISOString().slice(0, 10);
  const a = new Date(`${iso}T00:00:00Z`).getTime();
  const b = new Date(`${today}T00:00:00Z`).getTime();
  return Math.round((b - a) / DAY);
}
function weekdayOfIso(iso: string): number {
  return new Date(`${iso}T00:00:00Z`).getUTCDay();
}

/* ----------------------------- Timeline builder ----------------------------- */

export const DAY_START_MIN = 8 * 60; // 08:00
export const DAY_END_MIN = 20 * 60; // 20:00

export function buildDayTimeline(userId: string, iso: string): DaySegment[] {
  const user = userById(userId);
  if (!user || user.role === "client") return [];

  const rand = mulberry32(hashStr(`${userId}:${iso}`));
  const between = (min: number, max: number) => Math.floor(rand() * (max - min + 1)) + min;
  const pick = <T,>(arr: T[]) => arr[Math.floor(rand() * arr.length)];

  const weekday = weekdayOfIso(iso);
  const isWeekend = weekday === 0 || weekday === 6;
  const dayIndex = dayIndexFromIso(iso);

  // Target tracked minutes for the day.
  let target = user.trackedToday;
  if (dayIndex > 0) target = Math.round(user.trackedToday * (0.85 + rand() * 0.3));
  if (isWeekend) target = rand() > 0.5 ? Math.round(user.trackedToday * (0.2 + rand() * 0.25)) : 0;
  if (user.status === "offline" && dayIndex === 0) target = Math.round(target * 0.55);
  if (target <= 0) return [];

  const apps = appsByDesignation[user.designation] ?? defaultApps;
  const userProjects = projects.filter((p) => !p.archived && p.memberIds.includes(userId));
  const projPool = userProjects.length ? userProjects : [projects[0]];

  const segments: DaySegment[] = [];
  let cursor = DAY_START_MIN + between(0, 45); // arrives shortly after 08:00–08:45
  let tracked = 0;
  let hadLunch = false;
  let id = 0;

  const makeScreen = (app: string): ScreenMock => {
    const meta = appMeta[app] ?? appMeta["Google Chrome"];
    return { app, kind: meta.kind, accent: meta.accent };
  };

  while (cursor < DAY_END_MIN && tracked < target) {
    const roll = rand();

    // Lunch break around midday.
    if (!hadLunch && cursor >= 12 * 60 + 30 && cursor <= 14 * 60) {
      const dur = between(30, 55);
      segments.push(seg("break", cursor, dur, "Lunch break", null, 0, 0, 0, makeScreen("Google Chrome")));
      cursor += dur;
      hadLunch = true;
      continue;
    }

    if (roll < 0.12) {
      // Meeting
      const dur = between(25, 55);
      const proj = pick(projPool);
      const prod = Math.min(96, Math.max(35, user.productivity - between(5, 20)));
      const kb = Math.round((dur * prod * (5 + rand() * 6)) / 100);
      const ms = Math.round((dur * prod * (2 + rand() * 3)) / 100);
      segments.push(seg("meeting", cursor, dur, pick(appMeta.Zoom.titles), proj.id, prod, ms, kb, makeScreen("Zoom"), "Zoom"));
      cursor += dur;
      tracked += dur;
    } else if (roll < 0.24) {
      // Short idle / away
      const dur = between(6, 22);
      segments.push(seg("idle", cursor, dur, "Idle", null, 0, 0, 0, makeScreen("Google Chrome")));
      cursor += dur;
    } else {
      // Work block
      const dur = between(12, 45);
      const app = pick(apps);
      const meta = appMeta[app] ?? appMeta["Google Chrome"];
      const proj = pick(projPool);
      const base = user.productivity + between(-18, 12);
      const prod = Math.min(99, Math.max(28, meta.category === "unproductive" ? between(20, 45) : base));
      const kb = Math.round((dur * prod * (18 + rand() * 26)) / 100);
      const ms = Math.round((dur * prod * (5 + rand() * 12)) / 100);
      segments.push(seg("work", cursor, dur, pick(meta.titles), proj.id, prod, ms, kb, makeScreen(app), app));
      cursor += dur;
      tracked += dur;
    }
  }

  return segments;

  function seg(
    type: SegmentType,
    start: number,
    minutes: number,
    windowTitle: string,
    projectId: string | null,
    productivity: number,
    mouseClicks: number,
    keyboardHits: number,
    screen: ScreenMock,
    app = "—"
  ): DaySegment {
    id++;
    const segId = `${userId}-${iso}-${id}`;
    return {
      id: segId,
      type,
      startMin: start,
      endMin: Math.min(start + minutes, DAY_END_MIN),
      minutes,
      app,
      windowTitle,
      projectId,
      productivity,
      mouseClicks,
      keyboardHits,
      screen,
      tabs: BROWSERS.includes(app) ? genTabs(app, minutes, segId) : undefined,
    };
  }
}

/* ----------------------------- Summaries ----------------------------- */

export function daySummary(segments: DaySegment[]): DaySummary {
  const active = segments.filter((s) => s.type === "work" || s.type === "meeting");
  const tracked = active.reduce((s, x) => s + x.minutes, 0);
  const idle = segments.filter((s) => s.type === "idle" || s.type === "break").reduce((s, x) => s + x.minutes, 0);
  const productive = active.reduce((s, x) => s + (x.minutes * x.productivity) / 100, 0);
  const productivityAvg = tracked ? Math.round((productive / tracked) * 100) : 0;

  let longestFocus = 0;
  let run = 0;
  for (const s of segments) {
    if (s.type === "work") {
      run += s.minutes;
      longestFocus = Math.max(longestFocus, run);
    } else {
      run = 0;
    }
  }

  return {
    tracked,
    idle,
    productive: Math.round(productive),
    productivityAvg,
    firstMin: segments.length ? segments[0].startMin : null,
    lastMin: segments.length ? segments[segments.length - 1].endMin : null,
    longestFocus,
    meetings: segments.filter((s) => s.type === "meeting").length,
    breakMinutes: segments.filter((s) => s.type === "break").reduce((s, x) => s + x.minutes, 0),
    screenshots: active.length,
  };
}

export function hourlyBuckets(segments: DaySegment[]) {
  const startH = DAY_START_MIN / 60;
  const endH = DAY_END_MIN / 60;
  const buckets: { hour: string; minutes: number }[] = [];
  for (let h = startH; h < endH; h++) {
    const lo = h * 60;
    const hi = lo + 60;
    let minutes = 0;
    for (const s of segments) {
      if (s.type === "idle" || s.type === "break") continue;
      const overlap = Math.max(0, Math.min(s.endMin, hi) - Math.max(s.startMin, lo));
      minutes += overlap;
    }
    buckets.push({ hour: `${String(h).padStart(2, "0")}:00`, minutes: Math.round(minutes) });
  }
  return buckets;
}

export function appBreakdown(segments: DaySegment[]) {
  const map = new Map<string, { app: string; minutes: number; accent: string; category: AppCategory }>();
  for (const s of segments) {
    if (s.type === "idle" || s.type === "break") continue;
    const meta = appMeta[s.app] ?? { accent: "#94a3b8", category: "neutral" as AppCategory };
    const cur = map.get(s.app) ?? { app: s.app, minutes: 0, accent: meta.accent, category: meta.category };
    cur.minutes += s.minutes;
    map.set(s.app, cur);
  }
  return [...map.values()].sort((a, b) => b.minutes - a.minutes);
}

/** "09:35" from minutes-from-midnight. */
export function fmtMin(min: number): string {
  const h = Math.floor(min / 60) % 24;
  const m = Math.round(min % 60);
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;
}
