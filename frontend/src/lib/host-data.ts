import type { User } from "./types";
import {
  invoicesFor,
  monthlyCost,
  planById,
  plans,
  type Invoice,
  type Plan,
  type Workspace,
} from "./saas-data";
import { datasetFor, NOW } from "./tenant-data";

/* ============================================================================
 * Host / platform-level aggregation across every tenant (workspace).
 * These helpers read each tenant's isolated dataset + its subscription and
 * roll everything up into the numbers a SaaS operator cares about.
 * ========================================================================== */

const DAY = 24 * 60 * 60 * 1000;

export interface TenantMetrics {
  members: number;
  activeMembers: number;
  clients: number;
  projects: number;
  activities: number;
  avgProductivity: number;
  trackedToday: number; // minutes across the tenant
  mrr: number | null; // null = custom (enterprise)
  seats: number;
  storageGb: number;
}

export function tenantMetrics(ws: Workspace): TenantMetrics {
  const ds = datasetFor(ws.id);
  const members = ds.users.filter((u) => u.role !== "client");
  const activeUsers = members.filter((u) => u.productivity > 0);
  const avgProductivity = activeUsers.length
    ? Math.round(activeUsers.reduce((s, u) => s + u.productivity, 0) / activeUsers.length)
    : 0;
  return {
    members: members.length,
    activeMembers: members.filter((u) => u.status === "active").length,
    clients: ds.users.filter((u) => u.role === "client").length,
    projects: ds.projects.filter((p) => !p.archived).length,
    activities: ds.activities.length,
    avgProductivity,
    trackedToday: members.reduce((s, u) => s + u.trackedToday, 0),
    mrr: monthlyCost(ws),
    seats: ws.seatsUsed,
    storageGb: ws.storageUsedGb,
  };
}

export interface PlatformOverview {
  totalTenants: number;
  active: number;
  trialing: number;
  pastDue: number;
  newThisMonth: number;
  totalSeats: number;
  totalMembers: number;
  totalProjects: number;
  mrr: number;
  arr: number;
  storageGb: number;
  avgProductivity: number;
  enterpriseTenants: number;
}

export function platformOverview(workspaces: Workspace[]): PlatformOverview {
  let totalSeats = 0;
  let totalMembers = 0;
  let totalProjects = 0;
  let mrr = 0;
  let storageGb = 0;
  let prodSum = 0;
  let prodWeight = 0;
  let enterpriseTenants = 0;

  for (const ws of workspaces) {
    const m = tenantMetrics(ws);
    totalSeats += m.seats;
    totalMembers += m.members;
    totalProjects += m.projects;
    storageGb += m.storageGb;
    if (m.mrr === null) enterpriseTenants++;
    else mrr += m.mrr;
    if (m.members > 0) {
      prodSum += m.avgProductivity * m.members;
      prodWeight += m.members;
    }
  }

  return {
    totalTenants: workspaces.length,
    active: workspaces.filter((w) => w.status === "active").length,
    trialing: workspaces.filter((w) => w.status === "trialing").length,
    pastDue: workspaces.filter((w) => w.status === "past_due").length,
    newThisMonth: workspaces.filter((w) => NOW.getTime() - new Date(w.createdAt).getTime() < 31 * DAY).length,
    totalSeats,
    totalMembers,
    totalProjects,
    mrr,
    arr: mrr * 12,
    storageGb: Math.round(storageGb * 10) / 10,
    avgProductivity: prodWeight ? Math.round(prodSum / prodWeight) : 0,
    enterpriseTenants,
  };
}

export interface PlanBreakdown {
  plan: Plan;
  tenants: number;
  seats: number;
  mrr: number;
  color: string;
}

const PLAN_COLORS: Record<string, string> = {
  free: "#94a3b8",
  starter: "#0ea5e9",
  business: "#6d5efc",
  enterprise: "#ec4899",
};

export function planColor(planId: string): string {
  return PLAN_COLORS[planId] ?? "#94a3b8";
}

export function planBreakdown(workspaces: Workspace[]): PlanBreakdown[] {
  return plans.map((plan) => {
    const tenants = workspaces.filter((w) => w.planId === plan.id);
    return {
      plan,
      tenants: tenants.length,
      seats: tenants.reduce((s, w) => s + w.seatsUsed, 0),
      mrr: tenants.reduce((s, w) => s + (monthlyCost(w) ?? 0), 0),
      color: planColor(plan.id),
    };
  });
}

/* ----------------------------- Trends (deterministic) ----------------------------- */

function mulberry32(seed: number) {
  return function () {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

export interface GrowthPoint {
  month: string;
  mrr: number;
  tenants: number;
}

/** 12-month MRR + tenant growth curve ending at the current platform totals. */
export function growthTrend(workspaces: Workspace[]): GrowthPoint[] {
  const rand = mulberry32(20260714);
  const { mrr, totalTenants } = platformOverview(workspaces);
  const points: GrowthPoint[] = [];
  for (let i = 11; i >= 0; i--) {
    const decay = Math.pow(0.82, i) * (0.9 + rand() * 0.2);
    const d = new Date(NOW.getTime() - i * 30 * DAY);
    points.push({
      month: MONTHS[d.getMonth()],
      mrr: Math.max(0, Math.round((mrr * decay) / 10) * 10),
      tenants: Math.max(1, Math.round(totalTenants * Math.pow(0.86, i) * (0.92 + rand() * 0.16))),
    });
  }
  // Anchor the final point to the real current totals.
  points[points.length - 1] = { month: points[points.length - 1].month, mrr, tenants: totalTenants };
  return points;
}

/* ----------------------------- Global users ----------------------------- */

export interface GlobalUser {
  user: User;
  workspaceId: string;
  workspaceName: string;
  workspaceColor: string;
}

export function globalUsers(workspaces: Workspace[]): GlobalUser[] {
  const out: GlobalUser[] = [];
  for (const ws of workspaces) {
    for (const user of datasetFor(ws.id).users) {
      out.push({ user, workspaceId: ws.id, workspaceName: ws.name, workspaceColor: ws.color });
    }
  }
  return out;
}

/* ----------------------------- Platform billing ----------------------------- */

export interface PlatformInvoice extends Invoice {
  workspaceId: string;
  workspaceName: string;
  workspaceColor: string;
}

export function platformInvoices(workspaces: Workspace[]): PlatformInvoice[] {
  const out: PlatformInvoice[] = [];
  for (const ws of workspaces) {
    for (const inv of invoicesFor(ws)) {
      out.push({ ...inv, workspaceId: ws.id, workspaceName: ws.name, workspaceColor: ws.color });
    }
  }
  return out.sort((a, b) => +new Date(b.date) - +new Date(a.date));
}

export function ownerOf(ws: Workspace): User | undefined {
  const ds = datasetFor(ws.id);
  return ds.users.find((u) => u.role === "owner") ?? ds.users[0];
}

export { planById };
