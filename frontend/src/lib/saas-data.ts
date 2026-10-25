import { NOW } from "./demo-clock";
import { countSeats } from "./roles";
import { datasetFor } from "./tenant-data";

/* ----------------------------- Plans ----------------------------- */

export type PlanId = "free" | "starter" | "business" | "enterprise";

export interface Plan {
  id: PlanId;
  name: string;
  /** USD per user / month. null = custom pricing */
  pricePerUser: number | null;
  tagline: string;
  seats: number; // Infinity for unlimited
  projects: number; // Infinity for unlimited
  retentionDays: number;
  storageGb: number; // Infinity for unlimited
  features: string[];
  highlight?: boolean;
}

const INF = Number.POSITIVE_INFINITY;

export const plans: Plan[] = [
  {
    id: "free",
    name: "Free",
    pricePerUser: 0,
    tagline: "For trying things out",
    seats: 3,
    projects: 2,
    retentionDays: 7,
    storageGb: 1,
    features: ["Time & activity tracking", "Screenshots (7-day history)", "1 basic report", "Community support"],
  },
  {
    id: "starter",
    name: "Starter",
    pricePerUser: 6,
    tagline: "For small teams",
    seats: 10,
    projects: 10,
    retentionDays: 30,
    storageGb: 20,
    features: ["Everything in Free", "Unlimited screenshots", "Timesheets & approvals", "All standard reports", "Email support"],
  },
  {
    id: "business",
    name: "Business",
    pricePerUser: 12,
    tagline: "For growing companies",
    seats: 50,
    projects: INF,
    retentionDays: 180,
    storageGb: 500,
    features: ["Everything in Starter", "Member monitor & insights", "Payroll & billing reports", "Scheduled reports & exports", "Priority support"],
    highlight: true,
  },
  {
    id: "enterprise",
    name: "Enterprise",
    pricePerUser: null,
    tagline: "For organizations at scale",
    seats: INF,
    projects: INF,
    retentionDays: 365,
    storageGb: INF,
    features: ["Everything in Business", "SSO / SAML & SCIM", "Audit logs & data residency", "Dedicated success manager", "99.9% uptime SLA"],
  },
];

export const planById = (id: PlanId) => plans.find((p) => p.id === id)!;

/* ----------------------------- Workspaces (tenants) ----------------------------- */

/**
 * A workspace subscription's state, as the backend stores it. Suspended,
 * canceled and expired workspaces can't record time or add people (the API
 * answers 402 Tracker:SubscriptionInactive); a past-due one only until its
 * grace period ends.
 */
export type SubscriptionStatus = "active" | "trialing" | "past_due" | "suspended" | "canceled" | "expired";

/** Statuses under which the workspace is (or is about to be) cut off: owners are told what to do. */
export function subscriptionNeedsAttention(status: SubscriptionStatus): boolean {
  return status === "past_due" || status === "suspended" || status === "canceled" || status === "expired";
}

export interface Workspace {
  id: string;
  name: string;
  slug: string; // used as tenant subdomain: {slug}.dositracker.app
  planId: PlanId;
  status: SubscriptionStatus;
  color: string;
  createdAt: string;
  /** days until renewal (active) or trial end (trialing) */
  cycleDays: number;
  seatsUsed: number;
  projectsUsed: number;
  storageUsedGb: number;
  isPrimary?: boolean;
  /**
   * Present only for a LIVE (backend) workspace: the real plan as the backend
   * defines it. When set, it wins over the demo plan table below — a paying
   * tenant must be quoted its real seat limit and price, not the demo's.
   */
  live?: LivePlanInfo;
}

export interface LivePlanInfo {
  /** The subscription's plan was found in the backend plan list. */
  known: boolean;
  planName: string | null;
  pricePerUser: number | null;
  /** Infinity = unlimited; null = unknown. */
  seatLimit: number | null;
  trialEndsAt: string | null;
}

/* ----------------------------- Demo workspaces ----------------------------- */
// The demo tenants (lib/demo-tenants) register their workspaces here when a
// deployment with demo mode on loads them. A live deployment never does, so it
// has none: there is no demo workspace to fall back to.

let demoSeed: readonly Workspace[] = [];

/** Make `list` the demo's workspaces (lib/demo-tenants, when demo mode loads it). */
export function registerDemoWorkspaces(list: readonly Workspace[]): void {
  demoSeed = list;
}

/** The demo's seeded workspaces; empty unless demo mode loaded them. */
export function demoWorkspaces(): readonly Workspace[] {
  return demoSeed;
}

/** The demo workspace a new demo visitor opens, or null outside demo mode. */
export function primaryDemoWorkspace(): Workspace | null {
  return demoSeed.find((w) => w.isPrimary) ?? demoSeed[0] ?? null;
}

/** Shown for a live workspace whose plan couldn't be loaded. */
export const UNKNOWN_PLAN_NAME = "Unknown plan";

/**
 * Seat ceiling for a workspace: the backend's plan for a live tenant, else the
 * plan table. A live tenant whose plan is NOT known gets no ceiling in the
 * browser — the backend enforces seats itself — rather than the demo table's
 * guess (which blocked invites with "limit reached" on paid workspaces).
 */
export function seatLimitFor(ws: Workspace): number {
  if (ws.live) return ws.live.known ? (ws.live.seatLimit ?? INF) : INF;
  return planById(ws.planId).seats;
}

/** Display name of a workspace's plan (the backend's own name for a live tenant; "Unknown plan" when it couldn't be loaded). */
export function planNameFor(ws: Workspace): string {
  if (ws.live) return ws.live.known ? (ws.live.planName ?? UNKNOWN_PLAN_NAME) : UNKNOWN_PLAN_NAME;
  return planById(ws.planId).name;
}

/* ----------------------------- Usage ----------------------------- */

export interface UsageMetric {
  label: string;
  used: number;
  limit: number; // Infinity = unlimited
  unit: string;
}

/**
 * Seats in use. A live tenant reports the backend's own count (members holding
 * a project membership, as billing counts them); a demo workspace counts its
 * dataset roster (single source of truth).
 */
export function liveSeatsUsed(ws: Workspace): number {
  if (ws.live) return ws.seatsUsed;
  return countSeats(datasetFor(ws.id).users);
}

export function liveProjectsUsed(ws: Workspace): number {
  return datasetFor(ws.id).projects.filter((p) => !p.archived).length;
}

export function workspaceUsage(ws: Workspace): UsageMetric[] {
  const plan = planById(ws.planId);
  if (ws.live) {
    // Only what the backend actually meters: seats against the real plan limit
    // and the projects that exist. Storage/retention are not metered yet, so
    // they are not shown rather than invented.
    return [
      { label: "Seats", used: liveSeatsUsed(ws), limit: seatLimitFor(ws), unit: "members" },
      { label: "Projects", used: liveProjectsUsed(ws), limit: Number.POSITIVE_INFINITY, unit: "projects" },
    ];
  }
  return [
    { label: "Seats", used: liveSeatsUsed(ws), limit: plan.seats, unit: "members" },
    { label: "Projects", used: liveProjectsUsed(ws), limit: plan.projects, unit: "projects" },
    { label: "Storage", used: ws.storageUsedGb, limit: plan.storageGb, unit: "GB" },
    { label: "Screenshot history", used: plan.retentionDays, limit: plan.retentionDays, unit: "days" },
  ];
}

export function monthlyCost(ws: Workspace): number | null {
  if (ws.live) {
    // Unknown plan → no price rather than the demo table's guess.
    if (ws.live.pricePerUser === null) return null;
    return ws.live.pricePerUser * Math.max(1, liveSeatsUsed(ws));
  }
  const plan = planById(ws.planId);
  if (plan.pricePerUser === null) return null;
  return plan.pricePerUser * liveSeatsUsed(ws);
}

/* ----------------------------- Invoices ----------------------------- */

export interface Invoice {
  id: string;
  date: string; // ISO date
  amount: number;
  status: "paid" | "due" | "upcoming";
  plan: string;
}

const MONTH = 30 * 24 * 60 * 60 * 1000;

export function invoicesFor(ws: Workspace): Invoice[] {
  const plan = planById(ws.planId);
  const amount = monthlyCost(ws) ?? 0;
  if (ws.status === "trialing") {
    return [
      {
        id: `${ws.id}-inv-upcoming`,
        date: new Date(NOW.getTime() + ws.cycleDays * 24 * 60 * 60 * 1000).toISOString().slice(0, 10),
        amount,
        status: "upcoming",
        plan: plan.name,
      },
    ];
  }
  const out: Invoice[] = [];
  for (let i = 0; i < 5; i++) {
    const date = new Date(NOW.getTime() - i * MONTH);
    out.push({
      id: `${ws.id}-inv-${i}`,
      date: date.toISOString().slice(0, 10),
      amount,
      status: "paid",
      plan: plan.name,
    });
  }
  return out;
}

/* ----------------------------- Helpers ----------------------------- */

export function fmtLimit(n: number): string {
  return n === INF ? "Unlimited" : n.toLocaleString();
}

export function usagePct(used: number, limit: number): number {
  if (limit === INF) return 0;
  if (limit <= 0) return used > 0 ? 100 : 0; // guard 0/0 → NaN and n/0 → Infinity
  return Math.min(100, Math.round((used / limit) * 100));
}

export const statusLabel: Record<SubscriptionStatus, string> = {
  active: "Active",
  trialing: "Trial",
  past_due: "Past due",
  suspended: "Suspended",
  canceled: "Canceled",
  expired: "Expired",
};
