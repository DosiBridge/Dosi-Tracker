import type { Activity, Project, ScreenMock, User, WindowInfo } from "./types";
import type {
  AppUsage,
  AttendanceRecord,
  AttendanceStatus,
  BillingRow,
  AppNotification,
  Insight,
  ProductivitySplit,
} from "./reports-data";
import { trackedMembers, countSeats } from "./roles";
import {
  NOW,
  users as primaryUsers,
  projects as primaryProjects,
  activities as primaryActivities,
  weeklyTrend as primaryWeekly,
  hourlyToday as primaryHourly,
  topApps as primaryTopApps,
  summary as primarySummary,
  projectDistribution as primaryProjectDistribution,
} from "./mock-data";

export { NOW };

/* ============================================================================
 * A Dataset is the full, tenant-isolated demo data for a single workspace.
 * Switching workspaces swaps the active dataset (see setActiveWorkspace).
 * ========================================================================== */

export interface Summary {
  totalTrackedToday: number;
  activeMembers: number;
  totalMembers: number;
  activeProjects: number;
  avgProductivity: number;
  screenshotsToday: number;
}

export interface Dataset {
  workspaceId: string;
  users: User[];
  projects: Project[];
  activities: Activity[];
  weeklyTrend: { day: string; tracked: number; productive: number }[];
  hourlyToday: { hour: string; minutes: number }[];
  topApps: { app: string; minutes: number; color: string }[];
  summary: Summary;
  projectDistribution: { name: string; value: number; color: string }[];
  appCatalog: AppUsage[];
  productivitySplit: ProductivitySplit[];
  attendance: AttendanceRecord[];
  billing: BillingRow[];
  notifications: AppNotification[];
}

/* ----------------------------- Seeded RNG ----------------------------- */

function mulberry32(seed: number) {
  return function () {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
function hashStr(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/* ----------------------------- Shared app metadata (generation) ----------------------------- */

const screenTemplates: ScreenMock[] = [
  { app: "Visual Studio Code", kind: "editor", accent: "#3b82f6" },
  { app: "Google Chrome", kind: "browser", accent: "#f97316" },
  { app: "Figma", kind: "design", accent: "#ec4899" },
  { app: "Slack", kind: "chat", accent: "#a855f7" },
  { app: "Terminal", kind: "terminal", accent: "#22c55e" },
  { app: "Notion", kind: "docs", accent: "#64748b" },
];
const genericTitles: Record<string, string[]> = {
  "Visual Studio Code": ["index.ts", "app.tsx", "service.cs", "main.py", "styles.css"],
  "Google Chrome": ["Docs", "localhost:3000", "Stack Overflow", "Dashboard"],
  Figma: ["Design v2", "Components", "Flows", "Hero"],
  Slack: ["#general", "#standup", "Direct messages", "#design"],
  Terminal: ["npm run dev", "git status", "docker compose up", "build"],
  Notion: ["Planning", "RFC", "Checklist", "Notes"],
};
const otherApps = ["Postman", "Docker Desktop", "Spotify", "1Password", "Zoom", "Linear"];
const descriptions = [
  "Implementing new feature",
  "Fixing a reported bug",
  "Refactoring core module",
  "Building UI components",
  "Reviewing pull requests",
  "Designing the new view",
  "Writing tests",
  "Tuning performance",
  "Wiring up the API",
  "Polishing the UI",
];

function makeWindows(primary: ScreenMock, rand: () => number): { active: WindowInfo[]; running: WindowInfo[] } {
  const pick = <T>(arr: T[]) => arr[Math.floor(rand() * arr.length)];
  const between = (min: number, max: number) => Math.floor(rand() * (max - min + 1)) + min;
  const active: WindowInfo[] = [
    { appName: primary.app, windowTitle: pick(genericTitles[primary.app] ?? ["—"]), seconds: between(180, 520) },
    { appName: pick(screenTemplates).app, windowTitle: "—", seconds: between(40, 160) },
  ];
  const running: WindowInfo[] = [
    { appName: primary.app, windowTitle: pick(genericTitles[primary.app] ?? ["—"]), seconds: 0 },
    { appName: pick(otherApps), windowTitle: "—", seconds: 0 },
    { appName: pick(otherApps), windowTitle: "—", seconds: 0 },
  ];
  return { active, running };
}

/* ----------------------------- Generators ----------------------------- */

function genActivities(users: User[], projects: Project[], seedNum: number): Activity[] {
  const rand = mulberry32(seedNum);
  const pick = <T>(arr: T[]) => arr[Math.floor(rand() * arr.length)];
  const between = (min: number, max: number) => Math.floor(rand() * (max - min + 1)) + min;
  const list: Activity[] = [];
  const trackingUsers = trackedMembers(users);
  if (!projects.length || !trackingUsers.length) return list;
  let id = 0;

  for (let day = 0; day < 5; day++) {
    for (const user of trackingUsers) {
      const blocks = day === 0 ? between(2, 4) : between(1, 3);
      for (let b = 0; b < blocks; b++) {
        const template = pick(screenTemplates);
        const mine = projects.filter((p) => !p.archived && p.memberIds.includes(user.id));
        const project = (mine.length ? pick(mine) : projects[0]);
        const minutesAgo = day * 24 * 60 + b * between(35, 90) + between(5, 25);
        const ended = new Date(NOW.getTime() - minutesAgo * 60000);
        const started = new Date(ended.getTime() - project.intervalMinutes * 60000);
        const { active, running } = makeWindows(template, rand);
        id++;
        list.push({
          id: `${user.id}-a${id}`,
          userId: user.id,
          projectId: project.id,
          startedAt: started.toISOString(),
          endedAt: ended.toISOString(),
          description: pick(descriptions),
          productivity: between(48, 97),
          mouseClicks: between(60, 640),
          keyboardHits: between(200, 3200),
          activeWindows: active,
          runningPrograms: running,
          screen: template,
          hasWebcam: project.permissions.webcam && rand() > 0.5,
          online: day === 0 && b === 0 ? true : rand() > 0.25,
        });
      }
    }
  }
  return list.sort((a, b) => +new Date(b.endedAt) - +new Date(a.endedAt));
}

function genWeekly(users: User[], seedNum: number) {
  const rand = mulberry32(seedNum ^ 0x51ed);
  const team = countSeats(users) || 1;
  const days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
  const factor = [1, 1.08, 0.96, 1.14, 1.24, 0.45, 0.23];
  return days.map((day, i) => {
    const tracked = Math.round(team * (55 + rand() * 20) * factor[i]);
    const productive = Math.round(tracked * (0.72 + rand() * 0.12));
    return { day, tracked, productive };
  });
}

function genHourly(seedNum: number) {
  const rand = mulberry32(seedNum ^ 0x2b1c);
  const shape = [20, 42, 55, 48, 30, 12, 38, 52, 58, 46, 33, 18];
  return Array.from({ length: 12 }, (_, i) => ({
    hour: `${8 + i}:00`,
    minutes: Math.max(4, Math.round(shape[i] * (0.7 + rand() * 0.6))),
  }));
}

const APP_PALETTE: { app: string; category: AppUsage["category"]; color: string }[] = [
  { app: "Visual Studio Code", category: "productive", color: "#3b82f6" },
  { app: "Terminal", category: "productive", color: "#22c55e" },
  { app: "Figma", category: "productive", color: "#ec4899" },
  { app: "Postman", category: "productive", color: "#f97316" },
  { app: "Linear", category: "productive", color: "#8b7dff" },
  { app: "Google Chrome", category: "neutral", color: "#64748b" },
  { app: "Slack", category: "neutral", color: "#a855f7" },
  { app: "Notion", category: "neutral", color: "#94a3b8" },
  { app: "Zoom", category: "neutral", color: "#38bdf8" },
  { app: "YouTube", category: "unproductive", color: "#ef4444" },
  { app: "Spotify", category: "unproductive", color: "#f43f5e" },
];

function genAppCatalog(users: User[], seedNum: number): AppUsage[] {
  const rand = mulberry32(seedNum ^ 0x9a3f);
  const team = Math.max(1, countSeats(users));
  return APP_PALETTE.map((a) => ({
    app: a.app,
    category: a.category,
    color: a.color,
    minutes: Math.round((120 + rand() * 380) * team * 0.5),
    activeUsers: Math.max(1, Math.min(team, Math.round(1 + rand() * team))),
  })).sort((a, b) => b.minutes - a.minutes);
}

function genTopApps(catalog: AppUsage[]) {
  return [...catalog]
    .sort((a, b) => b.minutes - a.minutes)
    .slice(0, 6)
    .map((a) => ({ app: a.app, minutes: a.minutes, color: a.color }));
}

/* ----------------------------- Derived (shared logic; matches original) ----------------------------- */

function deriveSplit(users: User[]): ProductivitySplit[] {
  return trackedMembers(users)
    .map((u) => {
      const total = u.trackedToday;
      const productive = Math.round((total * u.productivity) / 100);
      const remaining = total - productive;
      const unproductive = Math.round(remaining * 0.35);
      const neutral = remaining - unproductive;
      const idle = Math.round(total * 0.08);
      return { userId: u.id, productive, neutral, unproductive, idle };
    });
}

const DAY = 24 * 60 * 60 * 1000;

function deriveAttendance(users: User[], seedNum: number): AttendanceRecord[] {
  const rand = mulberry32(seedNum);
  const tracking = trackedMembers(users);
  const rows: AttendanceRecord[] = [];
  for (let d = 6; d >= 0; d--) {
    const date = new Date(NOW.getTime() - d * DAY);
    const iso = date.toISOString().slice(0, 10);
    const weekend = date.getDay() === 0 || date.getDay() === 6;
    for (const u of tracking) {
      const roll = rand();
      let status: AttendanceStatus;
      if (weekend) status = roll > 0.75 ? "remote" : "absent";
      else if (roll > 0.9) status = "absent";
      else if (roll > 0.75) status = "late";
      else if (roll > 0.55) status = "remote";
      else status = "present";

      if (status === "absent") {
        rows.push({ userId: u.id, date: iso, status, clockIn: null, clockOut: null, worked: 0 });
        continue;
      }
      const inHour = status === "late" ? 10 + Math.floor(rand() * 2) : 9;
      const inMin = Math.floor(rand() * 55);
      const workedH = weekend ? 2 + rand() * 3 : 6 + rand() * 3;
      const clockIn = `${String(inHour).padStart(2, "0")}:${String(inMin).padStart(2, "0")}`;
      const outTotal = inHour * 60 + inMin + Math.round(workedH * 60);
      const clockOut = `${String(Math.floor(outTotal / 60) % 24).padStart(2, "0")}:${String(outTotal % 60).padStart(2, "0")}`;
      rows.push({ userId: u.id, date: iso, status, clockIn, clockOut, worked: Math.round(workedH * 60) });
    }
  }
  return rows;
}

function deriveBilling(users: User[], rates: Record<string, number>, seedNum: number): BillingRow[] {
  const rand = mulberry32(seedNum ^ 0x1234);
  return trackedMembers(users)
    .map((u) => ({
      userId: u.id,
      rate: rates[u.id] ?? 38 + Math.round(rand() * 5) * 4,
      billable: true,
    }));
}

function genNotifications(users: User[], projects: Project[]): AppNotification[] {
  const workers = users.filter((u) => u.role === "worker");
  const owner = users.find((u) => u.role === "owner");
  const proj = projects[0];
  const out: AppNotification[] = [];
  out.push({ id: "n1", type: "report", title: "Weekly report ready", body: "Your team's weekly productivity report is available.", at: new Date(NOW.getTime() - 8 * 60000).toISOString(), read: false });
  if (workers[0]) out.push({ id: "n2", type: "alert", title: "Low activity detected", body: `${workers[0].name} was below 40% activity for 2h.`, at: new Date(NOW.getTime() - 46 * 60000).toISOString(), read: false });
  if (workers[1] && proj) out.push({ id: "n3", type: "member", title: "New member joined", body: `${workers[1].name} accepted the invite to ${proj.title}.`, at: new Date(NOW.getTime() - 3 * 3600000).toISOString(), read: false });
  if (owner) out.push({ id: "n5", type: "alert", title: "Overtime warning", body: `${owner.name} exceeded 9h tracked today.`, at: new Date(NOW.getTime() - 9 * 3600000).toISOString(), read: true });
  out.push({ id: "n6", type: "system", title: "Agent updated", body: "Desktop tracker updated to the latest version.", at: new Date(NOW.getTime() - 26 * 3600000).toISOString(), read: true });
  return out;
}

function computeSummary(users: User[], projects: Project[]): Summary {
  const active = users.filter((u) => u.productivity > 0);
  return {
    totalTrackedToday: users.reduce((s, u) => s + u.trackedToday, 0),
    activeMembers: users.filter((u) => u.status === "active").length,
    totalMembers: countSeats(users),
    activeProjects: projects.filter((p) => !p.archived).length,
    avgProductivity: active.length ? Math.round(active.reduce((s, u) => s + u.productivity, 0) / active.length) : 0,
    screenshotsToday: countSeats(users) * 40,
  };
}

function computeProjectDistribution(projects: Project[]) {
  return projects
    .filter((p) => !p.archived)
    .map((p) => ({ name: p.title, value: p.loggedThisWeek, color: p.color }))
    .sort((a, b) => b.value - a.value);
}

/* ----------------------------- Primary (w1) — exact original values ----------------------------- */

const PRIMARY_APP_CATALOG: AppUsage[] = [
  { app: "Visual Studio Code", category: "productive", minutes: 1840, color: "#3b82f6", activeUsers: 6 },
  { app: "Terminal", category: "productive", minutes: 430, color: "#22c55e", activeUsers: 5 },
  { app: "Figma", category: "productive", minutes: 760, color: "#ec4899", activeUsers: 3 },
  { app: "Postman", category: "productive", minutes: 210, color: "#f97316", activeUsers: 3 },
  { app: "Linear", category: "productive", minutes: 180, color: "#8b7dff", activeUsers: 4 },
  { app: "Docker Desktop", category: "productive", minutes: 160, color: "#0ea5e9", activeUsers: 2 },
  { app: "Google Chrome", category: "neutral", minutes: 1120, color: "#64748b", activeUsers: 7 },
  { app: "Slack", category: "neutral", minutes: 540, color: "#a855f7", activeUsers: 7 },
  { app: "Notion", category: "neutral", minutes: 260, color: "#94a3b8", activeUsers: 5 },
  { app: "Zoom", category: "neutral", minutes: 190, color: "#38bdf8", activeUsers: 4 },
  { app: "YouTube", category: "unproductive", minutes: 220, color: "#ef4444", activeUsers: 4 },
  { app: "Spotify", category: "unproductive", minutes: 150, color: "#f43f5e", activeUsers: 3 },
  { app: "X (Twitter)", category: "unproductive", minutes: 95, color: "#fb7185", activeUsers: 3 },
];
const PRIMARY_RATES: Record<string, number> = { u1: 60, u2: 45, u3: 45, u4: 38, u5: 42, u6: 55, u8: 50 };
const PRIMARY_NOTIFICATIONS: AppNotification[] = [
  { id: "n1", type: "report", title: "Weekly report ready", body: "Your team's weekly productivity report is available.", at: new Date(NOW.getTime() - 8 * 60000).toISOString(), read: false },
  { id: "n2", type: "alert", title: "Low activity detected", body: "Sadia Akter was below 40% activity for 2h.", at: new Date(NOW.getTime() - 46 * 60000).toISOString(), read: false },
  { id: "n3", type: "member", title: "New member joined", body: "Imran Kabir accepted the invite to Dosi Web Platform.", at: new Date(NOW.getTime() - 3 * 3600000).toISOString(), read: false },
  { id: "n4", type: "mention", title: "You were mentioned", body: "Tanvir mentioned you in Sprint 24 planning.", at: new Date(NOW.getTime() - 6 * 3600000).toISOString(), read: true },
  { id: "n5", type: "alert", title: "Overtime warning", body: "Ayesha Rahman exceeded 9h tracked today.", at: new Date(NOW.getTime() - 9 * 3600000).toISOString(), read: true },
  { id: "n6", type: "system", title: "Agent updated", body: "Windows tracker updated to v0.1.0 on 4 machines.", at: new Date(NOW.getTime() - 26 * 3600000).toISOString(), read: true },
];

function buildPrimary(): Dataset {
  return {
    workspaceId: "w1",
    users: primaryUsers,
    projects: primaryProjects,
    activities: primaryActivities,
    weeklyTrend: primaryWeekly,
    hourlyToday: primaryHourly,
    topApps: primaryTopApps,
    summary: primarySummary,
    projectDistribution: primaryProjectDistribution(),
    appCatalog: PRIMARY_APP_CATALOG,
    productivitySplit: deriveSplit(primaryUsers),
    attendance: deriveAttendance(primaryUsers, 99887766),
    billing: deriveBilling(primaryUsers, PRIMARY_RATES, 1),
    notifications: PRIMARY_NOTIFICATIONS,
  };
}

/* ----------------------------- Other seed tenants ----------------------------- */

interface Roster {
  id: string;
  users: User[];
  projects: Project[];
}

function buildGenerated(roster: Roster): Dataset {
  const seed = hashStr(roster.id);
  const activities = genActivities(roster.users, roster.projects, seed);
  const appCatalog = genAppCatalog(roster.users, seed);
  return {
    workspaceId: roster.id,
    users: roster.users,
    projects: roster.projects,
    activities,
    weeklyTrend: genWeekly(roster.users, seed),
    hourlyToday: genHourly(seed),
    topApps: genTopApps(appCatalog),
    summary: computeSummary(roster.users, roster.projects),
    projectDistribution: computeProjectDistribution(roster.projects),
    appCatalog,
    productivitySplit: deriveSplit(roster.users),
    attendance: deriveAttendance(roster.users, seed),
    billing: deriveBilling(roster.users, {}, seed),
    notifications: genNotifications(roster.users, roster.projects),
  };
}

const perms = { screenshot: true, webcam: false, keyboard: true, mouse: true, activeWindow: true, runningPrograms: true };

// --- Acme Studio (w2): a design agency ---
const acmeUsers: User[] = [
  { id: "w2-u1", name: "Diego Alvarez", email: "diego@acmestudio.com", role: "owner", designation: "Creative Director", status: "active", timezone: "Europe/Madrid", trackedToday: 356, productivity: 84, joinedAt: "2026-06-28" },
  { id: "w2-u2", name: "Lucia Romero", email: "lucia@acmestudio.com", role: "worker", designation: "UI/UX Designer", status: "active", timezone: "Europe/Madrid", trackedToday: 322, productivity: 81, joinedAt: "2026-06-29" },
  { id: "w2-u3", name: "Marc Dubois", email: "marc@acmestudio.com", role: "worker", designation: "Frontend Developer", status: "idle", timezone: "Europe/Paris", trackedToday: 290, productivity: 76, joinedAt: "2026-07-01" },
  { id: "w2-u4", name: "Sofia Rossi", email: "sofia@acmestudio.com", role: "worker", designation: "UI/UX Designer", status: "offline", timezone: "Europe/Rome", trackedToday: 214, productivity: 69, joinedAt: "2026-07-02" },
  { id: "w2-u5", name: "Nora Klein", email: "nora@brandhaus.de", role: "client", designation: "Client · Brandhaus", status: "offline", timezone: "Europe/Berlin", trackedToday: 0, productivity: 0, joinedAt: "2026-07-03" },
];
const acmeProjects: Project[] = [
  { id: "w2-p1", title: "Brand Refresh 2026", description: "Full identity refresh for Brandhaus.", color: "#ec4899", archived: false, intervalMinutes: 10, permissions: { ...perms, webcam: true }, memberIds: ["w2-u1", "w2-u2", "w2-u4", "w2-u5"], createdAt: "2026-06-29", loggedThisWeek: 1260, loggedThisMonth: 3120, loggedTotal: 3120 },
  { id: "w2-p2", title: "Ad Campaign — Q3", description: "Social + display campaign assets.", color: "#f59e0b", archived: false, intervalMinutes: 15, permissions: perms, memberIds: ["w2-u2", "w2-u3"], createdAt: "2026-07-01", loggedThisWeek: 820, loggedThisMonth: 1980, loggedTotal: 1980 },
  { id: "w2-p3", title: "Packaging System", description: "Reusable packaging design system.", color: "#8b5cf6", archived: false, intervalMinutes: 10, permissions: perms, memberIds: ["w2-u1", "w2-u3", "w2-u4"], createdAt: "2026-07-02", loggedThisWeek: 540, loggedThisMonth: 1240, loggedTotal: 1240 },
];

// --- Nimbus Co (w3): a small dev shop ---
const nimbusUsers: User[] = [
  { id: "w3-u1", name: "Kwame Mensah", email: "kwame@nimbus.co", role: "owner", designation: "Founder / Engineer", status: "active", timezone: "Africa/Accra", trackedToday: 301, productivity: 80, joinedAt: "2026-05-15" },
  { id: "w3-u2", name: "Aisha Bello", email: "aisha@nimbus.co", role: "worker", designation: "Backend Developer", status: "active", timezone: "Africa/Lagos", trackedToday: 268, productivity: 77, joinedAt: "2026-05-18" },
  { id: "w3-u3", name: "Liang Wei", email: "liang@nimbus.co", role: "worker", designation: "Data Engineer", status: "idle", timezone: "Asia/Shanghai", trackedToday: 233, productivity: 72, joinedAt: "2026-05-20" },
];
const nimbusProjects: Project[] = [
  { id: "w3-p1", title: "Weather API", description: "Public weather data REST API.", color: "#0ea5e9", archived: false, intervalMinutes: 10, permissions: perms, memberIds: ["w3-u1", "w3-u2", "w3-u3"], createdAt: "2026-05-16", loggedThisWeek: 940, loggedThisMonth: 4200, loggedTotal: 8600 },
  { id: "w3-p2", title: "Nimbus Mobile", description: "Cross-platform mobile client.", color: "#22c55e", archived: false, intervalMinutes: 15, permissions: perms, memberIds: ["w3-u1", "w3-u2"], createdAt: "2026-05-22", loggedThisWeek: 610, loggedThisMonth: 2680, loggedTotal: 5100 },
];

/* ----------------------------- Empty (freshly created) tenants ----------------------------- */

function buildEmpty(id: string): Dataset {
  const owner: User = {
    id: `${id}-owner`,
    name: "You",
    email: "you@workspace.app",
    role: "owner",
    designation: "Workspace Owner",
    status: "active",
    timezone: "UTC",
    trackedToday: 0,
    productivity: 0,
    joinedAt: NOW.toISOString().slice(0, 10),
  };
  return {
    workspaceId: id,
    users: [owner],
    projects: [],
    activities: [],
    weeklyTrend: genWeekly([owner], hashStr(id)).map((d) => ({ ...d, tracked: 0, productive: 0 })),
    hourlyToday: genHourly(hashStr(id)).map((h) => ({ ...h, minutes: 0 })),
    topApps: [],
    summary: { totalTrackedToday: 0, activeMembers: 1, totalMembers: 1, activeProjects: 0, avgProductivity: 0, screenshotsToday: 0 },
    projectDistribution: [],
    appCatalog: [],
    productivitySplit: [],
    attendance: [],
    billing: [],
    notifications: [
      { id: "n1", type: "system", title: "Welcome to your workspace", body: "Invite members and create your first project to get started.", at: NOW.toISOString(), read: false },
    ],
  };
}

/* ----------------------------- Registry ----------------------------- */

const cache = new Map<string, Dataset>();

function getLocalStorageProjects(workspaceId: string): Project[] {
  if (typeof window === "undefined") return [];
  try {
    const val = localStorage.getItem(`dosi-projects-created-${workspaceId}`);
    return val ? JSON.parse(val) : [];
  } catch {
    return [];
  }
}

export function datasetFor(workspaceId: string): Dataset {
  const cached = cache.get(workspaceId);
  if (cached) return cached;

  let ds: Dataset;
  if (workspaceId === "w1") ds = buildPrimary();
  else if (workspaceId === "w2") ds = buildGenerated({ id: "w2", users: acmeUsers, projects: acmeProjects });
  else if (workspaceId === "w3") ds = buildGenerated({ id: "w3", users: nimbusUsers, projects: nimbusProjects });
  else ds = buildEmpty(workspaceId);

  const localProjects = getLocalStorageProjects(workspaceId);
  if (localProjects.length > 0) {
    ds.projects = [...localProjects, ...ds.projects];
  }

  cache.set(workspaceId, ds);
  return ds;
}

export function createTenantProject(workspaceId: string, project: Project): void {
  const ds = datasetFor(workspaceId);
  if (!ds.projects.some((p) => p.id === project.id)) {
    ds.projects = [project, ...ds.projects];
  }

  if (typeof window !== "undefined") {
    try {
      const key = `dosi-projects-created-${workspaceId}`;
      const existing = localStorage.getItem(key);
      const list = existing ? JSON.parse(existing) : [];
      if (!list.some((p: any) => p.id === project.id)) {
        list.unshift(project);
        localStorage.setItem(key, JSON.stringify(list));
      }
    } catch {}
  }

  if (active.workspaceId === workspaceId) {
    projects = active.projects;
  }
}

/* ============================================================================
 * Live bindings — these reflect the ACTIVE workspace. setActiveWorkspace()
 * swaps them; the dashboard subtree is remounted on workspace change so every
 * component re-reads the new tenant's data.
 * ========================================================================== */

let active: Dataset = datasetFor("w1");

export let users: User[] = active.users;
export let projects: Project[] = active.projects;
export let activities: Activity[] = active.activities;
export let weeklyTrend = active.weeklyTrend;
export let hourlyToday = active.hourlyToday;
export let topApps = active.topApps;
export let summary: Summary = active.summary;
export let appCatalog: AppUsage[] = active.appCatalog;
export let productivitySplit: ProductivitySplit[] = active.productivitySplit;
export let attendance: AttendanceRecord[] = active.attendance;
export let billing: BillingRow[] = active.billing;
export let notifications: AppNotification[] = active.notifications;

export function activeWorkspaceId(): string {
  return active.workspaceId;
}

export function setActiveWorkspace(workspaceId: string): void {
  const ds = datasetFor(workspaceId);
  if (ds === active) return;
  active = ds;
  users = ds.users;
  projects = ds.projects;
  activities = ds.activities;
  weeklyTrend = ds.weeklyTrend;
  hourlyToday = ds.hourlyToday;
  topApps = ds.topApps;
  summary = ds.summary;
  appCatalog = ds.appCatalog;
  productivitySplit = ds.productivitySplit;
  attendance = ds.attendance;
  billing = ds.billing;
  notifications = ds.notifications;
}

/* ----------------------------- Lookups & derived helpers (read active) ----------------------------- */

export const userById = (id: string) => users.find((u) => u.id === id);
export const projectById = (id: string) => projects.find((p) => p.id === id);
export const activitiesForProject = (projectId: string) => activities.filter((a) => a.projectId === projectId);
export const activitiesForUser = (userId: string) => activities.filter((a) => a.userId === userId);

export function projectDistribution() {
  return computeProjectDistribution(projects);
}

export const categoryColorTotals = () => {
  const totals: Record<AppUsage["category"], number> = { productive: 0, neutral: 0, unproductive: 0 };
  for (const a of appCatalog) totals[a.category] += a.minutes;
  return totals;
};

export function categoryTotals() {
  return categoryColorTotals();
}

export function attendanceSummary() {
  const s = { present: 0, late: 0, absent: 0, remote: 0 };
  for (const r of attendance) s[r.status]++;
  return s;
}

export function billingRow(userId: string) {
  return billing.find((b) => b.userId === userId);
}

export function trackedMinutesByUser(list: Activity[] = activities) {
  const map = new Map<string, number>();
  for (const a of list) {
    const p = projects.find((pr) => pr.id === a.projectId);
    const inc = p?.intervalMinutes ?? 10;
    map.set(a.userId, (map.get(a.userId) ?? 0) + inc);
  }
  return map;
}

export function insights(): Insight[] {
  const activeUsers = users.filter((u) => u.productivity > 0);
  if (!activeUsers.length) return [];
  const topPerformer = [...activeUsers].sort((a, b) => b.productivity - a.productivity)[0];
  const mostTracked = [...activeUsers].sort((a, b) => b.trackedToday - a.trackedToday)[0];
  const needsAttention = [...activeUsers].sort((a, b) => a.productivity - b.productivity)[0];
  const cats = categoryTotals();
  const totalCat = cats.productive + cats.neutral + cats.unproductive || 1;
  const focusRatio = Math.round((cats.productive / totalCat) * 100);
  const sortedApps = [...appCatalog].sort((a, b) => b.minutes - a.minutes);
  const topApp = sortedApps[0];

  const out: Insight[] = [
    { id: "i1", tone: "success", title: `${topPerformer.name.split(" ")[0]} is the top performer`, detail: `${topPerformer.productivity}% productivity today — leading the team.` },
    { id: "i2", tone: "info", title: `${mostTracked.name.split(" ")[0]} logged the most time`, detail: `${Math.floor(mostTracked.trackedToday / 60)}h ${mostTracked.trackedToday % 60}m tracked so far today.` },
    { id: "i3", tone: "warning", title: `${needsAttention.name.split(" ")[0]} may need support`, detail: `Lowest productivity at ${needsAttention.productivity}% — consider a check-in.` },
  ];
  if (topApp) out.push({ id: "i4", tone: "info", title: `${topApp.app} dominates usage`, detail: `${Math.floor(topApp.minutes / 60)}h across ${topApp.activeUsers} members this week.` });
  out.push({ id: "i5", tone: focusRatio >= 60 ? "success" : "warning", title: `Team focus is ${focusRatio}%`, detail: `Share of time spent in productive apps this week.` });
  out.push({ id: "i6", tone: "danger", title: `${Math.round(cats.unproductive / 60)}h on distractions`, detail: `Time in unproductive apps this week across the team.` });
  return out;
}
