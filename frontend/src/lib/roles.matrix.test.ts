import { describe, it, expect } from "vitest";
import fc from "fast-check";
import { canAccess, navAccess, inviteableRoles, countSeats, trackedMembers } from "./roles";
import type { Role } from "./types";

const allRoles: Role[] = ["host", "owner", "admin", "worker", "client"];

// The EXPECTED permission matrix, hand-written from product intent — deliberately NOT
// derived by calling canAccess. Any drift between this table and the implementation is
// a product decision that must be made consciously.
//
//   host   → platform console only ("/host"); never inside a tenant's app
//   owner  → everything in the tenant, including billing
//   admin  → everything except billing (and never the platform console)
//   worker → own work surfaces: dashboard, projects, monitor, activities,
//            screenshots, timesheet, settings — no team/reports/insights/billing
//   client → progress portal only: dashboard + settings
const expectedAccess: Record<string, Record<Role, boolean>> = {
  "/host": { host: true, owner: false, admin: false, worker: false, client: false },
  "/dashboard": { host: false, owner: true, admin: true, worker: true, client: true },
  "/projects": { host: false, owner: true, admin: true, worker: true, client: false },
  "/team": { host: false, owner: true, admin: true, worker: false, client: false },
  "/monitor": { host: false, owner: true, admin: true, worker: true, client: false },
  "/activities": { host: false, owner: true, admin: true, worker: true, client: false },
  "/screenshots": { host: false, owner: true, admin: true, worker: true, client: false },
  "/timesheet": { host: false, owner: true, admin: true, worker: true, client: false },
  "/reports": { host: false, owner: true, admin: true, worker: false, client: false },
  "/insights": { host: false, owner: true, admin: true, worker: false, client: false },
  "/billing": { host: false, owner: true, admin: false, worker: false, client: false },
  "/settings": { host: false, owner: true, admin: true, worker: true, client: true },
};

describe("canAccess — exhaustive role × route matrix", () => {
  it("the expected table covers exactly the routes navAccess governs (no drift)", () => {
    expect(Object.keys(expectedAccess).sort()).toEqual(Object.keys(navAccess).sort());
  });

  const cases = Object.entries(expectedAccess).flatMap(([route, byRole]) =>
    allRoles.map((role) => ({ route, role, allowed: byRole[role] }))
  );

  it.each(cases)("$role on $route → allowed=$allowed", ({ route, role, allowed }) => {
    expect(canAccess(role, route)).toBe(allowed);
  });
});

describe("canAccess — /host deep paths stay host-only", () => {
  it.each(["/host/tenants/x", "/host/tenants/w2/billing", "/host/revenue"])(
    "host may open %s but no tenant role can",
    (path) => {
      expect(canAccess("host", path)).toBe(true);
      for (const r of ["owner", "admin", "worker", "client"] as Role[]) {
        expect(canAccess(r, path)).toBe(false);
      }
    }
  );
});

describe("canAccess — client progress portal", () => {
  it("a client can open exactly dashboard and settings among governed routes", () => {
    const visible = Object.keys(navAccess).filter((route) => canAccess("client", route));
    expect(visible.sort()).toEqual(["/dashboard", "/settings"]);
  });

  it("blocks a client from capture-review deep paths", () => {
    expect(canAccess("client", "/activities/screens")).toBe(false);
    expect(canAccess("client", "/screenshots/2026-07-14")).toBe(false);
  });
});

describe("canAccess — longest-prefix resolution and nested paths", () => {
  it("lets a worker into /activities and its nested views", () => {
    expect(canAccess("worker", "/activities")).toBe(true);
    expect(canAccess("worker", "/activities/screens")).toBe(true);
  });

  it("keeps nested denies: a worker stays out of /reports/* and an admin out of /billing/*", () => {
    expect(canAccess("worker", "/reports/payroll")).toBe(false);
    expect(canAccess("admin", "/billing/history")).toBe(false);
  });

  it("only slash-delimited segments match a rule — '/hosting' is not '/host'", () => {
    // No rule governs "/hosting", so it falls to the permissive default (see below).
    expect(canAccess("owner", "/hosting")).toBe(true);
    expect(canAccess("client", "/billingual")).toBe(true);
  });

  it("PROPERTY: any deep child of a governed route resolves exactly like the route itself", () => {
    fc.assert(
      fc.property(
        fc.constantFrom(...allRoles),
        fc.constantFrom(...Object.keys(navAccess)),
        fc.string(),
        (role, route, suffix) => {
          expect(canAccess(role, `${route}/${suffix}`)).toBe(canAccess(role, route));
        }
      )
    );
  });
});

describe("canAccess — permissive default for unknown routes (documented current behavior)", () => {
  // CURRENT DESIGN: any path with no navAccess rule is open to EVERY role, including
  // client and host. New sensitive routes MUST be registered in navAccess or they ship
  // unprotected. This test pins that default so a change to deny-by-default is deliberate.
  it.each(allRoles)("%s may open an unlisted route like /totally-new", (role) => {
    expect(canAccess(role, "/totally-new")).toBe(true);
    expect(canAccess(role, "/totally-new/child")).toBe(true);
  });
});

describe("inviteableRoles — full matrix and escalation guard", () => {
  it.each([
    ["owner", ["worker", "admin", "client"]],
    ["admin", ["worker", "client"]],
    ["worker", []],
    ["client", []],
    ["host", []],
  ] as [Role, Role[]][])("%s may assign exactly %j", (inviter, expected) => {
    expect(inviteableRoles({ role: inviter })).toEqual(expected);
  });

  it("no inviter can ever hand out owner or host, and admins can never mint admins", () => {
    for (const inviter of allRoles) {
      const assignable = inviteableRoles({ role: inviter });
      expect(assignable).not.toContain("owner");
      expect(assignable).not.toContain("host");
    }
    expect(inviteableRoles({ role: "admin" })).not.toContain("admin");
  });
});

describe("seat accounting — clients and hosts never occupy seats", () => {
  it("PROPERTY: for any roster, countSeats counts exactly the owner/admin/worker members", () => {
    fc.assert(
      fc.property(fc.array(fc.constantFrom(...allRoles), { maxLength: 50 }), (roles) => {
        const roster = roles.map((role) => ({ role }));
        const expected = roles.filter((r) => r === "owner" || r === "admin" || r === "worker").length;
        expect(countSeats(roster)).toBe(expected);
        const tracked = trackedMembers(roster);
        expect(tracked).toHaveLength(expected);
        expect(tracked.every((m) => m.role !== "client" && m.role !== "host")).toBe(true);
      })
    );
  });
});
