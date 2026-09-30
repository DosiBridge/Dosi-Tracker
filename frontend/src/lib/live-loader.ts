import type { Activity, Project, User } from "./types";
import type { Workspace } from "./saas-data";
import {
  mapApiActivities,
  mapApiProject,
  mapTeamMember,
  memberIdsByProject,
  type ApiProjectDto,
  type ApiTeamMemberDto,
} from "./live-dataset";
import {
  buildLiveUser,
  buildLiveWorkspace,
  isAuthenticatedConfig,
  isHostConfig,
  type ApiPlanDto,
  type ApiSubscriptionDto,
  type AppConfiguration,
} from "./live-identity";

/* ============================================================================
 * LIVE SESSION LOADER — one round-trip set that turns a bearer token into the
 * signed-in user, their real workspace and the tenant's real rows.
 *
 * Kept in the pure lib layer (the HTTP getter is injected) so the orchestration
 * — which calls are required, which may fail softly, how the roster, projects
 * and seats are stitched together — is unit-tested without React or a network.
 * ========================================================================== */

/** GET helper with getApi semantics: resolves the JSON (items unwrapped), rejects on failure. */
export type LiveGetter = (endpoint: string) => Promise<unknown>;

/** The token is present but the backend no longer recognises it as a signed-in user. */
export class SessionExpiredError extends Error {
  constructor() {
    super("Your session has expired. Please sign in again.");
    this.name = "SessionExpiredError";
  }
}

export interface LiveSessionData {
  user: User;
  workspace: Workspace;
  users: User[];
  projects: Project[];
  activities: Activity[];
  /** Human-readable notes about data that could not be loaded (the rest still renders). */
  warnings: string[];
}

/** How far back the session preloads activity rows for dashboards and pickers. */
export const LIVE_ACTIVITY_WINDOW_DAYS = 30;
const DAY_MS = 24 * 60 * 60 * 1000;

export const LIVE_ENDPOINTS = {
  configuration: "/api/abp/application-configuration?IncludeLocalizationResources=false",
  members: "/api/app/team/members",
  projects: "/api/app/project?MaxResultCount=1000",
  subscription: "/api/app/workspace/current-subscription",
  plans: "/api/app/workspace/available-plans",
  activities: (from: Date, to: Date) =>
    `/api/app/activity?From=${encodeURIComponent(from.toISOString())}&To=${encodeURIComponent(to.toISOString())}&MaxResultCount=1000`,
} as const;

const soft = (p: Promise<unknown>): Promise<unknown | null> => p.catch(() => null);

/**
 * Load the live session. The identity call is REQUIRED (it rejects when the
 * backend is unreachable and throws SessionExpiredError when the token is no
 * longer accepted); everything else degrades to an honest empty list plus a
 * warning — never to demo data.
 */
export async function loadLiveSession(get: LiveGetter, now: Date = new Date()): Promise<LiveSessionData> {
  const config = await get(LIVE_ENDPOINTS.configuration);
  if (!isAuthenticatedConfig(config)) throw new SessionExpiredError();
  const cfg = config as AppConfiguration;
  const self = buildLiveUser(cfg);

  if (isHostConfig(cfg)) {
    // The platform operator has no tenant: host pages load their own data.
    const workspace = buildLiveWorkspace(cfg, null, null, now);
    return { user: self, workspace, users: [self], projects: [], activities: [], warnings: [] };
  }

  const from = new Date(now.getTime() - LIVE_ACTIVITY_WINDOW_DAYS * DAY_MS);
  const [rawMembers, rawProjects, rawActivities, rawSubscription, rawPlans] = await Promise.all([
    soft(get(LIVE_ENDPOINTS.members)),
    soft(get(LIVE_ENDPOINTS.projects)),
    soft(get(LIVE_ENDPOINTS.activities(from, now))),
    soft(get(LIVE_ENDPOINTS.subscription)),
    soft(get(LIVE_ENDPOINTS.plans)),
  ]);

  const warnings: string[] = [];
  if (!Array.isArray(rawProjects)) warnings.push("projects");
  if (!Array.isArray(rawActivities)) warnings.push("activity");
  if (!Array.isArray(rawMembers)) warnings.push("team");

  const activities = mapApiActivities(rawActivities);
  const members = (Array.isArray(rawMembers) ? rawMembers : []).filter(
    (m): m is ApiTeamMemberDto => !!m && typeof m === "object" && typeof (m as ApiTeamMemberDto).userId === "string",
  );

  // The viewer's own role and name come from their identity (authoritative);
  // their tracked figures come off their real rows like everyone else's.
  const selfRow = members.find((m) => m.userId === self.id);
  const selfStats = mapTeamMember(selfRow ?? { userId: self.id }, activities, now);
  // Presence ("Tracking now") must come from real agent rows, not from being signed in to the web app.
  const user: User = { ...self, trackedToday: selfStats.trackedToday, productivity: selfStats.productivity, status: selfStats.status };
  const users: User[] = [user, ...members.filter((m) => m.userId !== self.id).map((m) => mapTeamMember(m, activities, now))];

  const byProject = memberIdsByProject(members);
  const projects = (Array.isArray(rawProjects) ? rawProjects : [])
    .filter((p): p is ApiProjectDto => !!p && typeof p === "object" && typeof (p as ApiProjectDto).id === "string")
    .map((p) => mapApiProject(p, byProject.get(p.id) ?? []));

  // Billing counts a seat per member holding a project membership; the
  // workspace admin always occupies one.
  const seatHolders = members.filter((m) => Array.isArray(m.projectIds) && m.projectIds.length > 0).length;
  const seatsUsed = Math.max(1, members.length ? seatHolders : 1);

  const workspace = buildLiveWorkspace(
    cfg,
    rawSubscription && typeof rawSubscription === "object" ? (rawSubscription as ApiSubscriptionDto) : null,
    Array.isArray(rawPlans) ? (rawPlans as ApiPlanDto[]) : null,
    now,
    { seatsUsed, projectsUsed: projects.filter((p) => !p.archived).length },
  );

  return { user, workspace, users, projects, activities, warnings };
}

/** One sentence for the "some data didn't load" banner, or null when everything loaded. */
export function describeWarnings(warnings: string[]): string | null {
  if (warnings.length === 0) return null;
  const list = warnings.length === 1 ? warnings[0] : `${warnings.slice(0, -1).join(", ")} and ${warnings[warnings.length - 1]}`;
  return `Couldn't load your workspace's ${list} data. What's shown may be incomplete.`;
}
