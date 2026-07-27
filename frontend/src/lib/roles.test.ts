import { describe, it, expect } from "vitest";
import {
  canAccess,
  landingFor,
  isTrackedMember,
  isSeatMember,
  trackedMembers,
  countSeats,
  inviteableRoles,
  navAccess,
  roleLabels,
  roleDescriptions,
} from "./roles";
import type { Role } from "./types";

describe("canAccess (route authorization by longest-prefix match)", () => {
  it("grants a role listed for the matched route", () => {
    expect(canAccess("owner", "/billing")).toBe(true);
    expect(canAccess("owner", "/reports")).toBe(true);
    expect(canAccess("worker", "/projects")).toBe(true);
  });

  it("denies a role not listed for the matched route", () => {
    expect(canAccess("worker", "/billing")).toBe(false); // billing is owner-only
    expect(canAccess("client", "/reports")).toBe(false); // reports is owner/admin
    expect(canAccess("admin", "/host")).toBe(false); // host is platform-only
  });

  it("matches the LONGEST prefix, not the first — a nested path resolves to its own rule", () => {
    // "/projects" allows worker, but a deeper "/projects/123/settings" still resolves via "/projects".
    expect(canAccess("worker", "/projects/123/settings")).toBe(true);
    // "/settings" allows client, and a sub-path keeps that rule.
    expect(canAccess("client", "/settings/profile")).toBe(true);
  });

  it("treats a prefix boundary strictly (only exact or slash-delimited segments match)", () => {
    // "/hosting" must NOT match the "/host" rule (that would leak host-only routes).
    expect(canAccess("owner", "/hosting")).toBe(true); // no rule matches -> open
  });

  it("defaults to open when no rule matches the path", () => {
    expect(canAccess("client", "/some-unlisted-route")).toBe(true);
  });
});

describe("landingFor", () => {
  it("sends the platform host to /host and everyone else to /dashboard", () => {
    expect(landingFor("host")).toBe("/host");
    for (const r of ["owner", "admin", "worker", "client"] as Role[]) {
      expect(landingFor(r)).toBe("/dashboard");
    }
  });
});

describe("seat / tracked membership", () => {
  it("counts owner/admin/worker as tracked seat members", () => {
    for (const r of ["owner", "admin", "worker"] as Role[]) {
      expect(isTrackedMember({ role: r })).toBe(true);
      expect(isSeatMember({ role: r })).toBe(true);
    }
  });

  it("excludes client (stakeholder) and host (platform) from seats", () => {
    expect(isTrackedMember({ role: "client" })).toBe(false);
    expect(isTrackedMember({ role: "host" })).toBe(false);
  });

  it("trackedMembers filters a mixed list and countSeats agrees", () => {
    const list = [
      { role: "owner" as Role },
      { role: "client" as Role },
      { role: "worker" as Role },
      { role: "host" as Role },
    ];
    expect(trackedMembers(list)).toHaveLength(2);
    expect(countSeats(list)).toBe(2);
  });
});

describe("navAccess (the authorization matrix itself — pinned exactly)", () => {
  // Pinning the whole map makes any accidental broadening/narrowing of route access a test failure,
  // and kills mutations that would silently rewrite a route or its allowed-role set.
  const expected: Record<string, Role[]> = {
    "/host": ["host"],
    "/dashboard": ["owner", "admin", "worker", "client"],
    "/projects": ["owner", "admin", "worker"],
    "/team": ["owner", "admin"],
    "/monitor": ["owner", "admin", "worker"],
    "/activities": ["owner", "admin", "worker"],
    "/screenshots": ["owner", "admin", "worker"],
    "/timesheet": ["owner", "admin", "worker"],
    "/reports": ["owner", "admin"],
    "/insights": ["owner", "admin"],
    "/billing": ["owner"],
    "/settings": ["owner", "admin", "worker", "client"],
  };

  it("maps exactly the expected routes", () => {
    expect(Object.keys(navAccess).sort()).toEqual(Object.keys(expected).sort());
  });

  it.each(Object.entries(expected))("route %s allows exactly its roles", (route, roles) => {
    expect([...navAccess[route]].sort()).toEqual([...roles].sort());
  });

  it("billing is owner-only and host is host-only (the two tightest routes)", () => {
    expect(navAccess["/billing"]).toEqual(["owner"]);
    expect(navAccess["/host"]).toEqual(["host"]);
  });
});

describe("role display metadata", () => {
  it.each([
    ["host", "Platform Admin"],
    ["owner", "Owner"],
    ["admin", "Administrator"],
    ["worker", "Member"],
    ["client", "Client"],
  ] as [Role, string][])("labels %s as %s", (role, label) => {
    expect(roleLabels[role]).toBe(label);
  });

  it("has a non-empty description for every role", () => {
    (["host", "owner", "admin", "worker", "client"] as Role[]).forEach((r) => {
      expect(roleDescriptions[r].length).toBeGreaterThan(0);
    });
  });
});

describe("inviteableRoles (privilege-escalation guard)", () => {
  it("owner may invite worker/admin/client but the list never contains owner or host", () => {
    const roles = inviteableRoles({ role: "owner" });
    expect(roles).toEqual(["worker", "admin", "client"]);
    expect(roles).not.toContain("owner");
    expect(roles).not.toContain("host");
  });

  it("admin may NOT invite or promote to admin/owner (no self/upward escalation)", () => {
    const roles = inviteableRoles({ role: "admin" });
    expect(roles).toEqual(["worker", "client"]);
    expect(roles).not.toContain("admin");
    expect(roles).not.toContain("owner");
  });

  it("worker and client may invite no one", () => {
    expect(inviteableRoles({ role: "worker" })).toEqual([]);
    expect(inviteableRoles({ role: "client" })).toEqual([]);
  });
});
