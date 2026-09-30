import { describe, expect, it, vi } from "vitest";
import { describeWarnings, LIVE_ENDPOINTS, loadLiveSession, SessionExpiredError, type LiveGetter } from "./live-loader";

const NOW = new Date("2026-09-23T15:00:00.000Z");
const OWNER = "11111111-1111-1111-1111-111111111111";
const WORKER = "22222222-2222-2222-2222-222222222222";

const tenantConfig = {
  currentUser: { isAuthenticated: true, id: OWNER, tenantId: "t-1", name: "Mira", surName: "Okafor", email: "mira@realco.test", roles: ["admin"] },
  currentTenant: { id: "t-1", name: "RealCo" },
  auth: { grantedPolicies: {} },
};

/** A getter that serves a table of prefix → payload (or Error to reject). */
function getter(routes: Record<string, unknown>): LiveGetter & ReturnType<typeof vi.fn> {
  return vi.fn(async (endpoint: string) => {
    const key = Object.keys(routes)
      .sort((a, b) => b.length - a.length)
      .find((k) => endpoint.startsWith(k));
    if (!key) throw new Error(`unexpected ${endpoint}`);
    const value = routes[key];
    if (value instanceof Error) throw value;
    return value;
  });
}

describe("loadLiveSession", () => {
  it("builds the real user, workspace, roster, projects and rows for a tenant", async () => {
    const get = getter({
      "/api/abp/application-configuration": tenantConfig,
      "/api/app/team/members": [
        { userId: OWNER, name: "Mira", surname: "Okafor", isOwner: true, projectIds: ["p1"] },
        { userId: WORKER, userName: "sam", projectIds: ["p1"] },
        { userId: "33333333-3333-3333-3333-333333333333", userName: "no-project", projectIds: [] },
        { bogus: true },
      ],
      "/api/app/project": [{ id: "p1", title: "Apollo" }, { nope: 1 }],
      "/api/app/activity": [
        {
          id: "a1",
          userId: WORKER,
          projectId: "p1",
          startedAt: "2026-09-23T14:00:00Z",
          endedAt: "2026-09-23T14:55:00Z",
          productivity: 70,
          activeWindowsJson: '[{"appName":"Code"}]',
        },
      ],
      "/api/app/workspace/current-subscription": { planId: "plan-s", status: "active" },
      "/api/app/workspace/available-plans": [{ id: "plan-s", name: "Starter", pricePerUser: 6, maxSeats: 25 }],
    });

    const data = await loadLiveSession(get, NOW);

    expect(data.warnings).toEqual([]);
    // The owner has no agent rows: being signed in to the web app is not "tracking now".
    expect(data.user).toMatchObject({ id: OWNER, name: "Mira Okafor", role: "owner", status: "offline" });
    expect(data.users.map((u) => u.name)).toEqual(["Mira Okafor", "sam", "no-project"]);
    expect(data.users[1]).toMatchObject({ role: "worker", trackedToday: 55, status: "active" });
    expect(data.projects).toHaveLength(1);
    expect(data.projects[0]).toMatchObject({ id: "p1", title: "Apollo", memberIds: [OWNER, WORKER] });
    expect(data.activities).toHaveLength(1);
    expect(data.workspace).toMatchObject({ name: "RealCo", planId: "starter", seatsUsed: 2, projectsUsed: 1 });
    expect(data.workspace.live).toMatchObject({ planName: "Starter", seatLimit: 25 });

    const called = get.mock.calls.map(([e]) => String(e));
    expect(called[0]).toBe(LIVE_ENDPOINTS.configuration);
    expect(called.some((e) => e.startsWith("/api/app/activity?From="))).toBe(true);
  });

  it("degrades each failed list to empty + a warning (never demo data), keeping the identity", async () => {
    const down = new Error("503");
    const get = getter({
      "/api/abp/application-configuration": tenantConfig,
      "/api/app/team/members": down,
      "/api/app/project": down,
      "/api/app/activity": down,
      "/api/app/workspace/current-subscription": down,
      "/api/app/workspace/available-plans": down,
    });

    const data = await loadLiveSession(get, NOW);

    expect(data.warnings).toEqual(["projects", "activity", "team"]);
    expect(data.users.map((u) => u.id)).toEqual([OWNER]);
    expect(data.projects).toEqual([]);
    expect(data.activities).toEqual([]);
    expect(data.workspace).toMatchObject({ name: "RealCo", seatsUsed: 1 });
    expect(data.workspace.live?.known).toBe(false);
  });

  it("for the host, loads no tenant data at all", async () => {
    const get = getter({
      "/api/abp/application-configuration": {
        currentUser: { isAuthenticated: true, id: "host-1", tenantId: null, userName: "admin", roles: ["admin"] },
        currentTenant: { id: null },
      },
    });
    const data = await loadLiveSession(get, NOW);
    expect(data.user).toMatchObject({ role: "host", name: "admin" });
    expect(data.workspace.name).toBe("Platform");
    expect(get).toHaveBeenCalledTimes(1);
  });

  it("throws SessionExpiredError when the backend no longer accepts the token", async () => {
    const get = getter({ "/api/abp/application-configuration": { currentUser: { isAuthenticated: false } } });
    await expect(loadLiveSession(get, NOW)).rejects.toBeInstanceOf(SessionExpiredError);
  });

  it("rejects (so the UI shows an error) when the identity call itself fails", async () => {
    const get = getter({ "/api/abp/application-configuration": new Error("offline") });
    await expect(loadLiveSession(get, NOW)).rejects.toThrow("offline");
  });
});

describe("describeWarnings", () => {
  it("is null when everything loaded and a readable sentence otherwise", () => {
    expect(describeWarnings([])).toBeNull();
    expect(describeWarnings(["team"])).toMatch(/workspace's team data/);
    expect(describeWarnings(["projects", "activity", "team"])).toMatch(/projects, activity and team data/);
  });
});
