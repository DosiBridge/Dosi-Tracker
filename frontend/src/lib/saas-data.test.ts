import { describe, it, expect } from "vitest";
import {
  plans,
  planById,
  workspaces,
  primaryWorkspace,
  workspaceUsage,
  liveSeatsUsed,
  monthlyCost,
  invoicesFor,
  usagePct,
  fmtLimit,
  statusLabel,
  type PlanId,
  type SubscriptionStatus,
} from "./saas-data";
import { NOW } from "./mock-data";
import { countSeats } from "./roles";
import { datasetFor } from "./tenant-data";
import { buildUser, buildWorkspace } from "@/test/factories";

// NOTE ON STATE: datasetFor() caches per-workspace datasets for the lifetime of the
// module. Vitest isolates each test FILE in its own module registry, so mutations here
// cannot leak into other files — but within this file every mutated dataset gets its own
// explicit, unique workspace id so tests stay order-independent.

const DAY = 24 * 60 * 60 * 1000;
const isoDay = (t: number) => new Date(t).toISOString().slice(0, 10);

describe("plan catalog (the pricing page contract)", () => {
  it("offers exactly the four public plans, in ascending order", () => {
    expect(plans.map((p) => p.id)).toEqual(["free", "starter", "business", "enterprise"]);
  });

  it.each([
    ["free", 0],
    ["starter", 6],
    ["business", 12],
  ] as [PlanId, number][])("prices %s at $%d per user per month", (id, price) => {
    expect(planById(id).pricePerUser).toBe(price);
  });

  it("marks enterprise as custom pricing (null, never a number)", () => {
    expect(planById("enterprise").pricePerUser).toBeNull();
  });

  it.each([
    // [plan, seats, projects, storageGb, retentionDays] — hand-written from the pricing spec
    ["free", 3, 2, 1, 7],
    ["starter", 10, 10, 20, 30],
    ["business", 50, Infinity, 500, 180],
    ["enterprise", Infinity, Infinity, Infinity, 365],
  ] as [PlanId, number, number, number, number][])(
    "gives %s exactly %d seats / %d projects / %d GB / %d-day retention",
    (id, seats, projects, storageGb, retentionDays) => {
      const plan = planById(id);
      expect(plan.seats).toBe(seats);
      expect(plan.projects).toBe(projects);
      expect(plan.storageGb).toBe(storageGb);
      expect(plan.retentionDays).toBe(retentionDays);
    }
  );

  it("planById returns the exact plan object for every id", () => {
    for (const p of plans) expect(planById(p.id)).toBe(p);
  });

  it("keeps Dosi Labs (w1) as the primary demo workspace", () => {
    expect(primaryWorkspace.id).toBe("w1");
    expect(workspaces.some((w) => w.id === "w2")).toBe(true);
    expect(workspaces.some((w) => w.id === "w3")).toBe(true);
  });
});

describe("workspaceUsage (live consumption vs plan limits)", () => {
  it("reports w1 seats/projects from the live tenant dataset against business limits", () => {
    const w1 = workspaces.find((w) => w.id === "w1")!;
    const usage = workspaceUsage(w1);
    expect(usage.map((m) => m.label)).toEqual(["Seats", "Projects", "Storage", "Screenshot history"]);

    const ds = datasetFor("w1");
    const [seats, projects, storage, retention] = usage;
    // Seat usage counts only seat-occupying roles (owner/admin/worker) — recomputed
    // independently through the roles helper rather than trusting saas-data.
    expect(seats.used).toBe(countSeats(ds.users));
    expect(seats.limit).toBe(50);
    expect(projects.used).toBe(ds.projects.filter((p) => !p.archived).length);
    expect(projects.limit).toBe(Infinity);
    expect(storage).toMatchObject({ used: 128, limit: 500, unit: "GB" });
    expect(retention).toMatchObject({ used: 180, limit: 180, unit: "days" });
  });

  it("a freshly created (unknown) workspace starts with 1 seat (the owner) and 0 projects", () => {
    const ws = buildWorkspace({ id: "t-fresh-ws", planId: "starter", storageUsedGb: 2 });
    const usage = workspaceUsage(ws);
    expect(usage[0]).toMatchObject({ used: 1, limit: 10 }); // auto-created owner
    expect(usage[1]).toMatchObject({ used: 0, limit: 10 });
    expect(usage[2]).toMatchObject({ used: 2, limit: 20 });
  });

  it("clamps the seat bar at 100% when a free workspace exceeds its 3-seat limit", () => {
    const ws = buildWorkspace({ id: "t-overlimit-free", planId: "free" });
    const ds = datasetFor(ws.id);
    for (let i = 0; i < 4; i++) ds.users.push(buildUser({ role: "worker" }));
    const [seats] = workspaceUsage(ws);
    expect(seats.used).toBe(5); // 1 owner + 4 workers
    expect(seats.limit).toBe(3); // over-limit is reported truthfully…
    expect(usagePct(seats.used, seats.limit)).toBe(100); // …but the bar never overflows
  });
});

describe("monthlyCost (price × live seats)", () => {
  it("is null for enterprise (custom pricing), even with a full roster", () => {
    const ws = buildWorkspace({ id: "t-ent-active", planId: "enterprise" });
    expect(monthlyCost(ws)).toBeNull();
  });

  it("is 0 on the free plan regardless of seats", () => {
    const ws = buildWorkspace({ id: "t-free-cost", planId: "free" });
    expect(monthlyCost(ws)).toBe(0);
  });

  it("charges per live seat, not per the stale seatsUsed field", () => {
    // seatsUsed says 9, but the live tenant dataset for a fresh workspace has exactly
    // 1 seat member (the auto-created owner) — billing must follow the live count.
    const ws = buildWorkspace({ id: "t-starter-cost", planId: "starter", seatsUsed: 9 });
    expect(liveSeatsUsed(ws)).toBe(1);
    expect(monthlyCost(ws)).toBe(6);
  });

  it("charges w1 (business) exactly $12 × its seat-occupying members", () => {
    const w1 = workspaces.find((w) => w.id === "w1")!;
    const expected = 12 * countSeats(datasetFor("w1").users);
    expect(monthlyCost(w1)).toBe(expected);
    expect(expected).toBeGreaterThan(0);
  });

  it("costs $0 for a workspace whose roster holds no seat-occupying member", () => {
    const ws = buildWorkspace({ id: "t-zero-seats", planId: "starter" });
    const ds = datasetFor(ws.id);
    ds.users.splice(0, ds.users.length);
    ds.users.push(buildUser({ role: "client" })); // stakeholders never occupy a seat
    expect(liveSeatsUsed(ws)).toBe(0);
    expect(monthlyCost(ws)).toBe(0);
  });
});

describe("invoicesFor — trialing workspaces", () => {
  it("gives Acme Studio (trialing) a single upcoming invoice dated 9 days after the frozen clock", () => {
    const w2 = workspaces.find((w) => w.id === "w2")!;
    const invoices = invoicesFor(w2);
    expect(invoices).toHaveLength(1);
    const [inv] = invoices;
    expect(inv.status).toBe("upcoming");
    expect(inv.plan).toBe("Starter");
    // Frozen demo clock: 2026-07-14T15:30Z + 9 cycle days.
    expect(inv.date).toBe("2026-07-23");
    expect(inv.amount).toBe(6 * countSeats(datasetFor("w2").users));
    expect(Number.isFinite(inv.amount)).toBe(true);
    expect(inv.amount).toBeGreaterThan(0);
  });

  it("dates the upcoming invoice exactly cycleDays ahead of NOW for any trial length", () => {
    const ws = buildWorkspace({ id: "t-trial-33", planId: "starter", status: "trialing", cycleDays: 33 });
    const [inv] = invoicesFor(ws);
    expect(inv.date).toBe(isoDay(NOW.getTime() + 33 * DAY));
  });

  it("shows $0 (not NaN) on a trialing enterprise workspace with custom pricing", () => {
    const ws = buildWorkspace({ id: "t-ent-trial", planId: "enterprise", status: "trialing", cycleDays: 14 });
    const [inv] = invoicesFor(ws);
    expect(inv.amount).toBe(0);
    expect(Number.isNaN(inv.amount)).toBe(false);
    expect(inv.plan).toBe("Enterprise");
  });
});

describe("invoicesFor — active / past_due workspaces", () => {
  it("gives w1 five paid monthly invoices stepping back 30 days from the frozen clock", () => {
    const w1 = workspaces.find((w) => w.id === "w1")!;
    const invoices = invoicesFor(w1);
    expect(invoices).toHaveLength(5);
    expect(invoices.every((i) => i.status === "paid")).toBe(true);
    expect(invoices.every((i) => i.plan === "Business")).toBe(true);
    expect(invoices.map((i) => i.date)).toEqual([
      "2026-07-14",
      "2026-06-14",
      "2026-05-15",
      "2026-04-15",
      "2026-03-16",
    ]);
  });

  it("bills the same live-seat amount on every history invoice, never NaN or negative", () => {
    const w1 = workspaces.find((w) => w.id === "w1")!;
    const expected = monthlyCost(w1)!;
    for (const inv of invoicesFor(w1)) {
      expect(inv.amount).toBe(expected);
      expect(Number.isFinite(inv.amount)).toBe(true);
      expect(inv.amount).toBeGreaterThanOrEqual(0);
    }
  });

  it("treats past_due like active for history purposes (5 invoices, not a trial invoice)", () => {
    const ws = buildWorkspace({ id: "t-pastdue", planId: "free", status: "past_due" });
    const invoices = invoicesFor(ws);
    expect(invoices).toHaveLength(5);
    expect(invoices.every((i) => i.status === "paid")).toBe(true);
    expect(invoices.every((i) => i.amount === 0)).toBe(true); // free plan
  });

  it("gives every invoice a unique id (React keys / dedupe safety)", () => {
    const w1 = workspaces.find((w) => w.id === "w1")!;
    const ids = invoicesFor(w1).map((i) => i.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});

describe("usagePct boundaries", () => {
  it("is 0 when nothing is used", () => {
    expect(usagePct(0, 10)).toBe(0);
  });

  it("is exactly 100 at the limit", () => {
    expect(usagePct(10, 10)).toBe(100);
  });

  it("clamps to 100 when usage exceeds the limit", () => {
    expect(usagePct(11, 10)).toBe(100);
    expect(usagePct(1000, 3)).toBe(100);
  });

  it("is 0 against an unlimited (Infinity) limit", () => {
    expect(usagePct(0, Infinity)).toBe(0);
    expect(usagePct(999999, Infinity)).toBe(0);
  });

  it("guards a zero limit — 0% when unused, 100% when anything is used (never NaN)", () => {
    expect(usagePct(0, 0)).toBe(0);
    expect(usagePct(1, 0)).toBe(100);
    expect(usagePct(500, 0)).toBe(100);
  });

  it("treats a negative limit like zero (never NaN, Infinity or a negative percent)", () => {
    expect(usagePct(0, -5)).toBe(0);
    expect(usagePct(3, -5)).toBe(100);
  });

  it("rounds to the nearest whole percent", () => {
    expect(usagePct(1, 3)).toBe(33);
    expect(usagePct(2, 3)).toBe(67);
  });
});

describe("fmtLimit", () => {
  it("renders Infinity as the word Unlimited", () => {
    expect(fmtLimit(Infinity)).toBe("Unlimited");
  });

  it("renders finite limits with locale grouping", () => {
    expect(fmtLimit(3)).toBe("3");
    // The exact separator depends on the runtime locale; assert equivalence with the
    // platform's own locale formatting rather than hard-coding "1,000".
    expect(fmtLimit(1000)).toBe((1000).toLocaleString());
    expect(fmtLimit(1000)).not.toContain("NaN");
  });
});

describe("statusLabel", () => {
  it("labels every subscription status with human copy", () => {
    const expected: Record<SubscriptionStatus, string> = {
      active: "Active",
      trialing: "Trial",
      past_due: "Past due",
    };
    expect(statusLabel).toEqual(expected);
  });
});
