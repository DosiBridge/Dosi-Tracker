import type { Role, User } from "./types";
import type { PlanId, SubscriptionStatus, Workspace } from "./saas-data";
import { displayName, roleDesignation } from "./live-dataset";

/* ============================================================================
 * LIVE IDENTITY — who is signed in, and into which workspace, built ONLY from
 * the real backend (ABP application-configuration + the tenant subscription).
 * Pure functions; the session provider does the fetching.
 * ========================================================================== */

/** The slice of GET /api/abp/application-configuration the app reads. */
export interface AppConfiguration {
  currentUser?: {
    isAuthenticated?: boolean;
    id?: string | null;
    tenantId?: string | null;
    userName?: string | null;
    name?: string | null;
    surName?: string | null;
    email?: string | null;
    roles?: string[] | null;
  } | null;
  currentTenant?: {
    id?: string | null;
    name?: string | null;
    isAvailable?: boolean;
  } | null;
  auth?: {
    grantedPolicies?: Record<string, boolean> | null;
  } | null;
}

/** GET /api/app/workspace/current-subscription. */
export interface ApiSubscriptionDto {
  planId?: string | null;
  status?: string | null;
  trialEndsAt?: string | null;
}

/** GET /api/app/workspace/available-plans item. */
export interface ApiPlanDto {
  id: string;
  name?: string | null;
  pricePerUser?: number | null;
  maxSeats?: number | null;
  trialDays?: number | null;
}

export const PERMISSION = {
  viewAllActivities: "Tracker.Activities.ViewAll",
  manageTeam: "Tracker.Team.Manage",
  platform: "Tracker.Platform",
} as const;

function granted(config: AppConfiguration, policy: string): boolean {
  return config.auth?.grantedPolicies?.[policy] === true;
}

function hasRole(config: AppConfiguration, role: string): boolean {
  return (config.currentUser?.roles ?? []).some((r) => typeof r === "string" && r.toLowerCase() === role);
}

/** Signed in with a token the backend actually accepted. */
export function isAuthenticatedConfig(config: unknown): config is AppConfiguration {
  const c = config as AppConfiguration | null;
  return !!c && typeof c === "object" && c.currentUser?.isAuthenticated === true && !!c.currentUser?.id;
}

/** A host (platform-side) session: no tenant in scope. */
export function isHostConfig(config: AppConfiguration): boolean {
  return !config.currentUser?.tenantId && !config.currentTenant?.id;
}

/**
 * Map the real ABP identity onto the app's role model:
 *  - host side (no tenant) with Tracker.Platform or the host "admin" role →
 *    "host" (the platform operator);
 *  - tenant "admin" role → "owner" (the workspace's owner/admin account);
 *  - team managers / people who may see everyone's activity → "admin";
 *  - everyone else → "worker" (sees only their own data).
 */
export function resolveLiveRole(config: AppConfiguration): Role {
  if (isHostConfig(config)) {
    // Host side: the platform operator (Tracker.Platform, or the host admin).
    // Any other host-side account has no tenant and no platform rights.
    return granted(config, PERMISSION.platform) || hasRole(config, "admin") ? "host" : "worker";
  }
  if (hasRole(config, "admin")) return "owner";
  if (granted(config, PERMISSION.manageTeam) || granted(config, PERMISSION.viewAllActivities)) return "admin";
  return "worker";
}

/** The signed-in user, from the backend identity (never a demo person). */
export function buildLiveUser(config: AppConfiguration): User {
  const cu = config.currentUser ?? {};
  const role = resolveLiveRole(config);
  return {
    id: String(cu.id ?? "me"),
    name: displayName({ name: cu.name, surName: cu.surName, userName: cu.userName, email: cu.email }, "You"),
    email: cu.email ?? "",
    role,
    designation: roleDesignation[role],
    // Presence is derived from agent activity by the loader; identity alone says nothing.
    status: "offline",
    timezone: "UTC",
    trackedToday: 0,
    productivity: 0,
    joinedAt: "",
  };
}

/** Backend plan name → the app's plan id (unknown/custom plans read as enterprise-style custom pricing). */
export function planIdFromName(name: string | null | undefined): PlanId {
  const n = (name ?? "").trim().toLowerCase();
  if (n === "free" || n === "starter" || n === "business" || n === "enterprise") return n;
  return n ? "enterprise" : "free";
}

export function subscriptionStatus(raw: string | null | undefined): SubscriptionStatus {
  const s = (raw ?? "").toLowerCase().replace(/[\s-]/g, "_");
  if (s === "trialing" || s === "trial") return "trialing";
  if (s === "past_due" || s === "pastdue") return "past_due";
  return "active";
}

/** Whole days from `now` until an ISO instant (never negative); 0 when absent/invalid. */
export function daysUntil(iso: string | null | undefined, now: Date): number {
  const at = iso ? Date.parse(/([zZ]|[+-]\d{2}:?\d{2})$/.test(iso) ? iso : `${iso}Z`) : NaN;
  if (!Number.isFinite(at)) return 0;
  return Math.max(0, Math.ceil((at - now.getTime()) / 86_400_000));
}

/** Prefix of every live workspace id — never collides with the demo ids. */
export const LIVE_WORKSPACE_PREFIX = "live-";

/** Placeholder workspace id while the live identity is loading (or failed). */
export const LIVE_PENDING_WORKSPACE_ID = `${LIVE_WORKSPACE_PREFIX}pending`;

/** Stable id for the live workspace. */
export function liveWorkspaceId(config: AppConfiguration): string {
  return config.currentTenant?.id ? `${LIVE_WORKSPACE_PREFIX}tenant-${config.currentTenant.id}` : `${LIVE_WORKSPACE_PREFIX}platform`;
}

/**
 * The real workspace: the tenant's own name and its real subscription. Seat and
 * price limits come from the backend plan when it is known, so the UI never
 * quotes the demo plan table to a paying tenant.
 */
export function buildLiveWorkspace(
  config: AppConfiguration,
  subscription: ApiSubscriptionDto | null,
  plans: ApiPlanDto[] | null,
  now: Date,
  counts: { seatsUsed: number; projectsUsed: number } = { seatsUsed: 0, projectsUsed: 0 },
): Workspace {
  const name = (config.currentTenant?.name ?? "").trim() || (isHostConfig(config) ? "Platform" : "Workspace");
  const plan = subscription?.planId ? (plans ?? []).find((p) => p.id === subscription.planId) ?? null : null;
  const status = subscriptionStatus(subscription?.status);
  const maxSeats = typeof plan?.maxSeats === "number" ? plan.maxSeats : null;
  return {
    id: liveWorkspaceId(config),
    name,
    slug: name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/(^-|-$)/g, "") || "workspace",
    planId: planIdFromName(plan?.name),
    status,
    color: "#0d9488",
    createdAt: "",
    cycleDays: status === "trialing" ? daysUntil(subscription?.trialEndsAt, now) : 0,
    seatsUsed: counts.seatsUsed,
    projectsUsed: counts.projectsUsed,
    storageUsedGb: 0,
    live: {
      known: !!plan,
      planName: plan?.name ?? null,
      pricePerUser: typeof plan?.pricePerUser === "number" ? plan.pricePerUser : null,
      seatLimit: maxSeats === null ? null : maxSeats <= 0 ? Number.POSITIVE_INFINITY : maxSeats,
      trialEndsAt: subscription?.trialEndsAt ?? null,
    },
  };
}
