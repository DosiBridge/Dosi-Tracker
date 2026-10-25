import type { Activity, ActivityCoverage, Project, User } from "./types";
import type {
  AppUsage,
  AttendanceRecord,
  BillingRow,
  AppNotification,
  Insight,
  ProductivitySplit,
} from "./reports-data";
import { countSeats } from "./roles";
import { activitiesOnDay, activitiesOnZonedDay, averageProductivity, minutesByUser, totalMinutes } from "./metrics";
import {
  applyProjectTotals,
  deriveAppCatalog,
  deriveDailyTrend,
  deriveHourly,
  deriveTopApps,
  type ProjectTotals,
  type SummaryTotals,
} from "./live-dataset";
import { readStorage, writeStorage } from "./live-session";
import { zonedDayKey } from "./time-zone";
import { DAY_MS, NOW } from "./demo-clock";

export { NOW };

/* ============================================================================
 * A Dataset is the full, tenant-isolated data for a single workspace: a live
 * tenant's, built from the backend, or (only in a deployment that turns demo
 * mode on) a demo tenant's, built by lib/demo-tenants, which is loaded on
 * demand so a live deployment never downloads or generates the demo data.
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
  /** True for a dataset built from the real backend (see buildLiveDataset). */
  live?: boolean;
  /**
   * Live datasets only: how complete `activities` is. When `truncated`, the
   * rows are the newest ones and lists or charts built from rows before
   * `completeFrom` are partial — say so ("showing the newest N of M").
   * Totals (project week/month, today per member) come from the server and
   * are exact either way.
   */
  activityCoverage?: ActivityCoverage | null;
  /** Live datasets only: the reporting zone "today" was counted in. */
  timeZone?: string;
}


export function computeProjectDistribution(projects: Project[]) {
  return projects
    .filter((p) => !p.archived)
    .map((p) => ({ name: p.title, value: p.loggedThisWeek, color: p.color }))
    .sort((a, b) => b.value - a.value);
}

/* ----------------------------- Registry ----------------------------- */

const cache = new Map<string, Dataset>();

/** Live workspace ids (see live-identity.ts) all start with this prefix. */
const LIVE_ID_PREFIX = "live-";

/** How demo mode plugs its tenants in (lib/demo-tenants registers one). */
export interface DemoDatasets {
  /** The dataset of a demo workspace id (an unknown id: an empty, freshly created demo workspace). */
  build: (workspaceId: string) => Dataset;
  /** The workspace a demo visitor lands in (and returns to after a sign-out). */
  primaryWorkspaceId: string;
}

let demo: DemoDatasets | null = null;

/**
 * Plug the demo tenants in. Only lib/demo-tenants calls this, and only a
 * deployment with demo mode on loads that module. Datasets read before
 * (empty ones) are forgotten so the demo's replace them, and unless a live
 * tenant is active the primary demo workspace becomes the active one.
 */
export function registerDemoDatasets(datasets: DemoDatasets): void {
  demo = datasets;
  for (const [id, ds] of cache) {
    if (!ds.live) cache.delete(id);
  }
  if (!active.live) applyActive(datasetFor(datasets.primaryWorkspaceId));
}

/** Whether the demo tenants are loaded (demo mode, once lib/demo-tenants has run). */
export function demoDatasetsInstalled(): boolean {
  return demo !== null;
}

/** A dataset with nothing in it: what any non-live workspace reads outside demo mode. Never demo rows. */
function blankDataset(workspaceId: string): Dataset {
  return {
    workspaceId,
    users: [],
    projects: [],
    activities: [],
    weeklyTrend: [],
    hourlyToday: [],
    topApps: [],
    summary: { totalTrackedToday: 0, activeMembers: 0, totalMembers: 0, activeProjects: 0, avgProductivity: 0, screenshotsToday: 0 },
    projectDistribution: [],
    appCatalog: [],
    productivitySplit: [],
    attendance: [],
    billing: [],
    notifications: [],
  };
}

/** Demo projects created in this browser, per workspace. */
const createdProjectsKey = (workspaceId: string) => `dosi-projects-created-${workspaceId}`;

function readCreatedProjects(workspaceId: string): Project[] {
  const val = readStorage(createdProjectsKey(workspaceId));
  // Guard the parse: a corrupted value (e.g. "null" or "{}") must fall back
  // to "no local projects" instead of crashing `datasetFor` on `.length`.
  try {
    const parsed: unknown = val ? JSON.parse(val) : [];
    return Array.isArray(parsed) ? (parsed as Project[]) : [];
  } catch {
    return [];
  }
}

export function datasetFor(workspaceId: string): Dataset {
  const cached = cache.get(workspaceId);
  if (cached) return cached;

  let ds: Dataset;
  // A live workspace not hydrated yet reads as EMPTY — never as a demo tenant.
  if (workspaceId.startsWith(LIVE_ID_PREFIX)) {
    ds = buildLiveDataset({ workspaceId, users: [], projects: [], activities: [], now: new Date() });
  } else if (demo) {
    ds = demo.build(workspaceId);
    const localProjects = readCreatedProjects(workspaceId);
    if (localProjects.length > 0) {
      ds.projects = [...localProjects, ...ds.projects];
    }
  } else {
    ds = blankDataset(workspaceId);
  }

  cache.set(workspaceId, ds);
  return ds;
}

export function createTenantProject(workspaceId: string, project: Project): void {
  const ds = datasetFor(workspaceId);
  if (!ds.projects.some((p) => p.id === project.id)) {
    ds.projects = [project, ...ds.projects];
  }

  // Remembered for the next visit when the browser allows it; blocked storage only means it lasts this page view.
  const list = readCreatedProjects(workspaceId);
  if (!list.some((p) => p.id === project.id)) {
    list.unshift(project);
    writeStorage(createdProjectsKey(workspaceId), JSON.stringify(list));
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

// Nothing until the session decides: a live tenant's data, or (demo mode) a demo tenant's.
let active: Dataset = blankDataset("");

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

function applyActive(ds: Dataset): void {
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

export function setActiveWorkspace(workspaceId: string): void {
  const ds = datasetFor(workspaceId);
  if (ds === active) return;
  applyActive(ds);
}

/* ============================================================================
 * LIVE datasets — a real tenant's data, built from the backend by the session
 * provider. Nothing here is generated: every series is derived from the real
 * rows (live-dataset.ts) and anything the backend does not provide is EMPTY —
 * no demo people, notifications, attendance, billing rates or app categories.
 * ========================================================================== */

export interface LiveDatasetInput {
  workspaceId: string;
  users: User[];
  projects: Project[];
  activities: Activity[];
  /** The real clock ("today" for live data is today, not the demo's frozen NOW). */
  now: Date;
  /** The reporting zone whose calendar day is "today" (default UTC). */
  timeZone?: string;
  /** How complete `activities` is (see Dataset.activityCoverage). */
  coverage?: ActivityCoverage | null;
  /**
   * Exact totals from the server, preferred over sums of `activities` (which
   * may be partial): per project for the week / month / loaded window, and
   * today's team total and Activity %.
   */
  totals?: { projects?: ProjectTotals; today?: SummaryTotals | null };
}

/** "This week" on the dashboard: the last 7 days. */
const WEEK_MS = 7 * DAY_MS;

export function buildLiveDataset({
  workspaceId,
  users: liveUsers,
  projects: liveProjects,
  activities: liveActivities,
  now,
  timeZone,
  coverage,
  totals,
}: LiveDatasetInput): Dataset {
  const projectsCopy = liveProjects.map((p) => ({ ...p, memberIds: Array.isArray(p.memberIds) ? [...p.memberIds] : [] }));
  applyProjectTotals(projectsCopy, liveActivities, now, { timeZone, totals: totals?.projects });
  const today = timeZone ? zonedDayKey(now, timeZone) : now.toISOString().slice(0, 10);
  const todays = timeZone ? activitiesOnZonedDay(liveActivities, today, timeZone) : activitiesOnDay(liveActivities, today);
  // "This week" means the last 7 days — never the whole preloaded window.
  const appCatalog = deriveAppCatalog(liveActivities, { from: new Date(now.getTime() - WEEK_MS), to: now });
  const todayTotals = totals?.today ?? null;
  return {
    workspaceId,
    users: liveUsers,
    projects: projectsCopy,
    activities: liveActivities,
    weeklyTrend: deriveDailyTrend(liveActivities, now, 7, timeZone),
    hourlyToday: timeZone ? deriveHourly(liveActivities, now, timeZone) : deriveHourly(liveActivities, now),
    topApps: deriveTopApps(appCatalog),
    summary: {
      totalTrackedToday: Math.round(todayTotals ? todayTotals.totalMinutes : totalMinutes(todays)),
      activeMembers: liveUsers.filter((u) => u.status === "active").length,
      totalMembers: countSeats(liveUsers),
      activeProjects: projectsCopy.filter((p) => !p.archived).length,
      avgProductivity: todayTotals ? (todayTotals.activity ?? 0) : averageProductivity(todays),
      screenshotsToday: todays.length,
    },
    projectDistribution: computeProjectDistribution(projectsCopy),
    appCatalog,
    productivitySplit: [],
    attendance: [],
    billing: [],
    notifications: [],
    live: true,
    activityCoverage: coverage ?? null,
    ...(timeZone ? { timeZone } : {}),
  };
}

/** The active live dataset's row coverage (null for demo data and before a live load). */
export function activityCoverage(): ActivityCoverage | null {
  return active.live ? (active.activityCoverage ?? null) : null;
}

/** Register a live dataset under its workspace id and make it the active one. */
export function installLiveDataset(input: LiveDatasetInput): Dataset {
  const ds = buildLiveDataset(input);
  cache.set(ds.workspaceId, ds);
  applyActive(ds);
  return ds;
}

/**
 * An EMPTY live dataset (identity still loading, or the backend failed). Keeps
 * every page's reads valid without ever falling back to demo rows.
 */
export function installEmptyLiveDataset(workspaceId: string, now: Date = new Date()): Dataset {
  return installLiveDataset({ workspaceId, users: [], projects: [], activities: [], now });
}

/** Whether the ACTIVE dataset is a real tenant's (vs the demo). */
export function isLiveDataset(): boolean {
  return active.live === true;
}

/** Forget every live dataset (sign-out): back to the primary demo workspace in demo mode, else to nothing. */
export function dropLiveDatasets(): void {
  for (const [id, ds] of cache) {
    if (ds.live) cache.delete(id);
  }
  if (active.live) applyActive(demo ? datasetFor(demo.primaryWorkspaceId) : blankDataset(""));
}

export function hydrateLiveBackendData(liveProjects: Project[], liveActivities: Activity[]) {
  active.projects = liveProjects;
  active.activities = liveActivities;
  projects = liveProjects;
  activities = liveActivities;
  
  // Update the cache as well
  const ds = cache.get(active.workspaceId);
  if (ds) {
    ds.projects = liveProjects;
    ds.activities = liveActivities;
  }
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

/** Minutes per member. Delegates to metrics.ts — the one definition of "tracked". */
export function trackedMinutesByUser(list: Activity[] = activities) {
  return minutesByUser(list);
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

/* ============================================================================
 * Test-only reset
 * ========================================================================== */

/**
 * Drop every cached dataset and re-point the live bindings at the primary
 * demo workspace (or at nothing outside demo mode). lib/demo-tenants'
 * resetTenantDataForTests restores its seed rosters and then calls this.
 * Imported ONLY by test code and that reset, never by app code.
 */
export function resetTenantRegistryForTests(): void {
  cache.clear();
  applyActive(demo ? datasetFor(demo.primaryWorkspaceId) : blankDataset(""));
}
