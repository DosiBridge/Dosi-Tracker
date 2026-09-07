import { NOW, projects, users } from "./mock-data";
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

export type SubscriptionStatus = "active" | "trialing" | "past_due";

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
}

const activeMembers = countSeats(users);
const activeProjects = projects.filter((p) => !p.archived).length;

export const workspaces: Workspace[] = [
  {
    id: "w1",
    name: "Dosi Labs",
    slug: "dosi-labs",
    planId: "business",
    status: "active",
    color: "#6d5efc",
    createdAt: "2025-04-02",
    cycleDays: 18,
    seatsUsed: activeMembers,
    projectsUsed: activeProjects,
    storageUsedGb: 128,
    isPrimary: true,
  },
  {
    id: "w2",
    name: "Acme Studio",
    slug: "acme-studio",
    planId: "starter",
    status: "trialing",
    color: "#ec4899",
    createdAt: "2026-06-28",
    cycleDays: 9,
    seatsUsed: 4,
    projectsUsed: 3,
    storageUsedGb: 6,
  },
  {
    id: "w3",
    name: "Nimbus Co",
    slug: "nimbus",
    planId: "free",
    status: "active",
    color: "#0ea5e9",
    createdAt: "2026-05-15",
    cycleDays: 0,
    seatsUsed: 3,
    projectsUsed: 2,
    storageUsedGb: 0.6,
  },
];

export const primaryWorkspace = workspaces.find((w) => w.isPrimary) ?? workspaces[0];

/* ----------------------------- Usage ----------------------------- */

export interface UsageMetric {
  label: string;
  used: number;
  limit: number; // Infinity = unlimited
  unit: string;
}

/** Live seat count from the tenant dataset (single source of truth). */
export function liveSeatsUsed(ws: Workspace): number {
  return countSeats(datasetFor(ws.id).users);
}

export function liveProjectsUsed(ws: Workspace): number {
  return datasetFor(ws.id).projects.filter((p) => !p.archived).length;
}

export function workspaceUsage(ws: Workspace): UsageMetric[] {
  const plan = planById(ws.planId);
  return [
    { label: "Seats", used: liveSeatsUsed(ws), limit: plan.seats, unit: "members" },
    { label: "Projects", used: liveProjectsUsed(ws), limit: plan.projects, unit: "projects" },
    { label: "Storage", used: ws.storageUsedGb, limit: plan.storageGb, unit: "GB" },
    { label: "Screenshot history", used: plan.retentionDays, limit: plan.retentionDays, unit: "days" },
  ];
}

export function monthlyCost(ws: Workspace): number | null {
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
};
