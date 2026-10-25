import type { Activity, BlockEndReason, DevicePlatform, Project, ScreenMock, User, WindowInfo } from "./types";
import { countSeats } from "./roles";
import { activitiesOnDay, averageProductivity, minutesForProject, minutesOnDay } from "./metrics";
import { appColor } from "./apps";
import { utcOffsetMinutes } from "./time-zone";
import { DAY_MS, NOW, TODAY } from "./demo-clock";

// The frozen clock lives in lib/demo-clock (live code needs it without loading this dataset).
export { DAY_MS, NOW, TODAY };

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

/** Task notes members type into the agent ("What are you working on?"). */
const descriptions = [
  "Activity report – filter panel",
  "Screenshot capture – multi-monitor fix",
  "Project service – refactor",
  "Dashboard charts – weekly trend",
  "Checkout flow – PR review",
  "Timesheet view – design pass",
  "Invite flow – integration tests",
  "Windows agent – sync loop tuning",
  "Onboarding – invitation emails",
  "Dark mode – token polish",
];

/** How many days back the demo generates activity for. */
export const TRACKED_DAY_SPAN = 5;

export interface PlannedSession {
  startedAt: Date;
  endedAt: Date;
  minutes: number;
}

/**
 * Lay out one member's work for one day as realistic back-to-back SESSIONS —
 * roughly 35-75 minutes each, separated by short breaks, the most recent one
 * finishing a little before `anchor`.
 *
 * The sum of the returned sessions is EXACTLY `targetMinutes`: the last session
 * absorbs whatever the jitter left over. That exactness is the whole point —
 * it is what lets `user.trackedToday` be *derived* from these rows instead of
 * authored alongside them, so the dashboard and the reports cannot disagree.
 *
 * Deterministic: it draws only from the seeded `rand` it is handed.
 */
export function planDaySessions(
  targetMinutes: number,
  anchor: Date,
  rand: () => number,
): PlannedSession[] {
  const draw = (min: number, max: number) => Math.floor(rand() * (max - min + 1)) + min;
  const target = Math.max(0, Math.round(targetMinutes));
  if (target === 0) return [];

  const TYPICAL_SESSION = 55;
  const MIN_SESSION = 20;
  const sessions: PlannedSession[] = [];
  let left = Math.max(1, Math.round(target / TYPICAL_SESSION));
  let remaining = target;
  let cursor = anchor.getTime() - draw(5, 25) * 60_000; // clocked off a little while ago

  while (left > 0) {
    // Never spend so much that the sessions still to come fall under the floor;
    // the final session takes exactly what is left, so the total always lands.
    const headroom = remaining - (left - 1) * MIN_SESSION;
    const minutes =
      left === 1
        ? remaining
        : Math.max(MIN_SESSION, Math.min(Math.round(remaining / left) + draw(-12, 12), headroom));
    const endedAt = new Date(cursor);
    const startedAt = new Date(cursor - minutes * 60_000);
    sessions.push({ startedAt, endedAt, minutes });
    remaining -= minutes;
    left -= 1;
    cursor = startedAt.getTime() - draw(4, 14) * 60_000; // short break between blocks
  }
  return sessions;
}

/* ----------------------------- Agent data: devices, idle time, end reasons ----------------------------- */

/** A computer running the desktop agent. */
export interface DemoDevice {
  deviceId: string;
  deviceName: string;
  platform: DevicePlatform;
  osVersion: string;
  agentVersion: string;
}

/** The current agent release; a device still on an older one reads as outdated. */
export const DEMO_AGENT_VERSION = "0.2.0";

/** 32-bit FNV-1a: a stable seed from a string. */
function hashString(text: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

/** A stable, UUID-shaped id from a seed (the agents generate a random one per install). */
function demoUuid(seed: string): string {
  const next = mulberry32(hashString(seed));
  let hex = "";
  while (hex.length < 32) hex += Math.floor(next() * 0x100000000).toString(16).padStart(8, "0");
  const variant = "89ab"[parseInt(hex[16], 16) % 4];
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-4${hex.slice(13, 16)}-${variant}${hex.slice(17, 20)}-${hex.slice(20, 32)}`;
}

function demoDevice(seed: string, deviceName: string, platform: DevicePlatform, osVersion: string, agentVersion = DEMO_AGENT_VERSION): DemoDevice {
  return { deviceId: demoUuid(seed), deviceName, platform, osVersion, agentVersion };
}

const WINDOWS_VERSIONS = ["Windows 11 Pro 23H2 (10.0.22631)", "Windows 11 Pro 24H2 (10.0.26100)", "Windows 10 Pro 22H2 (10.0.19045)"];
const MAC_VERSIONS = ["macOS 14.5 (23F79)", "macOS 15.0 (24A335)", "macOS 14.4.1 (23E224)"];

/** The primary team's computers: a Windows/macOS mix, one member on two machines, one agent a release behind. */
const PRIMARY_DEVICES: Record<string, DemoDevice[]> = {
  u1: [demoDevice("u1-mbp", "Ayesha's MacBook Pro", "macos", "macOS 14.5 (23F79)")],
  u2: [
    demoDevice("u2-desktop", "TANVIR-DESKTOP", "windows", "Windows 11 Pro 23H2 (10.0.22631)"),
    demoDevice("u2-air", "Tanvir's MacBook Air", "macos", "macOS 14.4.1 (23E224)"),
  ],
  u3: [demoDevice("u3-laptop", "NUSRAT-LAPTOP", "windows", "Windows 11 Pro 23H2 (10.0.22631)")],
  u4: [demoDevice("u4-qa", "QA-RIG-02", "windows", "Windows 10 Pro 22H2 (10.0.19045)", "0.1.4")],
  u5: [demoDevice("u5-imac", "Sadia's iMac", "macos", "macOS 14.5 (23F79)")],
  u6: [demoDevice("u6-ws", "IMRAN-WS", "windows", "Windows 11 Pro 24H2 (10.0.26100)")],
  u8: [demoDevice("u8-mbp", "David's MacBook Pro", "macos", "macOS 15.0 (24A335)")],
};

/** The computers a demo member tracks on: authored for the primary team, derived (stably) for everyone else. */
export function demoDevicesFor(user: Pick<User, "id" | "name">): DemoDevice[] {
  const authored = PRIMARY_DEVICES[user.id];
  if (authored) return authored;
  const h = hashString(user.id);
  const first = (user.name.split(" ")[0] ?? "").replace(/[^A-Za-z]/g, "") || "Member";
  return h % 2 === 0
    ? [demoDevice(`${user.id}-mac`, `${first}'s MacBook Pro`, "macos", MAC_VERSIONS[h % MAC_VERSIONS.length])]
    : [demoDevice(`${user.id}-pc`, `${first.toUpperCase()}-PC`, "windows", WINDOWS_VERSIONS[h % WINDOWS_VERSIONS.length])];
}

/**
 * The agent fields of one generated session (contract §5): the computer it was
 * tracked on (a member with two switches between them by day), that device's
 * zone and UTC offset, the idle seconds its Activity % implies (the minutes
 * without input), and its task note. It draws nothing from the generators, so
 * every other value of the session stays exactly what it was.
 */
export function demoAgentFields(user: Pick<User, "id" | "name" | "timezone">, session: Activity, dayIndex: number): Partial<Activity> {
  const devices = demoDevicesFor(user);
  const device = devices[dayIndex % devices.length];
  const start = new Date(session.startedAt);
  const minutes = Math.max(0, Math.round((Date.parse(session.endedAt) - start.getTime()) / 60_000));
  const active = Math.round((minutes * session.productivity) / 100);
  return {
    deviceId: device.deviceId,
    deviceName: device.deviceName,
    platform: device.platform,
    osVersion: device.osVersion,
    agentVersion: device.agentVersion,
    timeZone: user.timezone,
    utcOffsetMinutes: utcOffsetMinutes(start, user.timezone),
    idleSeconds: (minutes - active) * 60,
    note: session.description,
  };
}

/** How the sessions with a gap after them end, weighted the way a real day's gaps are. */
const GAP_END_REASONS: readonly BlockEndReason[] = ["break", "break", "pause", "idle", "lock", "task"];
/** How a member's day ends. */
const DAY_END_REASONS: readonly BlockEndReason[] = ["quit", "quit", "shutdown", "signout", "sleep"];

/**
 * Give every generated session an end reason: a member's last session of a
 * past day ends the day (closed the app, shut down, signed out, put to sleep);
 * today's last one is still being tracked, so it last ended at its interval; a
 * session followed by work on another project ended with the switch; the rest
 * end at a break, a pause, an idle stretch, a locked screen or a new task
 * note. Deterministic per row.
 */
export function assignDemoEndReasons(list: Activity[], today: string): void {
  const days = new Map<string, Activity[]>();
  for (const a of list) {
    const key = `${a.userId}|${a.endedAt.slice(0, 10)}`;
    const rows = days.get(key) ?? [];
    rows.push(a);
    days.set(key, rows);
  }
  for (const rows of days.values()) {
    rows.sort((a, b) => Date.parse(a.startedAt) - Date.parse(b.startedAt));
    rows.forEach((a, i) => {
      const next = rows[i + 1];
      const h = hashString(a.id);
      if (!next) a.endReason = a.endedAt.startsWith(today) ? "interval" : DAY_END_REASONS[h % DAY_END_REASONS.length];
      else if (next.projectId !== a.projectId) a.endReason = "project";
      else a.endReason = GAP_END_REASONS[h % GAP_END_REASONS.length];
    });
  }
}

function buildActivities(): Activity[] {
  const list: Activity[] = [];
  const trackingUsers = users.filter((u) => u.role === "worker" || u.role === "owner" || u.role === "admin");
  let id = 0;

  // Spread activities across the last few days, most recent first.
  for (let day = 0; day < TRACKED_DAY_SPAN; day++) {
    const anchor = new Date(NOW.getTime() - day * DAY_MS);
    for (const user of trackingUsers) {
      // Today must account for exactly the member's intended day; earlier days
      // vary plausibly around it.
      const target =
        day === 0 ? user.trackedToday : Math.round(user.trackedToday * (0.55 + rand() * 0.6));

      planDaySessions(target, anchor, rand).forEach((session, index) => {
        const template = pick(screenTemplates);
        const project = pick(projects.filter((p) => !p.archived && p.memberIds.includes(user.id))) ?? projects[0];
        const { active, running } = makeWindows(template);
        id++;
        const row: Activity = {
          id: `a${id}`,
          userId: user.id,
          projectId: project.id,
          startedAt: session.startedAt.toISOString(),
          endedAt: session.endedAt.toISOString(),
          description: pick(descriptions),
          // Centered on the member's own score so the per-member "activity %"
          // a report computes from these rows matches what /team shows.
          productivity: Math.min(99, Math.max(30, user.productivity + between(-10, 8))),
          // Input volume scales with how long the block actually ran.
          mouseClicks: Math.round(session.minutes * between(3, 11)),
          keyboardHits: Math.round(session.minutes * between(12, 60)),
          activeWindows: active,
          runningPrograms: running,
          screen: template,
          hasWebcam: project.permissions.webcam && rand() > 0.5,
          online: day === 0 && index === 0 ? true : rand() > 0.25,
        };
        list.push(Object.assign(row, demoAgentFields(user, row, day)));
      });
    }
  }
  assignDemoEndReasons(list, NOW.toISOString().slice(0, 10));
  return list.sort((a, b) => +new Date(b.endedAt) - +new Date(a.endedAt));
}

export const activities: Activity[] = buildActivities();

// DERIVE, never duplicate. The `trackedToday` values authored on the roster
// above are the generator's TARGET; from here on the activity rows are the
// single source of truth and the roster is recomputed from them. Previously the
// two were written independently and drifted by 12-19x.
for (const user of users) {
  user.trackedToday = minutesOnDay(activities, TODAY, user.id);
}

/**
 * Same treatment for a project's "this week": the /projects list, the project
 * detail page and the dashboard's project-distribution pie all read
 * `loggedThisWeek`, while the project report aggregates the rows — so the week
 * figure is derived from the rows too.
 *
 * The month and all-time figures reach further back than the generated activity
 * window, so they stay authored; they are only widened when the derived week
 * would otherwise exceed the period that contains it.
 */
export function syncProjectTotals(projectList: Project[], activityList: Activity[]): void {
  const weekAgo = new Date(NOW.getTime() - 7 * DAY_MS);
  for (const project of projectList) {
    project.loggedThisWeek = minutesForProject(activityList, project.id, weekAgo, NOW);
    project.loggedThisMonth = Math.max(project.loggedThisMonth, project.loggedThisWeek);
    project.loggedTotal = Math.max(project.loggedTotal, project.loggedThisMonth);
  }
}
syncProjectTotals(projects, activities);

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

/** Coloured by lib/apps, so an app has the same colour here as on every other screen. */
export const topApps = [
  { app: "Visual Studio Code", minutes: 1840 },
  { app: "Google Chrome", minutes: 1120 },
  { app: "Figma", minutes: 760 },
  { app: "Slack", minutes: 540 },
  { app: "Terminal", minutes: 430 },
  { app: "Notion", minutes: 260 },
].map((a) => ({ ...a, color: appColor(a.app) }));

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

// Both time figures come straight off the activity rows via metrics.ts — the
// same derivation every report runs — so the dashboard KPI and the reports are
// arithmetically the same number, not two independent guesses at it.
export const summary = {
  totalTrackedToday: minutesOnDay(activities, TODAY),
  activeMembers: users.filter((u) => u.status === "active").length,
  totalMembers: countSeats(users),
  activeProjects: projects.filter((p) => !p.archived).length,
  avgProductivity: averageProductivity(activitiesOnDay(activities, TODAY)),
  screenshotsToday: 284,
};
