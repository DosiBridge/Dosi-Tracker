import { afterAll, beforeEach, describe, it, expect } from "vitest";
import { scopeActivities, scopeProjects, canViewProject } from "./scope";
import { hydrateLiveBackendData } from "./tenant-data";
import type { Activity, Project, Role, User } from "./types";
import { resetPrototypeState } from "@/test/harness";

function user(role: Role, id = "u1"): User {
  return {
    id,
    name: "Test User",
    email: "t@example.com",
    role,
    designation: "",
    status: "active",
    timezone: "UTC",
    trackedToday: 0,
    productivity: 0,
    joinedAt: "2024-01-01",
  };
}

function activity(userId: string, projectId: string, id = userId + projectId): Activity {
  return {
    id,
    userId,
    projectId,
    startedAt: "2024-01-01T09:00:00Z",
    endedAt: "2024-01-01T10:00:00Z",
    description: "work",
    productivity: 50,
    mouseClicks: 0,
    keyboardHits: 0,
    activeWindows: [],
    runningPrograms: [],
    screen: { app: "Code", kind: "editor", accent: "#000" },
    hasWebcam: false,
    online: false,
  };
}

function project(id: string, memberIds: string[]): Project {
  return {
    id,
    title: `Project ${id}`,
    description: "",
    color: "#000",
    archived: false,
    intervalMinutes: 10,
    permissions: {
      screenshot: true,
      webcam: false,
      keyboard: true,
      mouse: true,
      activeWindow: true,
      runningPrograms: true,
    },
    memberIds,
    createdAt: "2024-01-01",
    loggedThisWeek: 0,
    loggedThisMonth: 0,
    loggedTotal: 0,
  };
}

describe("scopeActivities", () => {
  const list = [activity("u1", "p1"), activity("u2", "p1"), activity("u3", "p2")];

  it("owner and admin see every activity", () => {
    expect(scopeActivities(user("owner"), list)).toHaveLength(3);
    expect(scopeActivities(user("admin"), list)).toHaveLength(3);
  });

  it("a worker sees only their own activities", () => {
    const scoped = scopeActivities(user("worker", "u2"), list);
    expect(scoped).toHaveLength(1);
    expect(scoped[0].userId).toBe("u2");
  });

  it("a worker with no activity sees nothing", () => {
    expect(scopeActivities(user("worker", "ghost"), list)).toHaveLength(0);
  });
});

describe("scopeProjects", () => {
  const list = [project("p1", ["u1", "u2"]), project("p2", ["u3"]), project("p3", ["u1"])];

  it("owner/admin see all projects", () => {
    expect(scopeProjects(user("owner"), list)).toHaveLength(3);
    expect(scopeProjects(user("admin"), list)).toHaveLength(3);
  });

  it("a member sees only projects they belong to", () => {
    const scoped = scopeProjects(user("worker", "u1"), list);
    expect(scoped.map((p) => p.id)).toEqual(["p1", "p3"]);
  });

  it("a non-member sees no projects", () => {
    expect(scopeProjects(user("client", "outsider"), list)).toHaveLength(0);
  });
});

describe("canViewProject", () => {
  it("owner/admin may open any project", () => {
    expect(canViewProject(user("owner"), project("p1", []))).toBe(true);
    expect(canViewProject(user("admin"), project("p1", []))).toBe(true);
  });

  it("a non-privileged user may open a project only if they are a member", () => {
    expect(canViewProject(user("worker", "u1"), project("p1", ["u1"]))).toBe(true);
    expect(canViewProject(user("client", "u1"), project("p1", ["u2"]))).toBe(false);
  });
});

/** A project whose payload carried no member list at all. */
const withoutMembers = (id: string): Project => ({ ...project(id, []), memberIds: undefined as unknown as string[] });

describe("scopeActivities — a client sees only their projects' work", () => {
  // A client's scope is read from the active workspace's projects (tenant-data), re-pointed here
  // through the real hydration entry point.
  beforeEach(() => {
    resetPrototypeState();
  });
  afterAll(() => {
    resetPrototypeState();
  });

  const list = [activity("u1", "p1"), activity("u3", "p2"), activity("u2", "p3"), activity("c1", "p1")];

  it("keeps the activities of the projects the client is a member of — everyone's on those, nobody's elsewhere", () => {
    hydrateLiveBackendData([project("p1", ["c1", "u1"]), project("p2", ["u3"]), withoutMembers("p3")], []);
    expect(scopeActivities(user("client", "c1"), list).map((a) => a.id)).toEqual(["u1p1", "c1p1"]);
  });

  it("a client of no project sees nothing, and a project without a member list counts for no one", () => {
    hydrateLiveBackendData([project("p1", ["u1"]), withoutMembers("p3")], []);
    expect(scopeActivities(user("client", "c1"), list)).toEqual([]);
  });
});

describe("projects without a member list", () => {
  it("are listed and opened for owners and admins only", () => {
    const bare = withoutMembers("p9");
    expect(scopeProjects(user("worker", "u1"), [bare])).toEqual([]);
    expect(canViewProject(user("worker", "u1"), bare)).toBe(false);
    expect(canViewProject(user("admin"), bare)).toBe(true);
  });
});
