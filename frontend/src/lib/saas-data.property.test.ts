import { describe, it, expect } from "vitest";
import fc from "fast-check";
import {
  planById,
  workspaceUsage,
  monthlyCost,
  invoicesFor,
  usagePct,
  fmtLimit,
  type PlanId,
  type SubscriptionStatus,
} from "./saas-data";
import { datasetFor } from "./tenant-data";
import { buildUser, buildWorkspace } from "@/test/factories";

// Factory workspace ids are sequential ("t-ws-N") and never reset in this file, so every
// generated workspace gets a fresh tenant dataset — no cross-run cache contamination.

const planIdArb = fc.constantFrom<PlanId>("free", "starter", "business", "enterprise");
const statusArb = fc.constantFrom<SubscriptionStatus>("active", "trialing", "past_due");

const workspaceArb = fc
  .record({
    planId: planIdArb,
    status: statusArb,
    cycleDays: fc.integer({ min: 0, max: 90 }),
    seatsUsed: fc.integer({ min: 0, max: 10000 }),
    projectsUsed: fc.integer({ min: 0, max: 10000 }),
    storageUsedGb: fc.double({ min: 0, max: 100000, noNaN: true, noDefaultInfinity: true }),
  })
  .map((over) => buildWorkspace(over));

describe("saas-data billing invariants (property-based)", () => {
  it("monthlyCost is null (custom pricing) or a finite non-negative number — for any workspace", () => {
    fc.assert(
      fc.property(workspaceArb, (ws) => {
        const cost = monthlyCost(ws);
        if (ws.planId === "enterprise") {
          expect(cost).toBeNull();
        } else {
          expect(cost).not.toBeNull();
          expect(Number.isFinite(cost)).toBe(true);
          expect(cost!).toBeGreaterThanOrEqual(0);
        }
      })
    );
  });

  it("monthlyCost equals pricePerUser × live seat members for any roster size", () => {
    fc.assert(
      fc.property(planIdArb, fc.integer({ min: 0, max: 40 }), (planId, seatMembers) => {
        const ws = buildWorkspace({ planId });
        const ds = datasetFor(ws.id);
        ds.users.splice(0, ds.users.length);
        for (let i = 0; i < seatMembers; i++) ds.users.push(buildUser({ role: "worker" }));
        // Clients never count toward the bill.
        ds.users.push(buildUser({ role: "client" }));

        const cost = monthlyCost(ws);
        const price = planById(planId).pricePerUser;
        if (price === null) expect(cost).toBeNull();
        else expect(cost).toBe(price * seatMembers);
      }),
      { numRuns: 60 }
    );
  });

  it("every usage metric renders as an integer percentage between 0 and 100", () => {
    fc.assert(
      fc.property(workspaceArb, (ws) => {
        for (const m of workspaceUsage(ws)) {
          const pct = usagePct(m.used, m.limit);
          expect(Number.isInteger(pct)).toBe(true);
          expect(pct).toBeGreaterThanOrEqual(0);
          expect(pct).toBeLessThanOrEqual(100);
        }
      })
    );
  });

  it("usagePct stays an integer in [0,100] for any usage against any non-negative or unlimited limit", () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 0, max: 10000 }),
        // Limit 0 included on purpose: an exhausted/absent quota must clamp, not NaN.
        fc.oneof(fc.integer({ min: 0, max: 10000 }), fc.constant(Infinity)),
        (used, limit) => {
          const pct = usagePct(used, limit);
          expect(Number.isInteger(pct)).toBe(true);
          expect(pct).toBeGreaterThanOrEqual(0);
          expect(pct).toBeLessThanOrEqual(100);
        }
      ),
      // Always exercise the 0-limit boundary (0/0 must not leak NaN).
      { examples: [[0, 0], [1, 0]] }
    );
  });

  it("invoicesFor is never empty and every amount is a finite non-negative number with a real date", () => {
    fc.assert(
      fc.property(workspaceArb, (ws) => {
        const invoices = invoicesFor(ws);
        expect(invoices.length).toBeGreaterThan(0);
        if (ws.status === "trialing") {
          expect(invoices).toHaveLength(1);
          expect(invoices[0].status).toBe("upcoming");
        } else {
          expect(invoices).toHaveLength(5);
          expect(invoices.every((i) => i.status === "paid")).toBe(true);
        }
        for (const inv of invoices) {
          expect(Number.isFinite(inv.amount)).toBe(true);
          expect(inv.amount).toBeGreaterThanOrEqual(0);
          expect(inv.date).toMatch(/^\d{4}-\d{2}-\d{2}$/);
          expect(inv.plan.length).toBeGreaterThan(0);
        }
      })
    );
  });

  it("fmtLimit never leaks 'NaN' for any non-negative or unlimited limit", () => {
    fc.assert(
      fc.property(
        fc.oneof(
          fc.integer({ min: 0, max: 2 ** 31 }),
          fc.double({ min: 0, max: 1e15, noNaN: true, noDefaultInfinity: true }),
          fc.constant(Infinity)
        ),
        (n) => {
          const out = fmtLimit(n);
          expect(typeof out).toBe("string");
          expect(out.length).toBeGreaterThan(0);
          expect(out).not.toContain("NaN");
        }
      )
    );
  });
});
