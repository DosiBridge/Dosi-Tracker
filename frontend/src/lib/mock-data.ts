import type { Activity, Project, ScreenMock, User, WindowInfo } from "./types";
import { countSeats } from "./roles";

/** Fixed reference time so SSR and client render identically (no hydration drift). */
export const NOW = new Date("2026-07-14T15:30:00.000Z");

function mulberry32(seed: number) {
  return function () {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const rand = mulberry32(20260714);
const pick = <T>(arr: T[]) => arr[Math.floor(rand() * arr.length)];
const between = (min: number, max: number) => Math.floor(rand() * (max - min + 1)) + min;

/* ----------------------------- Users ----------------------------- */

export const users: User[] = [
  {
    id: "u1", name: "Ayesha Rahman", email: "ayesha@dosi.dev", role: "owner",
    designation: "Engineering Lead", status: "active", timezone: "Asia/Dhaka",
    trackedToday: 372, productivity: 88, joinedAt: "2024-02-11",
  },
  {
    id: "u2", name: "Tanvir Hasan", email: "tanvir@dosi.dev", role: "worker",
    designation: "Frontend Developer", status: "active", timezone: "Asia/Dhaka",
    trackedToday: 344, productivity: 82, joinedAt: "2024-05-02",
  },
  {
    id: "u3", name: "Nusrat Jahan", email: "nusrat@dosi.dev", role: "worker",
    designation: "Backend Developer", status: "idle", timezone: "Asia/Dhaka",
    trackedToday: 298, productivity: 79, joinedAt: "2024-06-18",
  },
  {
    id: "u4", name: "Rafiq Islam", email: "rafiq@dosi.dev", role: "worker",
    designation: "QA Engineer", status: "active", timezone: "Asia/Dhaka",
    trackedToday: 265, productivity: 71, joinedAt: "2024-08-01",
  },
  {
    id: "u5", name: "Sadia Akter", email: "sadia@dosi.dev", role: "worker",
    designation: "UI/UX Designer", status: "offline", timezone: "Asia/Dhaka",
    trackedToday: 210, productivity: 64, joinedAt: "2024-09-20",
  },
  {
    id: "u6", name: "Imran Kabir", email: "imran@dosi.dev", role: "worker",
    designation: "DevOps Engineer", status: "active", timezone: "Asia/Dhaka",
    trackedToday: 318, productivity: 85, joinedAt: "2025-01-14",
  },
  {
    id: "u7", name: "Maria Gomez", email: "maria@acme.com", role: "client",
    designation: "Client · Acme Corp", status: "offline", timezone: "Europe/Madrid",
    trackedToday: 0, productivity: 0, joinedAt: "2025-03-03",
  },
  {
    id: "u8", name: "David Chen", email: "david@dosi.dev", role: "admin",
    designation: "System Administrator", status: "idle", timezone: "America/Los_Angeles",
    trackedToday: 142, productivity: 74, joinedAt: "2023-11-09",
  },
];

export const currentUser = users[0];

/* ----------------------------- Projects ----------------------------- */

export const projects: Project[] = [
  {
    id: "p1", title: "Dosi Web Platform", description: "Core SaaS dashboard & REST API.",
    color: "#6d5efc", archived: false, intervalMinutes: 10,
    permissions: { screenshot: true, webcam: false, keyboard: true, mouse: true, activeWindow: true, runningPrograms: true },
    memberIds: ["u1", "u2", "u3", "u6", "u8"], createdAt: "2025-04-02",
    loggedThisWeek: 1840, loggedThisMonth: 7320, loggedTotal: 41200,
  },
  {
    id: "p2", title: "Mobile App Revamp", description: "Cross-platform tracker clients.",
    color: "#ec4899", archived: false, intervalMinutes: 15,
    permissions: { screenshot: true, webcam: true, keyboard: true, mouse: true, activeWindow: true, runningPrograms: false },
    memberIds: ["u2", "u4", "u5"], createdAt: "2025-05-19",
    loggedThisWeek: 1210, loggedThisMonth: 5140, loggedTotal: 18600,
  },
  {
    id: "p3", title: "Acme Corp CRM", description: "Client engagement portal for Acme.",
    color: "#0ea5e9", archived: false, intervalMinutes: 10,
    permissions: { screenshot: true, webcam: false, keyboard: true, mouse: true, activeWindow: true, runningPrograms: true },
    memberIds: ["u1", "u3", "u4", "u7", "u8"], createdAt: "2025-06-08",
    loggedThisWeek: 980, loggedThisMonth: 4020, loggedTotal: 9800,
  },
  {
    id: "p4", title: "Marketing Website", description: "Public site & landing pages.",
    color: "#22c55e", archived: false, intervalMinutes: 20,
    permissions: { screenshot: true, webcam: false, keyboard: false, mouse: true, activeWindow: true, runningPrograms: false },
    memberIds: ["u5", "u2"], createdAt: "2025-06-25",
    loggedThisWeek: 540, loggedThisMonth: 2260, loggedTotal: 6100,
  },
  {
    id: "p5", title: "Data Pipeline", description: "Analytics ingestion & ETL jobs.",
    color: "#f59e0b", archived: true, intervalMinutes: 15,
    permissions: { screenshot: false, webcam: false, keyboard: true, mouse: true, activeWindow: true, runningPrograms: true },
    memberIds: ["u3", "u6"], createdAt: "2025-01-30",
    loggedThisWeek: 0, loggedThisMonth: 0, loggedTotal: 22400,
  },
  {
    id: "p6", title: "Design System", description: "Shared component & token library.",
    color: "#a855f7", archived: false, intervalMinutes: 10,
    permissions: { screenshot: true, webcam: false, keyboard: true, mouse: true, activeWindow: true, runningPrograms: true },
    memberIds: ["u5", "u1", "u2"], createdAt: "2025-03-12",
    loggedThisWeek: 720, loggedThisMonth: 3180, loggedTotal: 15400,
  },
];

/* ----------------------------- Activities ----------------------------- */

const screenTemplates: ScreenMock[] = [
  { app: "Visual Studio Code", kind: "editor", accent: "#3b82f6" },
  { app: "Google Chrome", kind: "browser", accent: "#f97316" },
  { app: "Figma", kind: "design", accent: "#ec4899" },
  { app: "Slack", kind: "chat", accent: "#a855f7" },
  { app: "Terminal", kind: "terminal", accent: "#22c55e" },
  { app: "Notion", kind: "docs", accent: "#64748b" },
];

const windowTitles: Record<string, string[]> = {
  "Visual Studio Code": ["dashboard.tsx — dosi-tracker", "api.rs — clients/windows", "ProjectAppService.cs", "main.swift — DosiTracker"],
  "Google Chrome": ["abp.io/docs — ABP Framework", "Next.js Docs", "localhost:3000/dashboard", "Stack Overflow — event tap"],
  Figma: ["Dosi · Dashboard v3", "Design System / Components", "Mobile App — Flows", "Marketing / Hero"],
  Slack: ["#engineering", "#dosi-standup", "Direct messages", "#design-review"],
  Terminal: ["cargo run --release", "abp new · Dosi.Tracker", "npm run dev", "git rebase -i"],
  Notion: ["Sprint 24 Planning", "Architecture RFC", "Release checklist", "Meeting notes"],
};

const otherApps = ["Postman", "Docker Desktop", "Spotify", "1Password", "Zoom", "Linear"];

function makeWindows(primary: ScreenMock): { active: WindowInfo[]; running: WindowInfo[] } {
  const active: WindowInfo[] = [
    { appName: primary.app, windowTitle: pick(windowTitles[primary.app]), seconds: between(180, 520) },
    { appName: pick(screenTemplates).app, windowTitle: "—", seconds: between(40, 160) },
  ];
  const running: WindowInfo[] = [
    { appName: primary.app, windowTitle: pick(windowTitles[primary.app]), seconds: 0 },
    { appName: pick(otherApps), windowTitle: "—", seconds: 0 },
    { appName: pick(otherApps), windowTitle: "—", seconds: 0 },
  ];
  return { active, running };
}

const descriptions = [
  "Implementing activity report filters",
  "Fixing screenshot capture on multi-monitor",
  "Refactoring project app service",
  "Building dashboard charts",
  "Reviewing pull requests",
  "Designing the timesheet view",
  "Writing integration tests",
  "Tuning the Rust agent sync loop",
  "Wiring up the invitation flow",
  "Polishing dark mode tokens",
];

function buildActivities(): Activity[] {
  const list: Activity[] = [];
  const trackingUsers = users.filter((u) => u.role === "worker" || u.role === "owner" || u.role === "admin");
  let id = 0;

  // Spread activities across the last 5 days, most recent first.
  for (let day = 0; day < 5; day++) {
    for (const user of trackingUsers) {
      const blocks = day === 0 ? between(2, 4) : between(1, 3);
      for (let b = 0; b < blocks; b++) {
        const template = pick(screenTemplates);
        const project = pick(projects.filter((p) => !p.archived && p.memberIds.includes(user.id))) ?? projects[0];
        const minutesAgo = day * 24 * 60 + b * between(35, 90) + between(5, 25);
        const ended = new Date(NOW.getTime() - minutesAgo * 60000);
        const durationMin = project.intervalMinutes;
        const started = new Date(ended.getTime() - durationMin * 60000);
        const { active, running } = makeWindows(template);
        id++;
        list.push({
          id: `a${id}`,
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

export const activities: Activity[] = buildActivities();

/* ----------------------------- Derived / chart data ----------------------------- */

export const weeklyTrend = [
  { day: "Mon", tracked: 412, productive: 342 },
  { day: "Tue", tracked: 448, productive: 388 },
  { day: "Wed", tracked: 396, productive: 300 },
  { day: "Thu", tracked: 470, productive: 402 },
  { day: "Fri", tracked: 512, productive: 451 },
  { day: "Sat", tracked: 188, productive: 150 },
  { day: "Sun", tracked: 96, productive: 70 },
];

export const hourlyToday = Array.from({ length: 12 }, (_, i) => {
  const hour = 8 + i; // 8am..7pm
  const base = [20, 42, 55, 48, 30, 12, 38, 52, 58, 46, 33, 18][i];
  return { hour: `${hour}:00`, minutes: base };
});

export function projectDistribution() {
  return projects
    .filter((p) => !p.archived)
    .map((p) => ({ name: p.title, value: p.loggedThisWeek, color: p.color }))
    .sort((a, b) => b.value - a.value);
}

export const topApps = [
  { app: "Visual Studio Code", minutes: 1840, color: "#3b82f6" },
  { app: "Google Chrome", minutes: 1120, color: "#f97316" },
  { app: "Figma", minutes: 760, color: "#ec4899" },
  { app: "Slack", minutes: 540, color: "#a855f7" },
  { app: "Terminal", minutes: 430, color: "#22c55e" },
  { app: "Notion", minutes: 260, color: "#64748b" },
];

/* ----------------------------- Lookups ----------------------------- */

export const userById = (id: string) => users.find((u) => u.id === id);
export const projectById = (id: string) => projects.find((p) => p.id === id);

export function activitiesForProject(projectId: string) {
  return activities.filter((a) => a.projectId === projectId);
}
export function activitiesForUser(userId: string) {
  return activities.filter((a) => a.userId === userId);
}

/* ----------------------------- Global summary ----------------------------- */

export const summary = {
  totalTrackedToday: users.reduce((s, u) => s + u.trackedToday, 0),
  activeMembers: users.filter((u) => u.status === "active").length,
  totalMembers: countSeats(users),
  activeProjects: projects.filter((p) => !p.archived).length,
  avgProductivity: Math.round(
    users.filter((u) => u.productivity > 0).reduce((s, u) => s + u.productivity, 0) /
      users.filter((u) => u.productivity > 0).length
  ),
  screenshotsToday: 284,
};
