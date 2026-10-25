// The dataset registry outside and inside demo mode. Each test gets a FRESH
// module registry (vi.resetModules), so the demo tenants the test setup
// installs for everyone else are absent until a test installs them itself —
// exactly a live deployment, which never loads lib/demo-tenants.
import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(() => {
  vi.resetModules();
});

async function freshDataLayer() {
  vi.resetModules();
  const tenantData = await import("./tenant-data");
  const saas = await import("./saas-data");
  return { tenantData, saas };
}

describe("outside demo mode (lib/demo-tenants never loaded)", () => {
  it("every non-live workspace reads as empty, never as demo data", async () => {
    const { tenantData } = await freshDataLayer();
    expect(tenantData.demoDatasetsInstalled()).toBe(false);
    for (const id of ["w1", "w2", "w3", "w-123", ""]) {
      const ds = tenantData.datasetFor(id);
      expect(ds.users, id).toEqual([]);
      expect(ds.projects, id).toEqual([]);
      expect(ds.activities, id).toEqual([]);
      expect(ds.notifications, id).toEqual([]);
      expect(ds.live, id).toBeUndefined();
    }
    // The live bindings start empty too.
    expect(tenantData.users).toEqual([]);
    expect(tenantData.activities).toEqual([]);
    expect(tenantData.isLiveDataset()).toBe(false);
  });

  it("offers no demo workspaces", async () => {
    const { saas } = await freshDataLayer();
    expect(saas.demoWorkspaces()).toEqual([]);
    expect(saas.primaryDemoWorkspace()).toBeNull();
  });

  it("a sign-out drops the live tenant and leaves nothing behind", async () => {
    const { tenantData } = await freshDataLayer();
    tenantData.installLiveDataset({
      workspaceId: "live-t1",
      users: [{ id: "u", name: "Real", email: "r@x.test", role: "owner", designation: "", status: "active", timezone: "UTC", trackedToday: 0, productivity: 0, joinedAt: "" }],
      projects: [],
      activities: [],
      now: new Date(),
    });
    expect(tenantData.users).toHaveLength(1);
    tenantData.dropLiveDatasets();
    expect(tenantData.users).toEqual([]);
    expect(tenantData.isLiveDataset()).toBe(false);
    tenantData.resetTenantRegistryForTests();
    expect(tenantData.users).toEqual([]);
  });
});

describe("demo mode (lib/demo-tenants installed on demand)", () => {
  it("installing swaps the empty datasets for the demo's, and the primary demo workspace becomes active", async () => {
    const { tenantData, saas } = await freshDataLayer();
    expect(tenantData.datasetFor("w1").users).toEqual([]); // read (and cached) before the demo arrived
    const demo = await import("./demo-tenants");
    demo.installDemoTenants();
    demo.installDemoTenants(); // idempotent

    expect(tenantData.demoDatasetsInstalled()).toBe(true);
    expect(tenantData.datasetFor("w1").users.length).toBeGreaterThan(0);
    expect(tenantData.activeWorkspaceId()).toBe("w1");
    expect(tenantData.users.length).toBeGreaterThan(0);
    expect(saas.demoWorkspaces().map((w) => w.id)).toEqual(["w1", "w2", "w3"]);
    expect(saas.primaryDemoWorkspace()?.id).toBe("w1");
    // An unknown demo id is a freshly created, empty demo workspace.
    expect(tenantData.datasetFor("w-new").users.map((u) => u.name)).toEqual(["You"]);
  });

  it("never replaces an active live tenant", async () => {
    const { tenantData } = await freshDataLayer();
    tenantData.installEmptyLiveDataset("live-t1");
    const demo = await import("./demo-tenants");
    demo.installDemoTenants();
    expect(tenantData.activeWorkspaceId()).toBe("live-t1");
    expect(tenantData.isLiveDataset()).toBe(true);
    // A sign-out then lands on the primary demo workspace.
    tenantData.dropLiveDatasets();
    expect(tenantData.activeWorkspaceId()).toBe("w1");
  });
});
