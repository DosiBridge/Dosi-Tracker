import { describe, expect, it } from "vitest";
import {
  buildLiveUser,
  buildLiveWorkspace,
  daysUntil,
  isAuthenticatedConfig,
  isHostConfig,
  LIVE_WORKSPACE_PREFIX,
  liveWorkspaceId,
  planIdFromName,
  resolveLiveRole,
  subscriptionStatus,
  type AppConfiguration,
} from "./live-identity";

const NOW = new Date("2026-09-23T12:00:00.000Z");

function config(overrides: {
  tenantId?: string | null;
  tenantName?: string | null;
  roles?: string[] | null;
  policies?: Record<string, boolean> | null;
  name?: string | null;
  surName?: string | null;
  userName?: string | null;
  email?: string | null;
} = {}): AppConfiguration {
  const tenantId = overrides.tenantId === undefined ? "t-1" : overrides.tenantId;
  return {
    currentUser: {
      isAuthenticated: true,
      id: "u-1",
      tenantId,
      userName: overrides.userName === undefined ? "mira" : overrides.userName,
      name: overrides.name === undefined ? "Mira" : overrides.name,
      surName: overrides.surName === undefined ? "Okafor" : overrides.surName,
      email: overrides.email === undefined ? "mira@realco.test" : overrides.email,
      roles: overrides.roles === undefined ? [] : overrides.roles,
    },
    currentTenant: tenantId ? { id: tenantId, name: overrides.tenantName === undefined ? "RealCo" : overrides.tenantName } : null,
    auth: { grantedPolicies: overrides.policies === undefined ? {} : overrides.policies },
  };
}

describe("isAuthenticatedConfig / isHostConfig", () => {
  it("requires an authenticated user with an id", () => {
    expect(isAuthenticatedConfig(config())).toBe(true);
    expect(isAuthenticatedConfig({ currentUser: { isAuthenticated: false } })).toBe(false);
    expect(isAuthenticatedConfig({ currentUser: { isAuthenticated: true, id: null } })).toBe(false);
    expect(isAuthenticatedConfig(null)).toBe(false);
    expect(isAuthenticatedConfig("x")).toBe(false);
  });

  it("host = no tenant in scope", () => {
    expect(isHostConfig(config({ tenantId: null }))).toBe(true);
    expect(isHostConfig(config())).toBe(false);
  });
});

describe("resolveLiveRole — real identity → app role", () => {
  it.each([
    ["tenant admin role → owner", config({ roles: ["admin"] }), "owner"],
    ["role match is case-insensitive", config({ roles: ["Admin"] }), "owner"],
    ["Team.Manage → admin", config({ policies: { "Tracker.Team.Manage": true } }), "admin"],
    ["Activities.ViewAll → admin", config({ policies: { "Tracker.Activities.ViewAll": true } }), "admin"],
    ["plain member → worker", config(), "worker"],
    ["missing roles/policies → worker", config({ roles: null, policies: null }), "worker"],
    ["host with Tracker.Platform → host", config({ tenantId: null, policies: { "Tracker.Platform": true } }), "host"],
    ["host admin → host", config({ tenantId: null, roles: ["admin"] }), "host"],
    ["host-side account without rights → worker", config({ tenantId: null }), "worker"],
  ] as const)("%s", (_label, cfg, expected) => {
    expect(resolveLiveRole(cfg)).toBe(expected);
  });
});

describe("buildLiveUser", () => {
  it("uses the real name, email and mapped role — never a demo person", () => {
    expect(buildLiveUser(config({ roles: ["admin"] }))).toMatchObject({
      id: "u-1",
      name: "Mira Okafor",
      email: "mira@realco.test",
      role: "owner",
      designation: "Workspace Owner",
    });
  });

  it("falls back to userName, then email, then 'You'", () => {
    expect(buildLiveUser(config({ name: null, surName: null })).name).toBe("mira");
    expect(buildLiveUser(config({ name: null, surName: null, userName: null })).name).toBe("mira@realco.test");
    expect(buildLiveUser(config({ name: null, surName: null, userName: null, email: null }))).toMatchObject({ name: "You", email: "" });
    expect(buildLiveUser({}).id).toBe("me");
  });
});

describe("plan + subscription mapping", () => {
  it.each([
    ["Free", "free"],
    [" starter ", "starter"],
    ["BUSINESS", "business"],
    ["Enterprise", "enterprise"],
    ["Custom Gold", "enterprise"],
    ["", "free"],
    [null, "free"],
  ] as const)("planIdFromName(%j) → %s", (name, id) => {
    expect(planIdFromName(name)).toBe(id);
  });

  it.each([
    ["trialing", "trialing"],
    ["Trial", "trialing"],
    ["past_due", "past_due"],
    ["past-due", "past_due"],
    ["PastDue", "past_due"],
    ["active", "active"],
    [null, "active"],
  ] as const)("subscriptionStatus(%j) → %s", (raw, status) => {
    expect(subscriptionStatus(raw)).toBe(status);
  });

  it("daysUntil rounds up, never goes negative, and pins zone-less dates to UTC", () => {
    expect(daysUntil("2026-09-25T12:00:00Z", NOW)).toBe(2);
    expect(daysUntil("2026-09-24T00:00:00", NOW)).toBe(1);
    expect(daysUntil("2026-09-01T00:00:00Z", NOW)).toBe(0);
    expect(daysUntil(null, NOW)).toBe(0);
    expect(daysUntil("garbage", NOW)).toBe(0);
  });
});

describe("buildLiveWorkspace", () => {
  const plans = [
    { id: "plan-s", name: "Starter", pricePerUser: 6, maxSeats: 25 },
    { id: "plan-x", name: "Unlimited", pricePerUser: 20, maxSeats: 0 },
  ];

  it("names the workspace after the real tenant and quotes the backend's plan", () => {
    const ws = buildLiveWorkspace(
      config({ tenantName: "RealCo Ltd." }),
      { planId: "plan-s", status: "trialing", trialEndsAt: "2026-09-30T12:00:00Z" },
      plans,
      NOW,
      { seatsUsed: 3, projectsUsed: 2 },
    );
    expect(ws).toMatchObject({
      id: `${LIVE_WORKSPACE_PREFIX}tenant-t-1`,
      name: "RealCo Ltd.",
      slug: "realco-ltd",
      planId: "starter",
      status: "trialing",
      cycleDays: 7,
      seatsUsed: 3,
      projectsUsed: 2,
      live: { known: true, planName: "Starter", pricePerUser: 6, seatLimit: 25 },
    });
  });

  it("treats a non-positive seat cap as unlimited", () => {
    const ws = buildLiveWorkspace(config(), { planId: "plan-x", status: "active" }, plans, NOW);
    expect(ws.live?.seatLimit).toBe(Number.POSITIVE_INFINITY);
    expect(ws.cycleDays).toBe(0);
  });

  it("reports an unknown plan honestly instead of borrowing the demo table", () => {
    const ws = buildLiveWorkspace(config({ tenantName: "  " }), null, null, NOW);
    expect(ws).toMatchObject({ name: "Workspace", slug: "workspace", planId: "free", status: "active" });
    expect(ws.live).toEqual({ known: false, planName: null, pricePerUser: null, seatLimit: null, trialEndsAt: null });
    expect(buildLiveWorkspace(config(), { planId: "missing" }, plans, NOW).live?.known).toBe(false);
  });

  it("names the host's pseudo-workspace 'Platform'", () => {
    const cfg = config({ tenantId: null });
    expect(liveWorkspaceId(cfg)).toBe(`${LIVE_WORKSPACE_PREFIX}platform`);
    expect(buildLiveWorkspace(cfg, null, null, NOW).name).toBe("Platform");
  });
});
