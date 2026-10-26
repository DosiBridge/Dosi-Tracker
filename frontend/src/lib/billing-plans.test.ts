// Owner self-service billing (E2E-08) and the deployment's billing mode: the
// plan rules, the workspace's limits while billing is off, and how the live
// session reads (and re-reads) the billing mode.
import { describe, expect, it, vi } from "vitest";
import { billingErrorText, canCancelSubscription, fittingFreePlan, isFreePlan, planAction, teamFits } from "./billing-plans";
import { billingEnabledFor, seatLimitFor, type Workspace } from "./saas-data";
import { loadLiveSession, type LiveGetter } from "./live-loader";

const FREE = { id: "f", name: "Free", pricePerUser: 0, maxSeats: 3 };
const SOLO = { id: "s1", name: "Solo", pricePerUser: 0, maxSeats: 1 };
const PAID = { id: "p", name: "Pro", pricePerUser: 8, maxSeats: 0 };

describe("plan rules", () => {
  it("tells free from paid, and whether a team fits", () => {
    expect(isFreePlan(FREE)).toBe(true);
    expect(isFreePlan({ id: "x", pricePerUser: null })).toBe(true);
    expect(isFreePlan(PAID)).toBe(false);
    expect(teamFits(FREE, 3)).toBe(true);
    expect(teamFits(FREE, 4)).toBe(false);
    expect(teamFits(PAID, 999)).toBe(true);
  });

  it("decides each card's action: current, switch, too small, or ask the platform", () => {
    expect(planAction(FREE, { currentPlanId: "f", seatsUsed: 2 })).toEqual({ kind: "current" });
    expect(planAction(FREE, { currentPlanId: "p", seatsUsed: 2 })).toEqual({ kind: "switch" });
    expect(planAction(SOLO, { currentPlanId: "p", seatsUsed: 2 })).toEqual({ kind: "too-small", maxSeats: 1 });
    expect(planAction(PAID, { currentPlanId: "f", seatsUsed: 2 })).toEqual({ kind: "request" });
    expect(planAction(PAID, { currentPlanId: null, seatsUsed: 2 })).toEqual({ kind: "request" });
  });

  it("finds the cheapest free plan the team fits, never the current one", () => {
    expect(fittingFreePlan([SOLO, FREE, PAID], 2, "p")).toBe(FREE);
    expect(fittingFreePlan([SOLO, FREE, PAID], 2, "f")).toBeNull();
    expect(fittingFreePlan([SOLO], 2, null)).toBeNull();
  });

  it("lets an owner cancel only a paid plan that is still running", () => {
    expect(canCancelSubscription({ pricePerUser: 6, status: "active" })).toBe(true);
    expect(canCancelSubscription({ pricePerUser: 6, status: "trialing" })).toBe(true);
    expect(canCancelSubscription({ pricePerUser: 6, status: "canceled" })).toBe(false);
    expect(canCancelSubscription({ pricePerUser: 6, status: "expired" })).toBe(false);
    expect(canCancelSubscription({ pricePerUser: 0, status: "active" })).toBe(false);
    expect(canCancelSubscription({ pricePerUser: null, status: "active" })).toBe(false);
  });

  it.each([
    ["Tracker:SeatLimitReached", /more members than that plan allows/],
    ["Tracker:PlanChangeNotAllowed", /ask support/i],
    ["Tracker:BillingDisabled", /turned off/],
    ["Tracker:PlanNotFound", /no longer exists/],
  ])("explains %s", (code, text) => {
    expect(billingErrorText({ code }, "x")).toMatch(text);
  });

  it("falls back to the server's words, then the fallback", () => {
    expect(billingErrorText({ code: "Other", serverMessage: " Nope. " }, "x")).toBe("Nope.");
    expect(billingErrorText(null, "Fallback")).toBe("Fallback");
  });
});

describe("a workspace on a deployment that doesn't bill", () => {
  const live = (patch: Partial<NonNullable<Workspace["live"]>>): Workspace => ({
    id: "live-tenant-x",
    name: "X",
    slug: "x",
    planId: "starter",
    status: "active",
    color: "#000",
    createdAt: "",
    cycleDays: 0,
    seatsUsed: 9,
    projectsUsed: 0,
    storageUsedGb: 0,
    live: { known: true, planName: "Starter", pricePerUser: 6, seatLimit: 5, trialEndsAt: null, ...patch },
  });

  it("is billed unless the server said billing is off", () => {
    expect(billingEnabledFor(live({}))).toBe(true);
    expect(billingEnabledFor(live({ billingEnabled: true }))).toBe(true);
    expect(billingEnabledFor(live({ billingEnabled: false }))).toBe(false);
  });

  it("has no seat limit while billing is off", () => {
    expect(seatLimitFor(live({}))).toBe(5);
    expect(seatLimitFor(live({ billingEnabled: false }))).toBe(Number.POSITIVE_INFINITY);
  });
});

describe("the live session's billing mode", () => {
  const config = {
    currentUser: { isAuthenticated: true, id: "u1", tenantId: "t-1", name: "Mira", email: "m@x.test", roles: ["admin"] },
    currentTenant: { id: "t-1", name: "RealCo" },
    auth: { grantedPolicies: {} },
  };
  function get(billing: unknown, subscription: unknown = { planId: "plan-s", status: "trialing" }): LiveGetter & ReturnType<typeof vi.fn> {
    return vi.fn(async (endpoint: string) => {
      if (endpoint.startsWith("/api/abp/application-configuration")) return config;
      if (endpoint === "/api/app/workspace/billing-mode") {
        if (billing instanceof Error) throw billing;
        return billing;
      }
      if (endpoint.startsWith("/api/app/workspace/current-subscription")) return subscription;
      if (endpoint.startsWith("/api/app/workspace/available-plans")) return [{ id: "plan-s", name: "Starter", pricePerUser: 6, maxSeats: 25 }];
      if (endpoint.startsWith("/api/app/team/members") || endpoint.startsWith("/api/app/project") || endpoint.startsWith("/api/app/activity")) return [];
      return null;
    });
  }

  it("marks the workspace unbilled, and a missing plan is then no warning", async () => {
    const data = await loadLiveSession(get({ enabled: false }, null), new Date("2026-10-26T10:00:00Z"));
    expect(data.workspace.live?.billingEnabled).toBe(false);
    expect(data.warnings).not.toContain("plan");
  });

  it("bills as before when the server can't say", async () => {
    const data = await loadLiveSession(get(new Error("404")), new Date("2026-10-26T10:00:00Z"));
    expect(data.workspace.live).not.toHaveProperty("billingEnabled");
    expect(billingEnabledFor(data.workspace)).toBe(true);
  });

  it("re-reads the subscription on a pulse only when asked to (after a plan change)", async () => {
    const now = new Date("2026-10-26T10:00:00Z");
    const first = await loadLiveSession(get({ enabled: true }), now);
    const later = get({ enabled: true }, { planId: "plan-s", status: "canceled" });
    const pulse = await loadLiveSession(later, now, { previous: first });
    expect(pulse.workspace.status).toBe("trialing");
    expect(later.mock.calls.some(([e]) => String(e).startsWith("/api/app/workspace/current-subscription"))).toBe(false);
    const fresh = await loadLiveSession(later, now, { previous: first, freshPlan: true });
    expect(fresh.workspace.status).toBe("canceled");
  });
});
