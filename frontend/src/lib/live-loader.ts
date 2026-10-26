import type { Activity, ActivityCoverage, Project, User, UserStatus } from "./types";
import type { Workspace } from "./saas-data";
import {
  API_ROUTES,
  BILLING_MODE_ROUTE,
  MAX_PAGE_SIZE,
  collectPages,
  projectMembersRoute,
  reportingRoute,
  toPagedResult,
  type PagedResultDto,
} from "./api-contract";
import {
  displayName,
  mapApiActivities,
  mapApiProject,
  mapReportSummary,
  mapTeamMember,
  memberIdsByProject,
  presenceStatusByUser,
  roleDesignation,
  type ApiProjectDto,
  type ApiTeamMemberDto,
  type ProjectTotals,
  type SummaryTotals,
} from "./live-dataset";
import {
  buildLiveUser,
  buildLiveWorkspace,
  isAuthenticatedConfig,
  isClientOnly,
  isHostConfig,
  isPlatformAdminConfig,
  type ApiPlanDto,
  type ApiSubscriptionDto,
  type AppConfiguration,
  type MembershipRole,
} from "./live-identity";
import { grantedPoliciesOf } from "./report-math";
import { browserTimeZone, effectiveTimeZone, startOfZonedDay, zonedDayKey } from "./time-zone";

/* ============================================================================
 * LIVE SESSION LOADER — one round-trip set that turns a bearer token into the
 * signed-in user, their real workspace and the tenant's real rows.
 *
 * Kept in the pure lib layer (the HTTP getters are injected) so the
 * orchestration — which calls are required, which may fail softly, how the
 * roster, projects and seats are stitched together — is unit-tested without
 * React or a network.
 *
 * Two kinds of numbers come out of it:
 *  - TOTALS (today per member, this week / month / the loaded window per
 *    project) come from /api/app/reporting/summary, which adds up every block
 *    on the server, so they are exact however large the team is;
 *  - ROWS (the activity feed, row-derived charts) are paged in, newest first,
 *    up to MAX_LIVE_ACTIVITY_ROWS. `coverage` says whether that list is
 *    complete, and from when, so a page can say "showing the newest N of M"
 *    instead of silently under-counting.
 * Without the summary (an older server, or a failed call) the totals fall back
 * to the rows.
 *
 * A PULSE (a silent refresh while the tab is open) passes the previous load:
 * identity, roster, projects, presence and totals are read again, but only the
 * last few hours of rows are re-read and merged into the ones already held.
 * ========================================================================== */

/** GET helper with getApi semantics: resolves the JSON (items unwrapped), rejects on failure. */
export type LiveGetter = (endpoint: string) => Promise<unknown>;

/** GET helper with getPagedApi semantics: one page as `{ items, totalCount }`, rejects on failure. */
export type LivePageGetter = (endpoint: string) => Promise<PagedResultDto<unknown>>;

/** The token is present but the backend no longer recognises it as a signed-in user. */
export class SessionExpiredError extends Error {
  constructor() {
    super("Your session has expired. Please sign in again.");
    this.name = "SessionExpiredError";
  }
}

/** Signed in, but no workspace is chosen (or the chosen one is no longer theirs): go choose one. */
export class WorkspaceRequiredError extends Error {
  constructor() {
    super("Choose a workspace to continue.");
    this.name = "WorkspaceRequiredError";
  }
}

/** Data that could not be loaded (the rest still renders); see describeWarnings. */
export type LiveWarning = "projects" | "activity" | "team" | "plan";

export type { ActivityCoverage };

/** Who a live session belongs to; another tab signing in as someone else changes it. */
export interface LiveIdentity {
  userId: string;
  /** Null for the platform (host) account. */
  tenantId: string | null;
  /** The workspace's own name (null for the platform account). */
  tenantName: string | null;
  /** The sign-in name (for signing in again in place). */
  userName: string;
}

export interface LiveSessionData {
  user: User;
  workspace: Workspace;
  users: User[];
  projects: Project[];
  activities: Activity[];
  /** Data that could not be loaded (the rest still renders). */
  warnings: LiveWarning[];
  coverage: ActivityCoverage;
  /** The reporting zone "today" and "this month" were counted in. */
  timeZone: string;
  identity: LiveIdentity;
  /** Exact per-period totals from the server (null where unavailable). */
  totals: { projects: ProjectTotals; today: SummaryTotals | null };
  /** The plan as loaded, reused by pulses (null where the call failed); `billing` is the deployment's billing mode. */
  plan: { subscription: ApiSubscriptionDto | null; plans: ApiPlanDto[] | null; billing?: { enabled: boolean } | null };
  /**
   * The viewer's granted permissions (ABP `auth.grantedPolicies`, re-read on
   * every load), so screens can tell who may see what without asking again;
   * null when the configuration didn't carry them.
   */
  grantedPolicies: Record<string, boolean> | null;
  /** When this load ran (ISO). */
  loadedAt: string;
}

/** How far back the session preloads activity rows for dashboards and pickers. */
export const LIVE_ACTIVITY_WINDOW_DAYS = 30;

/**
 * The most activity rows a session holds (about two weeks of a 10-person team
 * on 10-minute blocks). Beyond it the newest rows are kept and `coverage`
 * reports the list as partial; totals still come from the server.
 */
export const MAX_LIVE_ACTIVITY_ROWS = 5000;

/** A pulse re-reads the blocks that started in this span (blocks last at most 2 h, contract §1). */
export const PULSE_LOOKBACK_MS = 3 * 60 * 60 * 1000;

/** The most project rosters read to tell whether a member only takes part as a client. */
const MAX_ROLE_CHECKS = 20;

const DAY_MS = 24 * 60 * 60 * 1000;

const iso = (d: Date) => encodeURIComponent(d.toISOString());

export const LIVE_ENDPOINTS = {
  configuration: "/api/abp/application-configuration?IncludeLocalizationResources=false",
  mine: "/api/app/workspaces/mine",
  members: "/api/app/team/members",
  projects: "/api/app/project?MaxResultCount=1000",
  subscription: "/api/app/workspace/current-subscription",
  plans: "/api/app/workspace/available-plans",
  billingMode: BILLING_MODE_ROUTE,
  presence: API_ROUTES.presenceTeam,
  myPreferences: API_ROUTES.myPreferences,
  workspaceSettings: API_ROUTES.workspaceSettings,
  /** Blocks that STARTED in [from, to); paging parameters are added per page. */
  activities: (from: Date, to: Date) => `/api/app/activity?From=${iso(from)}&To=${iso(to)}`,
  /** Totals over [from, to); the zone is sent like on every reporting call (its "days" are the zone's). */
  summary: (from: Date, to: Date, timeZone: string) => reportingRoute("summary", { from, to, timeZone }),
  projectMembers: (projectId: string) => projectMembersRoute(projectId),
} as const;

/** A failed call resolves to null (the caller decides whether that is a warning). */
const soft = <T>(p: Promise<T>): Promise<T | null> => p.catch(() => null);

export interface LiveLoadOptions {
  /**
   * GET one page keeping the server's totalCount (getPagedApi). Without it the
   * pages come from `get`, whose unwrapped list carries no total, so reading
   * stops after the first page.
   */
  getPage?: LivePageGetter;
  /** The previous load: a pulse merges into it (ignored when it belongs to someone else). */
  previous?: LiveSessionData | null;
  /**
   * Read the subscription, plans and billing mode again even on a pulse (after
   * the owner changed or canceled the plan); otherwise a pulse reuses them.
   */
  freshPlan?: boolean;
  /** The browser's zone, the last fallback for the reporting zone (default: read from the platform). */
  browserZone?: string;
  /**
   * The current workspace id (sent as X-Workspace-Id). When given, the load
   * checks it against /api/app/workspaces/mine; null means none is chosen.
   * Undefined keeps the configuration's own scope (tests, older servers).
   */
  workspaceId?: string | null;
}

/* ----------------------------- Pieces ----------------------------- */

function identityOf(config: AppConfiguration): LiveIdentity {
  const cu = config.currentUser ?? {};
  const tenantId = config.currentWorkspace?.id ?? config.currentTenant?.id ?? cu.tenantId ?? null;
  return {
    userId: String(cu.id ?? ""),
    tenantId,
    tenantName: tenantId ? (config.currentWorkspace?.name ?? config.currentTenant?.name ?? "").trim() || null : null,
    userName: (cu.userName ?? cu.email ?? "").trim(),
  };
}

function sameIdentity(a: LiveIdentity, b: LiveIdentity): boolean {
  return a.userId === b.userId && (a.tenantId ?? "").toLowerCase() === (b.tenantId ?? "").toLowerCase();
}

function stringField(value: unknown, field: string): string | null {
  if (!value || typeof value !== "object") return null;
  const v = (value as Record<string, unknown>)[field];
  return typeof v === "string" ? v : null;
}

/** The reporting zone: the member's preference, then the workspace default, then the browser (contract §7). */
async function reportingZone(get: LiveGetter, browserZone: string): Promise<string> {
  const [preferences, settings] = await Promise.all([
    soft(get(LIVE_ENDPOINTS.myPreferences)),
    soft(get(LIVE_ENDPOINTS.workspaceSettings)),
  ]);
  return effectiveTimeZone({
    user: stringField(preferences, "timeZone"),
    workspace: stringField(settings, "defaultTimeZone"),
    browser: browserZone,
  }).timeZone;
}

interface RowLoad {
  activities: Activity[];
  coverage: ActivityCoverage;
  failed: boolean;
}

function emptyCoverage(from: Date, to: Date): ActivityCoverage {
  return { from: from.toISOString(), to: to.toISOString(), loaded: 0, total: 0, truncated: false, completeFrom: from.toISOString() };
}

const startOf = (a: Activity) => Date.parse(a.startedAt);
const newestFirst = (a: Activity, b: Activity) => Date.parse(b.endedAt) - Date.parse(a.endedAt) || (a.id < b.id ? 1 : a.id > b.id ? -1 : 0);

/** Page through [from, to) newest first, up to the row bound. */
async function readRows(getPage: LivePageGetter, from: Date, to: Date) {
  const collected = await collectPages<unknown>(getPage, LIVE_ENDPOINTS.activities(from, to), {
    pageSize: MAX_PAGE_SIZE,
    maxItems: MAX_LIVE_ACTIVITY_ROWS,
  });
  return { rows: mapApiActivities(collected.items), total: collected.totalCount, truncated: collected.truncated };
}

/** The oldest start among rows (or `fallback` when there are none). */
function oldestStart(rows: Activity[], fallback: number): number {
  return rows.reduce((min, a) => Math.min(min, startOf(a)), fallback);
}

/** A full read of the session window. */
async function loadAllRows(getPage: LivePageGetter, from: Date, to: Date): Promise<RowLoad> {
  try {
    const { rows, total, truncated } = await readRows(getPage, from, to);
    // Rows come newest first, so a cut list is complete from its oldest row on.
    const completeFrom = truncated ? new Date(oldestStart(rows, to.getTime())) : from;
    return {
      activities: rows,
      coverage: {
        from: from.toISOString(),
        to: to.toISOString(),
        loaded: rows.length,
        total: Math.max(total, rows.length),
        truncated,
        completeFrom: completeFrom.toISOString(),
      },
      failed: false,
    };
  } catch {
    return { activities: [], coverage: emptyCoverage(from, to), failed: true };
  }
}

/**
 * A pulse: re-read only the blocks that started in the last PULSE_LOOKBACK_MS
 * and merge them into the rows already held (fresh rows win; rows that slid
 * out of the window are dropped; a held row inside the re-read span that the
 * server no longer returns is gone — unless the re-read itself was cut short).
 */
async function pulseRows(getPage: LivePageGetter, previous: LiveSessionData, from: Date, to: Date): Promise<RowLoad> {
  const since = new Date(Math.max(from.getTime(), to.getTime() - PULSE_LOOKBACK_MS));
  let fresh: Awaited<ReturnType<typeof readRows>>;
  try {
    fresh = await readRows(getPage, since, to);
  } catch {
    return { activities: previous.activities, coverage: previous.coverage, failed: true };
  }
  const freshIds = new Set(fresh.rows.map((a) => a.id));
  const known = new Set(previous.activities.map((a) => a.id));
  let slidOut = 0;
  const kept = previous.activities.filter((a) => {
    const start = startOf(a);
    if (!(start >= from.getTime())) {
      slidOut++;
      return false;
    }
    if (freshIds.has(a.id)) return false;
    return start < since.getTime() || fresh.truncated;
  });
  const activities = [...fresh.rows, ...kept].sort(newestFirst);
  const added = fresh.rows.filter((a) => !known.has(a.id)).length;
  const prior = previous.coverage;
  const completeFrom = Math.max(
    from.getTime(),
    fresh.truncated ? oldestStart(fresh.rows, to.getTime()) : Date.parse(prior.completeFrom) || from.getTime(),
  );
  const truncated = completeFrom > from.getTime();
  const total = truncated ? Math.max(activities.length, prior.total - slidOut + added) : activities.length;
  return {
    activities,
    coverage: {
      from: from.toISOString(),
      to: to.toISOString(),
      loaded: activities.length,
      total,
      truncated,
      completeFrom: new Date(completeFrom).toISOString(),
    },
    failed: false,
  };
}

interface Periods {
  today: { from: Date; to: Date };
  week: { from: Date; to: Date };
  month: { from: Date; to: Date };
  window: { from: Date; to: Date };
}

function periodsFor(now: Date, timeZone: string, windowFrom: Date): Periods {
  const todayKey = zonedDayKey(now, timeZone);
  return {
    today: { from: startOfZonedDay(todayKey, timeZone), to: now },
    week: { from: new Date(now.getTime() - 7 * DAY_MS), to: now },
    month: { from: startOfZonedDay(`${todayKey.slice(0, 8)}01`, timeZone), to: now },
    window: { from: windowFrom, to: now },
  };
}

/** Exact totals per period from the server; a period whose call fails is null (the rows stand in). */
async function loadTotals(get: LiveGetter, periods: Periods, timeZone: string) {
  const summary = (p: { from: Date; to: Date }) =>
    soft(get(LIVE_ENDPOINTS.summary(p.from, p.to, timeZone))).then((raw) => mapReportSummary(raw));
  const [today, week, month, window] = await Promise.all([
    summary(periods.today),
    summary(periods.week),
    summary(periods.month),
    summary(periods.window),
  ]);
  return {
    today,
    projects: { week: week?.projects ?? null, month: month?.projects ?? null, window: window?.projects ?? null },
  };
}

/** The memberships of a project roster payload (a bare list or an `{ items }` envelope). */
function membershipsOf(raw: unknown): MembershipRole[] {
  const list = Array.isArray(raw) ? raw : raw && typeof raw === "object" ? (raw as { items?: unknown }).items : null;
  return Array.isArray(list) ? (list.filter((m) => m && typeof m === "object") as MembershipRole[]) : [];
}

interface RosterMembership extends MembershipRole {
  projectId?: string | null;
  userName?: string | null;
  name?: string | null;
  surname?: string | null;
  email?: string | null;
}

/**
 * For someone who isn't a manager: read the rosters of their projects to tell
 * whether they only take part as a client. Any roster that fails to load
 * leaves the answer at "no" (a client mistaken for a member sees an empty
 * tracker; the reverse would hide their own work from them).
 */
async function clientRosters(get: LiveGetter, projectIds: string[]): Promise<RosterMembership[] | null> {
  const ids = projectIds.slice(0, MAX_ROLE_CHECKS);
  if (ids.length === 0) return null;
  const rosters = await Promise.all(ids.map((id) => soft(get(LIVE_ENDPOINTS.projectMembers(id)))));
  if (rosters.some((r) => r === null)) return null;
  return rosters.flatMap((raw, i) =>
    membershipsOf(raw).map((m) => ({ ...(m as RosterMembership), projectId: (m as RosterMembership).projectId ?? ids[i] })),
  );
}

function teamRoleOf(projectRole: string | null | undefined): User["role"] {
  const r = (projectRole ?? "").trim().toLowerCase();
  if (r === "client") return "client";
  if (r === "admin" || r === "manager") return "admin";
  return "worker";
}

/* ----------------------------- The load ----------------------------- */

interface MineRow {
  id?: unknown;
  name?: unknown;
  role?: unknown;
}

/**
 * Put the chosen workspace (with the caller's role in it) into the
 * configuration. No workspace chosen: the platform operator gets the console;
 * anyone else must choose one (WorkspaceRequiredError).
 */
async function scopeToWorkspace(get: LiveGetter, cfg: AppConfiguration, workspaceId: string | null | undefined): Promise<AppConfiguration> {
  // A server that scopes the token itself (no X-Workspace-Id) already names the workspace.
  if (workspaceId === undefined || cfg.currentTenant?.id || cfg.currentUser?.tenantId) return cfg;
  if (workspaceId) {
    const raw = await get(LIVE_ENDPOINTS.mine);
    const list = (Array.isArray(raw) ? raw : []) as MineRow[];
    const row = list.find((w) => typeof w?.id === "string" && w.id.toLowerCase() === workspaceId.toLowerCase());
    if (row) {
      return {
        ...cfg,
        currentWorkspace: { id: String(row.id), name: typeof row.name === "string" ? row.name : "Workspace", role: typeof row.role === "string" ? row.role : null },
      };
    }
  }
  if (isPlatformAdminConfig(cfg)) return { ...cfg, currentWorkspace: null };
  throw new WorkspaceRequiredError();
}

/**
 * Load the live session. The identity call is REQUIRED (it rejects when the
 * backend is unreachable and throws SessionExpiredError when the token is no
 * longer accepted); everything else degrades to an honest empty list plus a
 * warning — never to demo data. A pulse (options.previous) keeps the previous
 * roster, projects or plan when re-reading them fails, instead of blanking
 * what is on screen.
 */
export async function loadLiveSession(get: LiveGetter, now: Date = new Date(), options: LiveLoadOptions = {}): Promise<LiveSessionData> {
  const config = await get(LIVE_ENDPOINTS.configuration);
  if (!isAuthenticatedConfig(config)) throw new SessionExpiredError();
  const cfg = await scopeToWorkspace(get, config as AppConfiguration, options.workspaceId);
  const identity = identityOf(cfg);
  const previous = options.previous && sameIdentity(options.previous.identity, identity) ? options.previous : null;
  const browserZone = options.browserZone ?? browserTimeZone();
  const windowFrom = new Date(now.getTime() - LIVE_ACTIVITY_WINDOW_DAYS * DAY_MS);
  const loadedAt = now.toISOString();

  if (isHostConfig(cfg)) {
    // The platform operator has no tenant: host pages load their own data.
    const self = buildLiveUser(cfg);
    return {
      user: self,
      workspace: buildLiveWorkspace(cfg, null, null, now),
      users: [self],
      projects: [],
      activities: [],
      warnings: [],
      coverage: emptyCoverage(windowFrom, now),
      timeZone: previous?.timeZone ?? browserZone,
      identity,
      totals: { projects: {}, today: null },
      plan: { subscription: null, plans: null, billing: null },
      grantedPolicies: grantedPoliciesOf(cfg),
      loadedAt,
    };
  }

  const getPage: LivePageGetter = options.getPage ?? (async (endpoint) => toPagedResult<unknown>(await get(endpoint)));
  const reusePlan = !options.freshPlan && previous && previous.plan.subscription && previous.plan.plans;
  const reuseRows = previous && !previous.warnings.includes("activity");

  const zoneP = previous ? Promise.resolve(previous.timeZone) : reportingZone(get, browserZone);
  // The totals need the zone (where "today" and "this month" begin) but not the rows: start them as soon as it's known.
  const totalsP = zoneP.then((zone) => loadTotals(get, periodsFor(now, zone, windowFrom), zone));
  const [rawMembers, rawProjects, rawPresence, rawSubscription, rawPlans, rawBilling, rows, timeZone, totals] = await Promise.all([
    soft(get(LIVE_ENDPOINTS.members)),
    soft(get(LIVE_ENDPOINTS.projects)),
    soft(get(LIVE_ENDPOINTS.presence)),
    reusePlan ? Promise.resolve(previous.plan.subscription) : soft(get(LIVE_ENDPOINTS.subscription)),
    reusePlan ? Promise.resolve(previous.plan.plans) : soft(get(LIVE_ENDPOINTS.plans)),
    reusePlan && previous.plan.billing ? Promise.resolve(previous.plan.billing) : soft(get(LIVE_ENDPOINTS.billingMode)),
    reuseRows ? pulseRows(getPage, previous, windowFrom, now) : loadAllRows(getPage, windowFrom, now),
    zoneP,
    totalsP,
  ]);

  const warnings: LiveWarning[] = [];
  const membersOk = Array.isArray(rawMembers);
  const projectsOk = Array.isArray(rawProjects);
  if (!projectsOk) warnings.push("projects");
  if (rows.failed) warnings.push("activity");
  if (!membersOk) warnings.push("team");
  const subscription = rawSubscription && typeof rawSubscription === "object" ? (rawSubscription as ApiSubscriptionDto) : null;
  const plans = Array.isArray(rawPlans) ? (rawPlans as ApiPlanDto[]) : null;
  // Billing off: there is no plan to miss. An older server (no such endpoint) bills as before.
  const enabledFlag = rawBilling && typeof rawBilling === "object" ? (rawBilling as { enabled?: unknown }).enabled : undefined;
  const billing = typeof enabledFlag === "boolean" ? { enabled: enabledFlag } : null;
  if ((!subscription || !plans) && billing?.enabled !== false) warnings.push("plan");

  const activities = rows.activities;
  const members = (membersOk ? rawMembers : []).filter(
    (m): m is ApiTeamMemberDto => !!m && typeof m === "object" && typeof (m as ApiTeamMemberDto).userId === "string",
  );
  const projectDtos = (projectsOk ? rawProjects : []).filter(
    (p): p is ApiProjectDto => !!p && typeof p === "object" && typeof (p as ApiProjectDto).id === "string",
  );
  const presence = rawPresence === null ? undefined : presenceStatusByUser(rawPresence);
  const intervals = new Map(projectDtos.map((p) => [p.id, Number(p.intervalMinutes) || 10] as const));
  const context = { timeZone, presence, intervals };

  // Someone without team rights may only take part as a client (contract §2):
  // their rosters say so, and give the client their projects' team by name.
  const selfRow = members.find((m) => m.userId === identity.userId);
  const baseRole = buildLiveUser(cfg).role;
  const rosters =
    baseRole === "worker" && selfRow?.projectIds?.length ? await clientRosters(get, selfRow.projectIds.filter(Boolean)) : null;
  const clientOnly = !!rosters && isClientOnly(rosters, identity.userId);
  const self = buildLiveUser(cfg, { clientOnly });

  // Today's figures: the server's exact totals when it gave them, else the rows.
  const withToday = (u: User): User => {
    const today = totals.today?.users.get(u.id);
    if (!totals.today) return u;
    // today?.activity is already null when nothing was measured (or the user
    // has no row at all today) — never coalesce that to a fabricated 0%.
    return { ...u, trackedToday: Math.round(today?.minutes ?? 0), productivity: today?.activity ?? null };
  };

  // The viewer's role and name come from their identity (authoritative);
  // their tracked figures come off their real rows like everyone else's.
  const selfStats = mapTeamMember(selfRow ?? { userId: identity.userId }, activities, now, context);
  // Presence ("Tracking now") must come from real agent rows or heartbeats, not from being signed in to the web app.
  const user = withToday({
    ...self,
    timezone: timeZone,
    trackedToday: selfStats.trackedToday,
    productivity: selfStats.productivity,
    status: clientOnly ? ("offline" as UserStatus) : selfStats.status,
  });
  const others = members.filter((m) => m.userId !== identity.userId).map((m) => withToday(mapTeamMember(m, activities, now, context)));

  // A client's roster lists only themselves; their projects' teams come from the project rosters.
  const teammates: User[] = [];
  if (clientOnly && rosters) {
    const seen = new Set([identity.userId, ...others.map((u) => u.id)]);
    for (const m of rosters) {
      const id = String(m.userId ?? "");
      if (!id || seen.has(id)) continue;
      seen.add(id);
      const role = teamRoleOf(m.role);
      teammates.push(
        withToday({
          id,
          name: displayName({ name: m.name, surname: m.surname, userName: m.userName, email: m.email }),
          email: m.email ?? "",
          role,
          designation: roleDesignation[role],
          status: presence?.get(id) ?? "offline",
          timezone: timeZone,
          trackedToday: 0,
          productivity: 0,
          joinedAt: "",
        }),
      );
    }
  }
  const users: User[] = [user, ...others, ...teammates];

  const byProject = memberIdsByProject(members);
  if (clientOnly && rosters) {
    for (const m of rosters) {
      const pid = String(m.projectId ?? "");
      const uid = String(m.userId ?? "");
      if (!pid || !uid) continue;
      const list = byProject.get(pid) ?? [];
      if (!list.includes(uid)) list.push(uid);
      byProject.set(pid, list);
    }
  }
  const projects = projectDtos.map((p) => mapApiProject(p, byProject.get(p.id) ?? []));

  // Billing counts a seat per member holding a project membership; the
  // workspace admin always occupies one.
  const seatHolders = members.filter((m) => Array.isArray(m.projectIds) && m.projectIds.length > 0).length;
  const seatsUsed = Math.max(1, members.length ? seatHolders : 1);

  const workspace = buildLiveWorkspace(
    cfg,
    subscription,
    plans,
    now,
    { seatsUsed, projectsUsed: projects.filter((p) => !p.archived).length },
    billing,
  );

  // A pulse that couldn't re-read the roster or projects keeps what it had rather than blanking the screen.
  const keepUsers = !membersOk && previous ? previous.users : users;
  const keepProjects = !projectsOk && previous ? previous.projects : projects;

  return {
    user,
    workspace,
    users: keepUsers,
    projects: keepProjects,
    activities,
    warnings,
    coverage: rows.coverage,
    timeZone,
    identity,
    totals,
    plan: { subscription, plans, billing },
    grantedPolicies: grantedPoliciesOf(cfg),
    loadedAt,
  };
}

/**
 * Who the stored token belongs to, or null when the backend no longer accepts
 * it. Rejects when the backend can't be reached. Cheap: one configuration call.
 */
export async function fetchLiveIdentity(get: LiveGetter, workspaceId?: string | null): Promise<LiveIdentity | null> {
  const config = await get(LIVE_ENDPOINTS.configuration);
  if (!isAuthenticatedConfig(config)) return null;
  const identity = identityOf(config as AppConfiguration);
  return workspaceId ? { ...identity, tenantId: workspaceId } : identity;
}

/** Whether two identities are the same account in the same workspace. */
export function isSameIdentity(a: LiveIdentity, b: LiveIdentity): boolean {
  return sameIdentity(a, b);
}

const WARNING_LABELS: Record<LiveWarning, string> = {
  projects: "projects",
  activity: "activity",
  team: "team",
  plan: "plan",
};

/** One sentence for the "some data didn't load" banner, or null when everything loaded. */
export function describeWarnings(warnings: readonly string[]): string | null {
  if (warnings.length === 0) return null;
  const names = warnings.map((w) => WARNING_LABELS[w as LiveWarning] ?? w);
  const list = names.length === 1 ? names[0] : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
  return `Couldn't load your workspace's ${list} data. What's shown may be incomplete.`;
}

/** "Showing the newest 5,000 of 12,345 activity blocks" for a cut list, or null when the list is complete. */
export function describeCoverage(coverage: ActivityCoverage | null | undefined): string | null {
  if (!coverage || !coverage.truncated) return null;
  const fmt = (n: number) => n.toLocaleString("en");
  return `Showing the newest ${fmt(coverage.loaded)} of ${fmt(coverage.total)} activity blocks from the last ${LIVE_ACTIVITY_WINDOW_DAYS} days. Totals are complete; lists and charts built from blocks may be partial.`;
}

/** Whether the held rows cover every block since `from`, so a page may total that range from rows. */
export function coversSince(coverage: ActivityCoverage | null | undefined, from: Date): boolean {
  if (!coverage) return true;
  return !coverage.truncated || Date.parse(coverage.completeFrom) <= from.getTime();
}
