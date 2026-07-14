import { activities, projectById, projects, userById } from "./tenant-data";
import type { Activity } from "./types";
import { rangeForKey, type RangeKey } from "./reports-data";

export type ActivitySort = "recent" | "oldest" | "prod-high" | "prod-low" | "active-high";

export interface ActivityFilters {
  query: string;
  memberId: string; // "all" or user id
  projectId: string; // "all" or project id
  rangeKey: RangeKey; // date preset ("custom" uses customFrom/customTo)
  customFrom: string; // YYYY-MM-DD
  customTo: string; // YYYY-MM-DD
  startTime: string; // "HH:MM" time-of-day lower bound
  endTime: string; // "HH:MM" time-of-day upper bound
  minProductivity: number; // 0-100
  maxProductivity: number; // 0-100
  app: string; // "all" or app name
  onlineOnly: boolean;
  webcamOnly: boolean;
  sort: ActivitySort;
}

export const defaultActivityFilters: ActivityFilters = {
  query: "",
  memberId: "all",
  projectId: "all",
  rangeKey: "7d",
  customFrom: "",
  customTo: "",
  startTime: "",
  endTime: "",
  minProductivity: 0,
  maxProductivity: 100,
  app: "all",
  onlineOnly: false,
  webcamOnly: false,
  sort: "recent",
};

/** Distinct application names seen across all activities (for the app filter). */
export function activityApps(): string[] {
  const set = new Set<string>();
  for (const a of activities) {
    set.add(a.screen.app);
    a.activeWindows.forEach((w) => set.add(w.appName));
    a.runningPrograms.forEach((w) => set.add(w.appName));
  }
  return [...set].filter((x) => x && x !== "—").sort();
}

function minutesOfDay(iso: string): number {
  const d = new Date(iso);
  return d.getHours() * 60 + d.getMinutes();
}

function parseTime(hhmm: string): number | null {
  if (!hhmm) return null;
  const [h, m] = hhmm.split(":").map(Number);
  if (Number.isNaN(h)) return null;
  return h * 60 + (m || 0);
}

function resolveRange(f: ActivityFilters): { from: number; to: number } {
  if (f.rangeKey === "custom" && (f.customFrom || f.customTo)) {
    const from = f.customFrom ? new Date(`${f.customFrom}T00:00:00`).getTime() : -Infinity;
    const to = f.customTo ? new Date(`${f.customTo}T23:59:59`).getTime() : Infinity;
    return { from, to };
  }
  const r = rangeForKey(f.rangeKey);
  return { from: r.from.getTime(), to: r.to.getTime() };
}

export function applyActivityFilters(list: Activity[], f: ActivityFilters): Activity[] {
  const q = f.query.trim().toLowerCase();
  const { from, to } = resolveRange(f);
  const startMin = parseTime(f.startTime);
  const endMin = parseTime(f.endTime);

  const out = list.filter((a) => {
    if (f.memberId !== "all" && a.userId !== f.memberId) return false;
    if (f.projectId !== "all" && a.projectId !== f.projectId) return false;

    const ended = new Date(a.endedAt).getTime();
    if (ended < from || ended > to) return false;

    if (startMin !== null || endMin !== null) {
      const mod = minutesOfDay(a.endedAt);
      if (startMin !== null && mod < startMin) return false;
      if (endMin !== null && mod > endMin) return false;
    }

    if (a.productivity < f.minProductivity || a.productivity > f.maxProductivity) return false;

    if (f.app !== "all") {
      const inApp =
        a.screen.app === f.app ||
        a.activeWindows.some((w) => w.appName === f.app) ||
        a.runningPrograms.some((w) => w.appName === f.app);
      if (!inApp) return false;
    }

    if (f.onlineOnly && !a.online) return false;
    if (f.webcamOnly && !a.hasWebcam) return false;

    if (q) {
      const hit =
        a.description.toLowerCase().includes(q) ||
        a.screen.app.toLowerCase().includes(q) ||
        a.activeWindows.some(
          (w) => w.appName.toLowerCase().includes(q) || w.windowTitle.toLowerCase().includes(q)
        ) ||
        (userById(a.userId)?.name.toLowerCase().includes(q) ?? false) ||
        (projectById(a.projectId)?.title.toLowerCase().includes(q) ?? false);
      if (!hit) return false;
    }

    return true;
  });

  const byEnd = (a: Activity) => new Date(a.endedAt).getTime();
  out.sort((a, b) => {
    switch (f.sort) {
      case "oldest":
        return byEnd(a) - byEnd(b);
      case "prod-high":
        return b.productivity - a.productivity;
      case "prod-low":
        return a.productivity - b.productivity;
      case "active-high":
        return b.mouseClicks + b.keyboardHits - (a.mouseClicks + a.keyboardHits);
      default:
        return byEnd(b) - byEnd(a);
    }
  });

  return out;
}

/** Count how many filters differ from defaults (for the "N active" badge). */
export function countActiveFilters(f: ActivityFilters): number {
  let n = 0;
  const d = defaultActivityFilters;
  if (f.memberId !== d.memberId) n++;
  if (f.projectId !== d.projectId) n++;
  if (f.rangeKey !== d.rangeKey || f.customFrom || f.customTo) n++;
  if (f.startTime || f.endTime) n++;
  if (f.minProductivity !== d.minProductivity || f.maxProductivity !== d.maxProductivity) n++;
  if (f.app !== d.app) n++;
  if (f.onlineOnly) n++;
  if (f.webcamOnly) n++;
  return n;
}

export interface FilterChip {
  key: keyof ActivityFilters | "productivity" | "time";
  label: string;
}

/** Human-readable chips for the active (non-default) filters. */
export function activeFilterChips(f: ActivityFilters): FilterChip[] {
  const chips: FilterChip[] = [];
  if (f.memberId !== "all") chips.push({ key: "memberId", label: `Member: ${userById(f.memberId)?.name ?? f.memberId}` });
  if (f.projectId !== "all") chips.push({ key: "projectId", label: `Project: ${projectById(f.projectId)?.title ?? f.projectId}` });
  if (f.rangeKey !== "7d" && !(f.rangeKey === "custom" && !f.customFrom && !f.customTo)) {
    if (f.rangeKey === "custom") chips.push({ key: "rangeKey", label: `Date: ${f.customFrom || "…"} → ${f.customTo || "…"}` });
    else chips.push({ key: "rangeKey", label: `Date: ${rangeLabel(f.rangeKey)}` });
  }
  if (f.startTime || f.endTime) chips.push({ key: "time", label: `Time: ${f.startTime || "00:00"}–${f.endTime || "23:59"}` });
  if (f.minProductivity !== 0 || f.maxProductivity !== 100) chips.push({ key: "productivity", label: `Productivity: ${f.minProductivity}–${f.maxProductivity}%` });
  if (f.app !== "all") chips.push({ key: "app", label: `App: ${f.app}` });
  if (f.onlineOnly) chips.push({ key: "onlineOnly", label: "Live only" });
  if (f.webcamOnly) chips.push({ key: "webcamOnly", label: "With webcam" });
  return chips;
}

function rangeLabel(key: RangeKey): string {
  const map: Record<RangeKey, string> = {
    today: "Today",
    yesterday: "Yesterday",
    "7d": "Last 7 days",
    "30d": "Last 30 days",
    month: "This month",
    custom: "Custom",
  };
  return map[key];
}

export const memberOptions = () => [
  { id: "all", name: "All members" },
  ...activities
    .reduce<string[]>((acc, a) => (acc.includes(a.userId) ? acc : [...acc, a.userId]), [])
    .map((id) => ({ id, name: userById(id)?.name ?? id })),
];

export const projectOptions = () => [
  { id: "all", title: "All projects" },
  ...projects.map((p) => ({ id: p.id, title: p.title })),
];
