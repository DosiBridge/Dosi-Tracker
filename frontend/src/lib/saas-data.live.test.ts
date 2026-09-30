// Usage and pricing for a LIVE workspace come from the backend's own plan —
// never from the demo plan table (which a paying tenant was being quoted).
import { describe, expect, it } from "vitest";
import {
  liveSeatsUsed,
  monthlyCost,
  planById,
  planNameFor,
  seatLimitFor,
  workspaces,
  workspaceUsage,
  type Workspace,
} from "./saas-data";

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

  it("an unknown backend plan falls back to the plan id's table entry for limits/name", () => {
    const ws = liveWs({ known: false, planName: null, seatLimit: null, pricePerUser: null });
    expect(seatLimitFor(ws)).toBe(planById("starter").seats);
    expect(planNameFor(ws)).toBe("Starter");
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
