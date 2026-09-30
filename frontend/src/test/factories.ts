// Deterministic test-data factories. Every builder returns a fresh object with
// sensible defaults and accepts partial overrides — never share references
// between tests. IDs are namespaced ("t-…") so they can never collide with
// seeded demo data (u1…, p1…, w1…).
import type { Activity, Project, Role, User, WindowInfo } from "@/lib/types";
import type { PlanId, Workspace } from "@/lib/saas-data";

let seq = 0;
const nextId = (prefix: string) => `t-${prefix}-${++seq}`;

/** Reset the id counter so test output is stable within a file. */
export function resetFactories(): void {
  seq = 0;
}

export function buildUser(overrides: Partial<User> = {}): User {
  const id = overrides.id ?? nextId("user");
  return {
    id,
    name: "Test Person",
    email: `${id}@example.test`,
    role: "worker" as Role,
    designation: "Engineer",
    status: "active",
    timezone: "UTC",
    trackedToday: 120,
    productivity: 80,
    joinedAt: "2026-01-15",
    ...overrides,
  };
}

export function buildProject(overrides: Partial<Project> = {}): Project {
  return {
    id: overrides.id ?? nextId("project"),
    title: "Test Project",
    description: "A project built by the test factory",
    color: "#6d5efc",
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
    memberIds: [],
    createdAt: "2026-02-01",
    loggedThisWeek: 300,
    loggedThisMonth: 1200,
    loggedTotal: 4800,
    ...overrides,
  };
}

export function buildWindowInfo(overrides: Partial<WindowInfo> = {}): WindowInfo {
  return { appName: "Visual Studio Code", windowTitle: "index.ts", seconds: 300, ...overrides };
}

export function buildActivity(overrides: Partial<Activity> = {}): Activity {
  return {
    id: overrides.id ?? nextId("activity"),
    userId: "u1",
    projectId: "p1",
    startedAt: "2026-07-14T14:00:00.000Z",
    endedAt: "2026-07-14T14:10:00.000Z",
    description: "Implementing new feature",
    productivity: 85,
    mouseClicks: 200,
    keyboardHits: 900,
    activeWindows: [buildWindowInfo()],
    runningPrograms: [buildWindowInfo({ seconds: 0 })],
    screen: { app: "Visual Studio Code", kind: "editor", accent: "#3b82f6" },
    hasWebcam: false,
    online: false,
    ...overrides,
  };
}

export function buildWorkspace(overrides: Partial<Workspace> = {}): Workspace {
  return {
    id: overrides.id ?? nextId("ws"),
    name: "Factory Workspace",
    slug: "factory-workspace",
    planId: "starter" as PlanId,
    status: "active",
    color: "#0ea5e9",
    createdAt: "2026-03-01",
    cycleDays: 12,
    seatsUsed: 2,
    projectsUsed: 1,
    storageUsedGb: 1,
    ...overrides,
  };
}
