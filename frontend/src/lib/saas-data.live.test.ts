// Usage and pricing for a LIVE workspace come from the backend's own plan —
// never from the demo plan table (which a paying tenant was being quoted).
import { describe, expect, it } from "vitest";
import {
  UNKNOWN_PLAN_NAME,
  liveSeatsUsed,
  monthlyCost,
  planById,
  planNameFor,
  seatLimitFor,
  usagePct,
  workspaceUsage,
  type Workspace,
} from "./saas-data";
import { workspaces } from "./demo-tenants";

function liveWs(live: Partial<NonNullable<Workspace["live"]>> = {}, seatsUsed = 4): Workspace {
  return {
    id: "live-tenant-t1",
    name: "RealCo",
    slug: "realco",
    planId: "starter",
    status: "active",
    color: "#0d9488",
    createdAt: "",
    cycleDays: 0,
    seatsUsed,
    projectsUsed: 0,
    storageUsedGb: 0,
    live: { known: true, planName: "Starter", pricePerUser: 6, seatLimit: 25, trialEndsAt: null, ...live },
  };
}

describe("live workspace plan facts", () => {
  it("seat limit and plan name come from the backend plan", () => {
    const ws = liveWs();
    expect(seatLimitFor(ws)).toBe(25); // the demo table says Starter = 10
    expect(planById("starter").seats).toBe(10);
    expect(planNameFor(ws)).toBe("Starter");
  });

  it("an unknown backend plan says so and blocks nothing — never the demo table's Free / 3 seats", () => {
    // A failed plan lookup used to read as Free: "8 of 3 seats used · limit reached" and a disabled Invite on a paid workspace.
    const ws = liveWs({ known: false, planName: null, seatLimit: null, pricePerUser: null }, 8);
    expect(seatLimitFor(ws)).toBe(Number.POSITIVE_INFINITY);
    expect(planNameFor(ws)).toBe(UNKNOWN_PLAN_NAME);
    expect(UNKNOWN_PLAN_NAME).toBe("Unknown plan");
    const seats = workspaceUsage(ws)[0];
    expect(seats).toMatchObject({ used: 8, limit: Number.POSITIVE_INFINITY });
    expect(usagePct(seats.used, seats.limit)).toBe(0);
    expect(monthlyCost(ws)).toBeNull();
  });

  it("an unknown plan ignores whatever plan id it carries", () => {
    const ws = { ...liveWs({ known: false, planName: null, seatLimit: null, pricePerUser: null }), planId: "free" as const };
    expect(seatLimitFor(ws)).toBe(Number.POSITIVE_INFINITY);
    expect(planNameFor(ws)).toBe(UNKNOWN_PLAN_NAME);
  });

  it("a known plan without a seat figure is not capped in the browser", () => {
    expect(seatLimitFor(liveWs({ seatLimit: null }))).toBe(Number.POSITIVE_INFINITY);
    expect(planNameFor(liveWs({ planName: null }))).toBe(UNKNOWN_PLAN_NAME);
  });

  it("demo workspaces keep reading the plan table", () => {
    const demo = workspaces[0];
    expect(seatLimitFor(demo)).toBe(planById(demo.planId).seats);
    expect(planNameFor(demo)).toBe(planById(demo.planId).name);
  });
});

describe("live usage + cost", () => {
  it("seats are the backend's count; only metered usage is shown", () => {
    const ws = liveWs();
    expect(liveSeatsUsed(ws)).toBe(4);
    const usage = workspaceUsage(ws);
    expect(usage.map((m) => m.label)).toEqual(["Seats", "Projects"]); // no invented storage/retention
    expect(usage[0]).toMatchObject({ used: 4, limit: 25, unit: "members" });
    expect(usage[1].limit).toBe(Number.POSITIVE_INFINITY);
  });

  it("prices seats at the backend's per-user price (at least one seat)", () => {
    expect(monthlyCost(liveWs())).toBe(24);
    expect(monthlyCost(liveWs({}, 0))).toBe(6);
  });

  it("an unknown price is null (shown as a dash), never the demo price", () => {
    expect(monthlyCost(liveWs({ pricePerUser: null }))).toBeNull();
  });
});
