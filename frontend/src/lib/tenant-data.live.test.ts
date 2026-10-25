// The LIVE side of the tenant data layer: a real tenant's dataset is built only
// from its real rows, registered under its own id, and never mixes with (or
// overwrites) the demo workspaces.
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  activities,
  activityCoverage,
  appCatalog,
  attendance,
  billing,
  buildLiveDataset,
  createTenantProject,
  datasetFor,
  dropLiveDatasets,
  installEmptyLiveDataset,
  installLiveDataset,
  isLiveDataset,
  notifications,
  productivitySplit,
  projects,
  summary,
  topApps,
  users,
  weeklyTrend,
} from "./tenant-data";
import { buildActivity, buildProject, buildUser } from "@/test/factories";
import { resetPrototypeState } from "@/test/harness";

const NOW = new Date("2026-09-23T15:00:00.000Z");

beforeEach(() => resetPrototypeState());
afterAll(() => resetPrototypeState());

function sample() {
  const owner = buildUser({ id: "live-u1", name: "Mira Okafor", role: "owner", status: "active" });
  const worker = buildUser({ id: "live-u2", name: "Sam Lee", role: "worker", status: "offline" });
  const project = buildProject({ id: "live-p1", title: "Apollo", memberIds: ["live-u1", "live-u2"], loggedThisWeek: 999 });
  const rows = [
    buildActivity({
      id: "r1",
      userId: "live-u2",
      projectId: "live-p1",
      startedAt: "2026-09-23T13:00:00.000Z",
      endedAt: "2026-09-23T14:00:00.000Z",
      productivity: 50,
      activeWindows: [{ appName: "Code", windowTitle: "", seconds: 3600 }],
    }),
  ];
  return { owner, worker, project, rows };
}

describe("buildLiveDataset", () => {
  it("derives every series from the real rows and leaves unmeasured ones EMPTY", () => {
    const { owner, worker, project, rows } = sample();
    const ds = buildLiveDataset({ workspaceId: "live-tenant-t1", users: [owner, worker], projects: [project], activities: rows, now: NOW });

    expect(ds.live).toBe(true);
    expect(ds.projects[0].loggedThisWeek).toBe(60); // recomputed from rows, not the authored 999
    expect(project.loggedThisWeek).toBe(999); // the input is not mutated
    expect(ds.projects[0].memberIds).not.toBe(project.memberIds);
    expect(ds.summary).toEqual({
      totalTrackedToday: 60,
      activeMembers: 1,
      totalMembers: 2,
      activeProjects: 1,
      avgProductivity: 50,
      screenshotsToday: 1,
    });
    // Canonical app identity (lib/apps): the agent's "Code" is Visual Studio Code everywhere.
    expect(ds.topApps).toEqual([{ app: "Visual Studio Code", minutes: 60, color: expect.any(String) }]);
    expect(ds.weeklyTrend).toHaveLength(7);
    expect(ds.weeklyTrend[6]).toMatchObject({ tracked: 60, productive: 30 });
    expect(ds.projectDistribution).toEqual([{ name: "Apollo", value: 60, color: project.color }]);
    // Nothing the backend doesn't measure is invented.
    expect(ds.productivitySplit).toEqual([]);
    expect(ds.attendance).toEqual([]);
    expect(ds.billing).toEqual([]);
    expect(ds.notifications).toEqual([]);
  });

  it("'Most used apps — this week' covers the last 7 days, never the whole preloaded window", () => {
    const { owner, project, rows } = sample();
    const monthOld = buildActivity({
      id: "r-old",
      userId: "live-u1",
      projectId: "live-p1",
      startedAt: "2026-09-02T09:00:00.000Z",
      endedAt: "2026-09-02T13:00:00.000Z",
      activeWindows: [{ appName: "zoom.exe", windowTitle: "", seconds: 4 * 3600 }],
    });
    const ds = buildLiveDataset({ workspaceId: "live-x", users: [owner], projects: [project], activities: [...rows, monthOld], now: NOW });
    expect(ds.appCatalog.map((a) => a.app)).toEqual(["Visual Studio Code"]);
    expect(ds.topApps.map((a) => a.app)).toEqual(["Visual Studio Code"]);
  });

  it("prefers the server's exact totals over sums of rows that may be partial", () => {
    const { owner, worker, project, rows } = sample();
    const coverage = {
      from: "2026-08-24T15:00:00.000Z",
      to: NOW.toISOString(),
      loaded: 1,
      total: 7200,
      truncated: true,
      completeFrom: "2026-09-23T13:00:00.000Z",
    };
    const ds = buildLiveDataset({
      workspaceId: "live-tenant-t1",
      users: [owner, worker],
      projects: [project],
      activities: rows,
      now: NOW,
      coverage,
      totals: {
        projects: { week: new Map([["live-p1", 3360]]), month: new Map([["live-p1", 12000]]), window: new Map([["live-p1", 14400]]) },
        today: { projects: new Map(), users: new Map(), totalMinutes: 480, activity: null },
      },
    });
    expect(ds.projects[0]).toMatchObject({ loggedThisWeek: 3360, loggedThisMonth: 12000, loggedTotal: 14400 });
    expect(ds.projectDistribution).toEqual([{ name: "Apollo", value: 3360, color: project.color }]);
    expect(ds.summary.totalTrackedToday).toBe(480);
    expect(ds.summary.avgProductivity).toBe(0); // nothing measured today: the server said null
    expect(ds.activityCoverage).toEqual(coverage);
  });

  it("counts 'today' in the reporting zone when one is given", () => {
    const { owner } = sample();
    // 20:00-21:00 UTC on the 22nd is the 23rd in Dhaka, the 22nd in UTC.
    const late = buildActivity({ id: "late", userId: "live-u1", projectId: "live-p1", startedAt: "2026-09-22T20:00:00.000Z", endedAt: "2026-09-22T21:00:00.000Z" });
    expect(buildLiveDataset({ workspaceId: "live-z", users: [owner], projects: [], activities: [late], now: NOW }).summary.totalTrackedToday).toBe(0);
    const zoned = buildLiveDataset({ workspaceId: "live-z", users: [owner], projects: [], activities: [late], now: NOW, timeZone: "Asia/Dhaka" });
    expect(zoned.summary.totalTrackedToday).toBe(60);
    expect(zoned.timeZone).toBe("Asia/Dhaka");
  });

  it("guards a project whose memberIds is missing", () => {
    const broken = { ...buildProject({ id: "live-p2" }), memberIds: undefined as unknown as string[] };
    const ds = buildLiveDataset({ workspaceId: "live-x", users: [], projects: [broken], activities: [], now: NOW });
    expect(ds.projects[0].memberIds).toEqual([]);
  });
});

describe("installing a live dataset", () => {
  it("points every live binding at the real tenant and keeps the demo workspaces intact", () => {
    const { owner, worker, project, rows } = sample();
    const demoUsersBefore = datasetFor("w1").users.length;

    installLiveDataset({ workspaceId: "live-tenant-t1", users: [owner, worker], projects: [project], activities: rows, now: NOW });

    expect(isLiveDataset()).toBe(true);
    expect(users.map((u) => u.name)).toEqual(["Mira Okafor", "Sam Lee"]);
    expect(projects.map((p) => p.title)).toEqual(["Apollo"]);
    expect(activities).toHaveLength(1);
    expect(summary.totalTrackedToday).toBe(60);
    expect(topApps[0].app).toBe("Visual Studio Code");
    expect(appCatalog[0].app).toBe("Visual Studio Code");
    expect(weeklyTrend).toHaveLength(7);
    expect([productivitySplit, attendance, billing, notifications].every((l) => l.length === 0)).toBe(true);
    expect(datasetFor("live-tenant-t1").users).toHaveLength(2);
    expect(datasetFor("w1").users).toHaveLength(demoUsersBefore);
  });

  it("an empty live dataset has no demo people or notifications", () => {
    installEmptyLiveDataset("live-pending", NOW);
    expect(isLiveDataset()).toBe(true);
    expect(users).toEqual([]);
    expect(projects).toEqual([]);
    expect(notifications).toEqual([]);
    expect(summary.totalMembers).toBe(0);
  });

  it("an unknown live id reads as EMPTY (not a fresh demo tenant with a fake owner)", () => {
    const ds = datasetFor("live-tenant-never-loaded");
    expect(ds.live).toBe(true);
    expect(ds.users).toEqual([]);
    expect(datasetFor("live-tenant-never-loaded")).toBe(ds); // cached
    // …whereas a demo-created workspace still gets its starter owner.
    expect(datasetFor("w-demo-created").users.map((u) => u.name)).toEqual(["You"]);
  });

  it("activityCoverage() reports the live rows' coverage, and nothing for demo data", () => {
    expect(activityCoverage()).toBeNull();
    const { owner, rows } = sample();
    const coverage = { from: "a", to: "b", loaded: 1, total: 1, truncated: false, completeFrom: "a" };
    installLiveDataset({ workspaceId: "live-tenant-t1", users: [owner], projects: [], activities: rows, now: NOW, coverage });
    expect(activityCoverage()).toEqual(coverage);
    dropLiveDatasets();
    expect(activityCoverage()).toBeNull();
  });

  it("a live dataset installed without a coverage report claims none", () => {
    installLiveDataset({ workspaceId: "live-tenant-t2", users: [], projects: [], activities: [], now: NOW });
    expect(isLiveDataset()).toBe(true);
    expect(activityCoverage()).toBeNull();
  });

  it("dropLiveDatasets forgets the tenant and returns to the primary demo workspace", () => {
    const { owner, project, rows } = sample();
    installLiveDataset({ workspaceId: "live-tenant-t1", users: [owner], projects: [project], activities: rows, now: NOW });

    dropLiveDatasets();

    expect(isLiveDataset()).toBe(false);
    expect(users.some((u) => u.name === "Ayesha Rahman")).toBe(true);
    // A later read of the dropped id is a fresh EMPTY dataset, not the old rows.
    expect(datasetFor("live-tenant-t1").activities).toEqual([]);
    // Dropping with nothing live is a no-op.
    dropLiveDatasets();
    expect(isLiveDataset()).toBe(false);
  });
});

describe("createTenantProject — storage robustness", () => {
  afterEach(() => vi.restoreAllMocks());

  it("still adds the project for this page view when the browser blocks site data", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("denied", "SecurityError");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("denied", "SecurityError");
    });
    const p = buildProject({ id: "t-blocked", title: "Kept in memory" });
    expect(() => createTenantProject("w1", p)).not.toThrow();
    expect(datasetFor("w1").projects.some((x) => x.id === "t-blocked")).toBe(true);
  });

  it("replaces a corrupted persisted list instead of throwing", () => {
    localStorage.setItem("dosi-projects-created-w1", "{\"not\":\"a list\"}");
    const p = buildProject({ id: "t-new", title: "Recovered" });
    expect(() => createTenantProject("w1", p)).not.toThrow();
    const saved: unknown = JSON.parse(localStorage.getItem("dosi-projects-created-w1") ?? "null");
    expect(Array.isArray(saved) && saved.some((x) => (x as { id: string }).id === "t-new")).toBe(true);
  });
});

describe("createTenantProject — once per project, and only in its own workspace", () => {
  it("creating the same project twice (a double submit) lists and remembers it once", () => {
    const p = buildProject({ id: "t-once", title: "Only once" });
    createTenantProject("w1", p);
    createTenantProject("w1", p);
    expect(datasetFor("w1").projects.filter((x) => x.id === "t-once")).toHaveLength(1);
    expect(projects.filter((x) => x.id === "t-once")).toHaveLength(1); // the active workspace's bindings
    const saved = JSON.parse(localStorage.getItem("dosi-projects-created-w1") ?? "[]") as { id: string }[];
    expect(saved.filter((x) => x.id === "t-once")).toHaveLength(1);
  });

  it("a project created in another workspace never shows up in the active one", () => {
    createTenantProject("w2", buildProject({ id: "t-elsewhere", title: "Acme only" }));
    expect(datasetFor("w2").projects.some((x) => x.id === "t-elsewhere")).toBe(true);
    expect(projects.some((x) => x.id === "t-elsewhere")).toBe(false);
  });
});
