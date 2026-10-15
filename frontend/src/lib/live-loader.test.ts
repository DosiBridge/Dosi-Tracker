import { describe, expect, it, vi } from "vitest";
import {
  coversSince,
  describeCoverage,
  describeWarnings,
  fetchLiveIdentity,
  isSameIdentity,
  LIVE_ACTIVITY_WINDOW_DAYS,
  LIVE_ENDPOINTS,
  loadLiveSession,
  MAX_LIVE_ACTIVITY_ROWS,
  PULSE_LOOKBACK_MS,
  SessionExpiredError,
  WorkspaceRequiredError,
  type LiveGetter,
  type LivePageGetter,
} from "./live-loader";
import type { ApiActivityDto } from "./live-dataset";

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

    expect(data.warnings).toEqual(["projects", "activity", "team", "plan"]);
    expect(data.users.map((u) => u.id)).toEqual([OWNER]);
    expect(data.projects).toEqual([]);
    expect(data.activities).toEqual([]);
    expect(data.workspace).toMatchObject({ name: "RealCo", seatsUsed: 1 });
    // An unknown plan is said to be unknown — it never borrows the demo table's Free plan.
    expect(data.workspace.live?.known).toBe(false);
    expect(data.workspace.planId).toBe("enterprise");
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
    expect(describeWarnings(["plan"])).toMatch(/workspace's plan data/);
  });
});

/* ----------------------------- A small in-memory backend ----------------------------- */

const HOUR = 3_600_000;
const DAY = 24 * HOUR;
const P1 = "p1";

/** A block of `minutes` that started `startedAgoMs` before NOW. */
function block(id: string, startedAgoMs: number, minutes = 10, extra: Partial<ApiActivityDto> = {}): ApiActivityDto {
  const start = NOW.getTime() - startedAgoMs;
  return {
    id,
    userId: WORKER,
    projectId: P1,
    startedAt: new Date(start).toISOString(),
    endedAt: new Date(start + minutes * 60_000).toISOString(),
    productivity: 70,
    activeWindowsJson: '[{"appName":"Code","seconds":600}]',
    ...extra,
  };
}

/**
 * GET /api/app/activity over `rows`, the way the backend pages it: blocks that
 * STARTED in [From, To), newest first, SkipCount/MaxResultCount, with the total.
 */
function activityServer(rows: ApiActivityDto[]) {
  const calls: string[] = [];
  const getPage: LivePageGetter = vi.fn(async (endpoint: string) => {
    calls.push(endpoint);
    const url = new URL(endpoint, "https://api.test");
    const from = Date.parse(url.searchParams.get("From") ?? "");
    const to = Date.parse(url.searchParams.get("To") ?? "");
    const skip = Number(url.searchParams.get("SkipCount") ?? 0);
    const take = Number(url.searchParams.get("MaxResultCount") ?? 10);
    const matching = rows
      .filter((r) => Date.parse(r.startedAt) >= from && Date.parse(r.startedAt) < to)
      .sort((a, b) => Date.parse(b.startedAt) - Date.parse(a.startedAt));
    return { items: matching.slice(skip, skip + take), totalCount: matching.length };
  });
  return { getPage, calls };
}

interface Backend {
  config?: unknown;
  members?: unknown;
  projects?: unknown;
  presence?: unknown;
  subscription?: unknown;
  plans?: unknown;
  preferences?: unknown;
  settings?: unknown;
  /** Summary by period name, chosen from the request's From. */
  summaries?: Partial<Record<"today" | "week" | "month" | "window", unknown>>;
  projectMembers?: Record<string, unknown>;
}

const fail = new Error("unavailable");

/** Every other endpoint the loader reads; an Error value (or a missing one) rejects like a failed call. */
function backend(b: Backend, zone = "UTC"): LiveGetter & ReturnType<typeof vi.fn> {
  const at = (iso: string) => new Date(iso).toISOString();
  const periods: Record<string, keyof NonNullable<Backend["summaries"]>> = {
    [at(new Date(NOW.getTime() - 7 * DAY).toISOString())]: "week",
    [at(new Date(NOW.getTime() - LIVE_ACTIVITY_WINDOW_DAYS * DAY).toISOString())]: "window",
  };
  // Today and this month start at local midnight in the zone; for UTC that's plain midnight.
  if (zone === "UTC") {
    periods[at("2026-09-23T00:00:00.000Z")] = "today";
    periods[at("2026-09-01T00:00:00.000Z")] = "month";
  }
  return vi.fn(async (endpoint: string) => {
    const path = endpoint.split("?")[0];
    const answer = (value: unknown) => {
      if (value === undefined || value instanceof Error) throw value ?? fail;
      return value;
    };
    if (path === "/api/abp/application-configuration") return answer(b.config ?? tenantConfig);
    if (path === "/api/app/team/members") return answer(b.members);
    if (path === "/api/app/project") return answer(b.projects);
    if (path === "/api/app/presence/team") return answer(b.presence);
    if (path === "/api/app/workspace/current-subscription") return answer(b.subscription);
    if (path === "/api/app/workspace/available-plans") return answer(b.plans);
    if (path === "/api/app/workspace/my-preferences") return answer(b.preferences);
    if (path === "/api/app/workspace/settings") return answer(b.settings);
    if (path === "/api/app/reporting/summary") {
      const from = new URL(endpoint, "https://api.test").searchParams.get("From") ?? "";
      const period = periods[from];
      return answer(period ? b.summaries?.[period] : undefined);
    }
    if (path.startsWith("/api/app/team/project-members/")) return answer(b.projectMembers?.[decodeURIComponent(path.split("/").pop() ?? "")]);
    throw new Error(`unexpected ${endpoint}`);
  });
}

const team = [
  { userId: OWNER, name: "Mira", surname: "Okafor", isOwner: true, projectIds: [P1] },
  { userId: WORKER, userName: "sam", projectIds: [P1] },
];
const plan = { subscription: { planId: "plan-s", status: "active" }, plans: [{ id: "plan-s", name: "Starter", pricePerUser: 6, maxSeats: 25 }] };

describe("loading the activity rows", () => {
  it("pages through every block of the window — the old 1,000-row cap is gone", async () => {
    const rows = Array.from({ length: 2500 }, (_, i) => block(`a${i}`, (i + 1) * 15 * 60_000, 10));
    const server = activityServer(rows);
    const data = await loadLiveSession(backend({ members: team, projects: [{ id: P1, title: "Apollo" }], ...plan }), NOW, { getPage: server.getPage, browserZone: "UTC" });

    expect(data.activities).toHaveLength(2500);
    expect(server.calls.map((c) => new URL(c, "https://x").searchParams.get("SkipCount"))).toEqual(["0", "1000", "2000"]);
    expect(server.calls.every((c) => new URL(c, "https://x").searchParams.get("To") === NOW.toISOString())).toBe(true);
    expect(data.coverage).toMatchObject({ loaded: 2500, total: 2500, truncated: false });
    expect(data.coverage.completeFrom).toBe(data.coverage.from);
    expect(describeCoverage(data.coverage)).toBeNull();
  });

  it(`stops at ${MAX_LIVE_ACTIVITY_ROWS.toLocaleString("en")} rows, keeps the newest, and says the list is partial`, async () => {
    const rows = Array.from({ length: 6000 }, (_, i) => block(`a${i}`, (i + 1) * 6 * 60_000, 5));
    const server = activityServer(rows);
    const data = await loadLiveSession(backend({ members: team, projects: [], ...plan }), NOW, { getPage: server.getPage, browserZone: "UTC" });

    expect(server.calls).toHaveLength(5);
    expect(data.activities).toHaveLength(MAX_LIVE_ACTIVITY_ROWS);
    expect(data.activities[0].id).toBe("a0"); // newest first
    const oldestHeld = data.activities[data.activities.length - 1];
    expect(data.coverage).toMatchObject({ loaded: 5000, total: 6000, truncated: true, completeFrom: oldestHeld.startedAt });
    expect(describeCoverage(data.coverage)).toBe(
      "Showing the newest 5,000 of 6,000 activity blocks from the last 30 days. Totals are complete; lists and charts built from blocks may be partial.",
    );
    // Rows cover the last 20 days only: a page may total the last week from them, not the last month.
    expect(coversSince(data.coverage, new Date(NOW.getTime() - 7 * DAY))).toBe(true);
    expect(coversSince(data.coverage, new Date(NOW.getTime() - 25 * DAY))).toBe(false);
    expect(coversSince(null, new Date(0))).toBe(true);
  });

  it("without a paged getter, reads one page from `get` (older wiring) and still reports a complete list", async () => {
    const data = await loadLiveSession(
      getter({
        "/api/abp/application-configuration": tenantConfig,
        "/api/app/team/members": team,
        "/api/app/project": [],
        "/api/app/activity": [block("only", HOUR)],
        "/api/app/workspace/current-subscription": plan.subscription,
        "/api/app/workspace/available-plans": plan.plans,
      }),
      NOW,
    );
    expect(data.activities.map((a) => a.id)).toEqual(["only"]);
    expect(data.coverage).toMatchObject({ loaded: 1, total: 1, truncated: false });
  });
});

describe("exact totals from the server", () => {
  it("takes today's per-member figures and the week / month / window project totals from the reporting summary", async () => {
    const server = activityServer([block("a1", 2 * HOUR, 30)]);
    const data = await loadLiveSession(
      backend({
        members: team,
        projects: [{ id: P1, title: "Apollo" }],
        ...plan,
        summaries: {
          today: { totalTrackedMinutes: 515, averageProductivity: 64, perUser: [{ userId: WORKER, trackedMinutes: 480, averageProductivity: 63.6 }], perProject: [{ projectId: P1, trackedMinutes: 515 }] },
          week: { totalTrackedMinutes: 2400, perUser: [], perProject: [{ projectId: P1, trackedMinutes: 2400 }] },
          month: { totalTrackedMinutes: 9100, perUser: [], perProject: [{ projectId: P1, trackedMinutes: 9100 }] },
          window: { totalTrackedMinutes: 12000, perUser: [], perProject: [{ projectId: P1, trackedMinutes: 12000 }] },
        },
      }),
      NOW,
      { getPage: server.getPage, browserZone: "UTC" },
    );
    const sam = data.users.find((u) => u.id === WORKER);
    expect(sam).toMatchObject({ trackedToday: 480, productivity: 64 }); // not the 30 minutes of rows held
    expect(data.users.find((u) => u.id === OWNER)).toMatchObject({ trackedToday: 0, productivity: 0 });
    expect(data.totals.today?.totalMinutes).toBe(515);
    expect(data.totals.projects.week?.get(P1)).toBe(2400);
    expect(data.totals.projects.month?.get(P1)).toBe(9100);
    expect(data.totals.projects.window?.get(P1)).toBe(12000);
  });

  it("sends the reporting zone with every summary call", async () => {
    const get = backend({ members: team, projects: [], ...plan, preferences: { timeZone: "Asia/Dhaka" } }, "Asia/Dhaka");
    await loadLiveSession(get, NOW, { getPage: activityServer([]).getPage, browserZone: "UTC" });
    const summaryCalls = get.mock.calls.map(([endpoint]) => String(endpoint)).filter((e) => e.startsWith("/api/app/reporting/summary"));
    expect(summaryCalls).toHaveLength(4); // today, the last 7 days, this month, the session window
    for (const call of summaryCalls) expect(new URL(call, "https://api.test").searchParams.get("TimeZone")).toBe("Asia/Dhaka");
  });

  it("falls back to the rows when the summary isn't available (older server) — without a warning", async () => {
    const server = activityServer([block("a1", 2 * HOUR, 30)]);
    const data = await loadLiveSession(backend({ members: team, projects: [], ...plan }), NOW, { getPage: server.getPage, browserZone: "UTC" });
    expect(data.totals.today).toBeNull();
    expect(data.totals.projects).toEqual({ week: null, month: null, window: null });
    expect(data.users.find((u) => u.id === WORKER)?.trackedToday).toBe(30);
    expect(data.warnings).toEqual([]);
  });
});

describe("the reporting zone", () => {
  const load = (b: Backend) =>
    loadLiveSession(backend({ members: team, projects: [], ...plan, ...b }), NOW, { getPage: activityServer([]).getPage, browserZone: "America/New_York" });

  it("is the member's preference, then the workspace default, then the browser's", async () => {
    expect((await load({ preferences: { timeZone: "Asia/Dhaka" }, settings: { defaultTimeZone: "Europe/Berlin" } })).timeZone).toBe("Asia/Dhaka");
    expect((await load({ preferences: { timeZone: null }, settings: { defaultTimeZone: "Europe/Berlin" } })).timeZone).toBe("Europe/Berlin");
    expect((await load({ preferences: { timeZone: "Mars/Olympus" } })).timeZone).toBe("America/New_York");
    expect((await load({})).timeZone).toBe("America/New_York"); // both calls failed (an older server)
  });

  it("counts a member's 'today' in that zone", async () => {
    // 20:00–21:00 UTC yesterday is today (the 23rd) in Dhaka.
    const server = activityServer([block("late", 19 * HOUR, 60)]);
    const data = await loadLiveSession(backend({ members: team, projects: [], ...plan, preferences: { timeZone: "Asia/Dhaka" } }), NOW, {
      getPage: server.getPage,
      browserZone: "UTC",
    });
    expect(data.users.find((u) => u.id === WORKER)?.trackedToday).toBe(60);
    expect(data.user.timezone).toBe("Asia/Dhaka");
  });
});

describe("presence", () => {
  it("comes from the agents' heartbeats when the server has them", async () => {
    const data = await loadLiveSession(
      backend({
        members: team,
        projects: [],
        ...plan,
        presence: [
          { userId: WORKER, status: "paused", online: true },
          { userId: OWNER, status: "tracking", online: true },
        ],
      }),
      NOW,
      { getPage: activityServer([]).getPage, browserZone: "UTC" },
    );
    expect(data.users.find((u) => u.id === WORKER)?.status).toBe("idle");
    expect(data.user.status).toBe("active");
  });

  it("otherwise judges a member by their project's interval: a 30-minute project's block can be 25 minutes old", async () => {
    const server = activityServer([block("a1", 55 * 60_000, 30)]); // ended 25 minutes ago
    const load = (interval: number) =>
      loadLiveSession(backend({ members: team, projects: [{ id: P1, title: "Apollo", intervalMinutes: interval }], ...plan }), NOW, {
        getPage: server.getPage,
        browserZone: "UTC",
      });
    expect((await load(30)).users.find((u) => u.id === WORKER)?.status).toBe("active");
    expect((await load(10)).users.find((u) => u.id === WORKER)?.status).toBe("offline");
  });
});

describe("clients", () => {
  const CLIENT = "44444444-4444-4444-4444-444444444444";
  const clientConfig = {
    currentUser: { isAuthenticated: true, id: CLIENT, tenantId: "t-1", userName: "nora", email: "nora@brandhaus.test", roles: [] },
    currentTenant: { id: "t-1", name: "RealCo" },
    auth: { grantedPolicies: {} },
  };

  it("someone who only takes part as a Client is a client, and sees their project's team by name", async () => {
    const data = await loadLiveSession(
      backend({
        config: clientConfig,
        members: [{ userId: CLIENT, userName: "nora", projectIds: [P1] }],
        projects: [{ id: P1, title: "Brand refresh" }],
        ...plan,
        projectMembers: {
          [P1]: [
            { projectId: P1, userId: CLIENT, role: "Client" },
            { projectId: P1, userId: WORKER, role: "Worker", name: "Sam", surname: "Lee" },
            { projectId: P1, userId: OWNER, role: "Admin", userName: "mira" },
          ],
        },
        summaries: { today: { totalTrackedMinutes: 90, perUser: [{ userId: WORKER, trackedMinutes: 90, averageProductivity: 70 }], perProject: [] } },
      }),
      NOW,
      { getPage: activityServer([]).getPage, browserZone: "UTC" },
    );
    expect(data.user).toMatchObject({ id: CLIENT, role: "client", designation: "Client", status: "offline" });
    expect(data.users.map((u) => [u.name, u.role])).toEqual([
      ["nora", "client"],
      ["Sam Lee", "worker"],
      ["mira", "admin"],
    ]);
    expect(data.users.find((u) => u.id === WORKER)?.trackedToday).toBe(90);
    expect(data.projects[0].memberIds.sort()).toEqual([CLIENT, OWNER, WORKER].sort());
  });

  it("stays a member when any membership tracks, or when a roster can't be read", async () => {
    const asWorker = await loadLiveSession(
      backend({
        config: clientConfig,
        members: [{ userId: CLIENT, projectIds: [P1, "p2"] }],
        projects: [],
        ...plan,
        projectMembers: { [P1]: [{ userId: CLIENT, role: "Client" }], p2: [{ userId: CLIENT, role: "Worker" }] },
      }),
      NOW,
      { getPage: activityServer([]).getPage, browserZone: "UTC" },
    );
    expect(asWorker.user.role).toBe("worker");
    const unknown = await loadLiveSession(
      backend({ config: clientConfig, members: [{ userId: CLIENT, projectIds: [P1] }], projects: [], ...plan, projectMembers: {} }),
      NOW,
      { getPage: activityServer([]).getPage, browserZone: "UTC" },
    );
    expect(unknown.user.role).toBe("worker");
  });

  it("never reads rosters for owners and managers", async () => {
    const get = backend({ members: team, projects: [], ...plan });
    await loadLiveSession(get, NOW, { getPage: activityServer([]).getPage, browserZone: "UTC" });
    expect(get.mock.calls.some(([e]) => String(e).includes("/project-members/"))).toBe(false);
  });
});

describe("pulses (silent refresh)", () => {
  it("re-reads only the newest blocks and merges them into the rows already held", async () => {
    const old = block("old", 5 * DAY);
    const recent = block("recent", HOUR);
    const gone = block("deleted-since", 2 * HOUR);
    const first = await loadLiveSession(backend({ members: team, projects: [], ...plan }), NOW, {
      getPage: activityServer([old, recent, gone]).getPage,
      browserZone: "UTC",
    });
    expect(first.activities.map((a) => a.id).sort()).toEqual(["deleted-since", "old", "recent"]);

    const later = new Date(NOW.getTime() + 10 * 60_000);
    const fresh = { ...block("new", 0, 5), startedAt: new Date(NOW.getTime() + 2 * 60_000).toISOString(), endedAt: new Date(NOW.getTime() + 7 * 60_000).toISOString() };
    const server = activityServer([old, { ...recent, description: "Note added later" }, fresh]);
    const get = backend({ members: team, projects: [], ...plan });
    const second = await loadLiveSession(get, later, { getPage: server.getPage, previous: first, browserZone: "UTC" });

    // One read, of the last few hours only.
    expect(server.calls).toHaveLength(1);
    const from = Date.parse(new URL(server.calls[0], "https://x").searchParams.get("From") ?? "");
    expect(later.getTime() - from).toBe(PULSE_LOOKBACK_MS);
    expect(second.activities.map((a) => a.id)).toEqual(["new", "recent", "old"]);
    expect(second.activities.find((a) => a.id === "recent")?.note).toBe("Note added later");
    expect(second.coverage).toMatchObject({ loaded: 3, total: 3, truncated: false });
    // The plan and the zone are reused, not fetched again.
    const called = get.mock.calls.map(([e]) => String(e).split("?")[0]);
    expect(called).not.toContain("/api/app/workspace/current-subscription");
    expect(called).not.toContain("/api/app/workspace/my-preferences");
  });

  it("drops rows that slid out of the window and keeps what failed to reload instead of blanking it", async () => {
    const edge = block("edge", LIVE_ACTIVITY_WINDOW_DAYS * DAY - 60_000);
    const first = await loadLiveSession(backend({ members: team, projects: [{ id: P1, title: "Apollo" }], ...plan }), NOW, {
      getPage: activityServer([edge]).getPage,
      browserZone: "UTC",
    });
    const later = new Date(NOW.getTime() + 5 * 60_000);
    const second = await loadLiveSession(backend({}), later, { getPage: activityServer([]).getPage, previous: first, browserZone: "UTC" });
    expect(second.activities).toEqual([]); // "edge" is now older than the window
    expect(second.users.map((u) => u.id)).toEqual(first.users.map((u) => u.id));
    expect(second.projects.map((p) => p.id)).toEqual([P1]);
    expect(second.warnings).toEqual(["projects", "team"]);
  });

  it("re-reads the whole window when the previous load's rows had failed, and ignores a previous load of another account", async () => {
    const failed = await loadLiveSession(backend({ members: team, projects: [], ...plan }), NOW, {
      getPage: async () => {
        throw fail;
      },
      browserZone: "UTC",
    });
    expect(failed.warnings).toContain("activity");
    const server = activityServer([block("a", 5 * DAY)]);
    const retried = await loadLiveSession(backend({ members: team, projects: [], ...plan }), NOW, { getPage: server.getPage, previous: failed, browserZone: "UTC" });
    expect(retried.activities.map((a) => a.id)).toEqual(["a"]);

    const other = { ...retried, identity: { ...retried.identity, userId: "someone-else" } };
    const fresh = activityServer([block("b", 5 * DAY)]);
    const full = await loadLiveSession(backend({ members: team, projects: [], ...plan }), NOW, { getPage: fresh.getPage, previous: other, browserZone: "UTC" });
    expect(full.activities.map((a) => a.id)).toEqual(["b"]);
  });
});

describe("identity", () => {
  it("fetchLiveIdentity reads who the token belongs to, or null when it is no longer accepted", async () => {
    expect(await fetchLiveIdentity(getter({ "/api/abp/application-configuration": tenantConfig }))).toEqual({
      userId: OWNER,
      tenantId: "t-1",
      tenantName: "RealCo",
      userName: "mira@realco.test",
    });
    expect(await fetchLiveIdentity(getter({ "/api/abp/application-configuration": { currentUser: { isAuthenticated: false } } }))).toBeNull();
    await expect(fetchLiveIdentity(getter({ "/api/abp/application-configuration": new Error("offline") }))).rejects.toThrow("offline");
  });

  it("isSameIdentity compares the account and the workspace", () => {
    const a = { userId: "u", tenantId: "t", tenantName: "T", userName: "x" };
    expect(isSameIdentity(a, { ...a, userName: "renamed", tenantName: "Renamed" })).toBe(true);
    expect(isSameIdentity(a, { ...a, userId: "v" })).toBe(false);
    expect(isSameIdentity(a, { ...a, tenantId: "other" })).toBe(false);
    expect(isSameIdentity({ ...a, tenantId: null }, { ...a, tenantId: null })).toBe(true);
  });

  it("the loaded session says who it belongs to", async () => {
    const data = await loadLiveSession(backend({ members: team, projects: [], ...plan }), NOW, { getPage: activityServer([]).getPage, browserZone: "UTC" });
    expect(data.identity).toEqual({ userId: OWNER, tenantId: "t-1", tenantName: "RealCo", userName: "mira@realco.test" });
    expect(data.loadedAt).toBe(NOW.toISOString());
  });
});

describe("loadLiveSession — the workspace chosen with X-Workspace-Id", () => {
  const WS = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
  const globalUser = (roles: string[] = [], policies: Record<string, boolean> = {}) => ({
    currentUser: { isAuthenticated: true, id: OWNER, tenantId: null, userName: "mira", email: "mira@realco.test", roles },
    currentTenant: { id: null },
    auth: { grantedPolicies: policies },
  });
  const empty = { "/api/app/": [] };

  it("takes the workspace's name and the caller's role in it from workspaces/mine", async () => {
    const get = getter({
      ...empty,
      "/api/abp/application-configuration": globalUser(),
      "/api/app/workspaces/mine": [
        { id: WS.toUpperCase(), name: "RealCo", role: "manager" },
        { id: "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb", name: "Other", role: "owner" },
      ],
    });
    const data = await loadLiveSession(get, NOW, { workspaceId: WS });
    expect(data.workspace.name).toBe("RealCo");
    expect(data.user.role).toBe("admin");
    expect(data.identity.tenantId?.toLowerCase()).toBe(WS);
  });

  it("maps owner → owner and member → worker", async () => {
    const owner = await loadLiveSession(
      getter({ ...empty, "/api/abp/application-configuration": globalUser(), "/api/app/workspaces/mine": [{ id: WS, name: "A", role: "owner" }] }),
      NOW,
      { workspaceId: WS },
    );
    expect(owner.user.role).toBe("owner");
    const member = await loadLiveSession(
      getter({ ...empty, "/api/abp/application-configuration": globalUser(), "/api/app/workspaces/mine": [{ id: WS, name: "A", role: "member" }] }),
      NOW,
      { workspaceId: WS },
    );
    expect(member.user.role).toBe("worker");
  });

  it("no workspace chosen, or one no longer theirs: WorkspaceRequiredError", async () => {
    const get = getter({ "/api/abp/application-configuration": globalUser(), "/api/app/workspaces/mine": [] });
    await expect(loadLiveSession(get, NOW, { workspaceId: null })).rejects.toBeInstanceOf(WorkspaceRequiredError);
    await expect(loadLiveSession(get, NOW, { workspaceId: WS })).rejects.toBeInstanceOf(WorkspaceRequiredError);
  });

  it("the platform admin with no workspace gets the console", async () => {
    const get = getter({ "/api/abp/application-configuration": globalUser(["platform-admin"]) });
    const data = await loadLiveSession(get, NOW, { workspaceId: null });
    expect(data.user.role).toBe("host");
    expect(get).toHaveBeenCalledTimes(1);
  });

  it("fetchLiveIdentity carries the chosen workspace, so a renewal is the same identity", async () => {
    const get = getter({ "/api/abp/application-configuration": globalUser() });
    const a = await fetchLiveIdentity(get, WS);
    expect(isSameIdentity(a!, { userId: OWNER, tenantId: WS.toUpperCase(), tenantName: "RealCo", userName: "mira" })).toBe(true);
  });
});
