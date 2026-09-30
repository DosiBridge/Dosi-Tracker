// The LIVE side of the tenant data layer: a real tenant's dataset is built only
// from its real rows, registered under its own id, and never mixes with (or
// overwrites) the demo workspaces.
import { afterAll, beforeEach, describe, expect, it } from "vitest";
import {
  activities,
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
    expect(ds.topApps).toEqual([{ app: "Code", minutes: 60, color: expect.any(String) }]);
    expect(ds.weeklyTrend).toHaveLength(7);
    expect(ds.weeklyTrend[6]).toMatchObject({ tracked: 60, productive: 30 });
    expect(ds.projectDistribution).toEqual([{ name: "Apollo", value: 60, color: project.color }]);
    // Nothing the backend doesn't measure is invented.
    expect(ds.productivitySplit).toEqual([]);
    expect(ds.attendance).toEqual([]);
    expect(ds.billing).toEqual([]);
    expect(ds.notifications).toEqual([]);
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
    expect(topApps[0].app).toBe("Code");
    expect(appCatalog[0].app).toBe("Code");
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
  it("replaces a corrupted persisted list instead of throwing", () => {
    localStorage.setItem("dosi-projects-created-w1", "{\"not\":\"a list\"}");
    const p = buildProject({ id: "t-new", title: "Recovered" });
    expect(() => createTenantProject("w1", p)).not.toThrow();
    const saved: unknown = JSON.parse(localStorage.getItem("dosi-projects-created-w1") ?? "null");
    expect(Array.isArray(saved) && saved.some((x) => (x as { id: string }).id === "t-new")).toBe(true);
  });
});
