// ProjectsPage behavior in demo mode (no backend token): the create-project
// wizard, title enforcement, cancel, active/archived segregation, and
// persistence of demo-created projects, all through the real page + modal +
// session wiring. Only global fetch is stubbed — demo mode must never call it
// (the page's useApi is disabled without a live session); a never-settling
// promise would hang any accidental request.
//
// Persistence contract: demo-mode creation goes through createTenantProject()
// in src/lib/tenant-data.ts, which writes localStorage
// "dosi-projects-created-w1" — the key datasetFor() reads back on the next
// (re)build — so a created project survives a full reload.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import ProjectsPage from "@/app/(dashboard)/projects/page";
import { renderAsRole, resetPrototypeState, simulateReload } from "@/test/harness";

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

beforeEach(() => {
  resetPrototypeState();
  fetchMock = vi.fn<typeof fetch>().mockImplementation(() => new Promise<Response>(() => {}));
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const newProjectButton = () => screen.getByRole("button", { name: /new project/i });

async function openWizard(user: ReturnType<typeof userEvent.setup>) {
  await user.click(newProjectButton());
  const dialog = screen.getByRole("dialog");
  expect(within(dialog).getByRole("heading", { name: "Create project" })).toBeInTheDocument();
  return dialog;
}

describe("projects page — create project (owner, demo mode)", () => {
  it("the New project button opens the create wizard on its Details step", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });

    const dialog = await openWizard(user);
    expect(within(dialog).getByText("Project name")).toBeInTheDocument();
    expect(within(dialog).getByPlaceholderText("e.g. Mobile App Revamp")).toBeInTheDocument();
  });

  it("cannot advance past Details until a title is entered", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });

    const dialog = await openWizard(user);
    const next = within(dialog).getByRole("button", { name: /continue/i });
    expect(next).toBeDisabled();

    await user.click(next); // click on a disabled button is a no-op
    expect(within(dialog).getByText("Project name")).toBeInTheDocument(); // still on Details

    await user.type(within(dialog).getByPlaceholderText("e.g. Mobile App Revamp"), "Orbital Launch");
    expect(next).toBeEnabled();
  });

  it("completing the wizard closes it and shows the new project card in the active list", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });
    expect(screen.getByText(/5 active/)).toBeInTheDocument(); // w1 seeds 5 active + 1 archived

    const dialog = await openWizard(user);
    await user.type(within(dialog).getByPlaceholderText("e.g. Mobile App Revamp"), "Orbital Launch");
    await user.click(within(dialog).getByRole("button", { name: /continue/i })); // -> Team
    await user.click(within(dialog).getByRole("button", { name: /continue/i })); // -> Tracking
    await user.click(within(dialog).getByRole("button", { name: /create project/i }));

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByText("Orbital Launch")).toBeInTheDocument();
    expect(screen.getByText(/6 active/)).toBeInTheDocument();
    // No network write in demo mode — creation is purely local.
    expect(fetchMock).not.toHaveBeenCalled(); // demo mode never touches the network (not even a list GET)
  });

  it("a demo-created project is persisted and survives a remount (simulated reload)", async () => {
    const user = userEvent.setup();
    const { unmount } = renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });

    const dialog = await openWizard(user);
    await user.type(within(dialog).getByPlaceholderText("e.g. Mobile App Revamp"), "Orbital Launch");
    await user.click(within(dialog).getByRole("button", { name: /continue/i })); // -> Team
    await user.click(within(dialog).getByRole("button", { name: /continue/i })); // -> Tracking
    await user.click(within(dialog).getByRole("button", { name: /create project/i }));
    expect(screen.getByText("Orbital Launch")).toBeInTheDocument();

    // Creation must persist through the tenant-data layer, not just React state.
    const saved = localStorage.getItem("dosi-projects-created-w1");
    expect(saved).toContain("Orbital Launch");

    // Simulate a full reload: module-level dataset caches are dropped, web
    // storage (the persisted key) survives — the fresh render must read the
    // project back through datasetFor()'s localStorage path.
    unmount();
    simulateReload();

    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });
    expect(screen.getByText("Orbital Launch")).toBeInTheDocument();
    expect(screen.getByText(/6 active/)).toBeInTheDocument(); // 5 seeded + the survivor
  });

  it("cancel closes the wizard without adding a card", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });

    const dialog = await openWizard(user);
    await user.type(within(dialog).getByPlaceholderText("e.g. Mobile App Revamp"), "Half Finished");
    await user.click(within(dialog).getByRole("button", { name: /cancel/i }));

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByText("Half Finished")).not.toBeInTheDocument();
    expect(screen.getByText(/5 active/)).toBeInTheDocument();
  });

  it("cancel discards typed input — reopening starts on a blank Details step", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });

    const dialog = await openWizard(user);
    await user.type(within(dialog).getByPlaceholderText("e.g. Mobile App Revamp"), "Half Finished");
    await user.click(within(dialog).getByRole("button", { name: /cancel/i }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    const reopened = await openWizard(user);
    expect(within(reopened).getByPlaceholderText("e.g. Mobile App Revamp")).toHaveValue("");
    expect(within(reopened).getByPlaceholderText("Short summary")).toHaveValue("");
  });

  it("the Archived tab shows archived projects and hides active ones", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });

    expect(screen.getByText("Dosi Web Platform")).toBeInTheDocument();
    expect(screen.queryByText("Data Pipeline")).not.toBeInTheDocument(); // p5 is archived

    await user.click(screen.getByRole("tab", { name: "Archived" }));

    expect(screen.getByText("Data Pipeline")).toBeInTheDocument();
    expect(screen.queryByText("Dosi Web Platform")).not.toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Active" }));
    expect(screen.getByText("Dosi Web Platform")).toBeInTheDocument();
  });

  it("a worker gets no create affordance and only their own projects", () => {
    renderAsRole(<ProjectsPage />, "worker", { route: "/projects" });

    expect(screen.queryByRole("button", { name: /new project/i })).not.toBeInTheDocument();
    // u2 (seeded worker) is a member of Mobile App Revamp but not Acme Corp CRM.
    expect(screen.getByText("Mobile App Revamp")).toBeInTheDocument();
    expect(screen.queryByText("Acme Corp CRM")).not.toBeInTheDocument();
  });
});

describe("projects page — create wizard details (demo mode)", () => {
  it("offers every tracking setting: block length, idle detection, blur and the capture switches", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });

    const dialog = await openWizard(user);
    await user.type(within(dialog).getByPlaceholderText("e.g. Mobile App Revamp"), "Orbital Launch");
    await user.click(within(dialog).getByRole("button", { name: /continue/i }));
    await user.click(within(dialog).getByRole("button", { name: /continue/i }));

    expect(within(dialog).getByRole("radio", { name: "10 min" })).toBeChecked();
    expect(within(dialog).getByLabelText(/idle detection/i)).toHaveValue("5");
    expect(within(dialog).getByRole("switch", { name: /blur screenshots/i })).toHaveAttribute("aria-checked", "false");
    for (const name of [/^screenshots/i, /^webcam/i, /keyboard activity/i, /mouse activity/i, /active window/i, /running programs/i]) {
      expect(within(dialog).getByRole("switch", { name })).toBeInTheDocument();
    }
  });

  it("names every colour swatch for assistive tech and labels every field", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });

    const dialog = await openWizard(user);
    expect(within(dialog).getByLabelText(/project name/i)).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/description/i)).toBeInTheDocument();
    for (const colour of ["Teal", "Blue", "Indigo", "Pink", "Green", "Amber"]) {
      expect(within(dialog).getByRole("radio", { name: colour })).toBeInTheDocument();
    }
    expect(within(dialog).getByRole("radio", { name: "Teal" })).toBeChecked();
    await user.click(within(dialog).getByRole("radio", { name: "Pink" }));
    expect(within(dialog).getByRole("radio", { name: "Pink" })).toBeChecked();
  });

  it("a11y: the create wizard has no axe violations", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });
    const dialog = await openWizard(user);
    expect(await axe(dialog)).toHaveNoViolations();
  });

  it("labels each card's figures and capture settings", () => {
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });

    const card = screen.getByRole("heading", { name: "Mobile App Revamp" }).closest("a")!;
    expect(within(card).getByText("Screenshot every 15 min")).toBeInTheDocument();
    expect(within(card).getByText("This week")).toBeInTheDocument();
    expect(within(card).getByText("Last 30 days")).toBeInTheDocument();
    const captures = within(card).getByRole("list", { name: /what the agent captures/i });
    expect(within(captures).getByText("Webcam: on")).toBeInTheDocument();
    expect(within(captures).getByText("Running programs: off")).toBeInTheDocument();
  });
});

/* ----------------------------- Live mode ----------------------------- */

const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");
const ME = "11111111-1111-1111-1111-111111111111";
const SAM = "22222222-2222-2222-2222-222222222222";

function json(body: unknown, status = 200): Response {
  const text = JSON.stringify(body);
  return {
    ok: status < 300,
    status,
    statusText: status < 300 ? "OK" : "Error",
    headers: new Headers(),
    json: async () => JSON.parse(text),
    text: async () => text,
  } as Response;
}

function liveCalls(path: string, method: string) {
  return fetchMock.mock.calls.filter(([u, i]) => String(u).replace(API, "").startsWith(path) && (i?.method ?? "GET") === method);
}

/** A live tenant; `owner` signs in its owner, otherwise a worker on no project the roster knows of. */
function liveBackend(opts: { owner?: boolean; createStatus?: number } = {}) {
  const owner = opts.owner ?? true;
  fetchMock.mockImplementation(async (input, init) => {
    const url = String(input).replace(API, "");
    const method = init?.method ?? "GET";
    if (url.startsWith("/api/abp/application-configuration")) {
      return json({
        currentUser: { isAuthenticated: true, id: ME, tenantId: "t-1", userName: "mira", name: "Mira", surName: "Okafor", roles: owner ? ["admin"] : [] },
        currentTenant: { id: "t-1", name: "RealCo", isAvailable: true },
        auth: { grantedPolicies: owner ? { "Tracker.Team.Manage": true, "Tracker.Activities.ViewAll": true } : {} },
      });
    }
    // The roster says nothing about the worker's projects (e.g. added after sign-in).
    if (url.startsWith("/api/app/team/members")) {
      return json({
        items: owner
          ? [
              { userId: ME, userName: "mira", name: "Mira", surname: "Okafor", isOwner: true, isManager: true, projectIds: [] },
              { userId: SAM, userName: "sam", name: "Sam", surname: "Lee", email: "sam@realco.test", isOwner: false, isManager: false, projectIds: [] },
            ]
          : [{ userId: ME, userName: "mira", name: "Mira", surname: "Okafor", isOwner: false, isManager: false, projectIds: [] }],
      });
    }
    if (url.startsWith("/api/app/team/project-members/")) {
      return json({ items: [{ id: "m-1", projectId: "p-live", userId: SAM, role: "Worker", hourlyRate: 0, userName: "sam", name: "Sam", surname: "Lee" }] });
    }
    if (url.startsWith("/api/app/team/member") && method === "POST") return json({ id: "m-2" });
    if (url === "/api/app/project" && method === "POST") {
      if (opts.createStatus) return json({ error: { code: "Volo.Abp:Validation", message: "Project name already in use." } }, opts.createStatus);
      return json({ id: "p-new", ...JSON.parse(String(init?.body)) });
    }
    if (url.startsWith("/api/app/project")) {
      return json({
        items: [
          { id: "p-live", title: "Apollo", description: null, color: "#0284c7", intervalMinutes: 20, isArchived: false, allowScreenshot: true, allowWebcam: false, allowKeyboard: true, allowMouse: true, allowActiveWindow: true, allowRunningPrograms: true },
        ],
        totalCount: 1,
      });
    }
    if (url.startsWith("/api/app/workspace/settings")) {
      return json({
        defaultTimeZone: null,
        newProjectDefaults: {
          intervalMinutes: 30,
          allowScreenshot: false,
          allowWebcam: false,
          allowKeyboard: true,
          allowMouse: true,
          allowActiveWindow: false,
          allowRunningPrograms: false,
          blurScreenshots: true,
          idleThresholdMinutes: 10,
        },
      });
    }
    if (url.startsWith("/api/app/reporting/summary")) {
      return json({
        totalActivities: 3,
        totalTrackedMinutes: 270,
        averageProductivity: 70,
        perUser: [],
        perProject: [{ projectId: "p-live", activityCount: 3, trackedMinutes: url.includes("p-live") ? 0 : 270, averageProductivity: 70 }],
      });
    }
    if (url.startsWith("/api/app/reporting/")) return json([]);
    if (url.startsWith("/api/app/activity")) return json({ items: [], totalCount: 0 });
    if (url.startsWith("/api/app/workspace/current-subscription")) return json({ planId: "plan-s", status: "active" });
    if (url.startsWith("/api/app/workspace/available-plans")) return json({ items: [{ id: "plan-s", name: "Starter", pricePerUser: 6, maxSeats: 25 }] });
    if (url.startsWith("/api/app/workspace/")) return json({}, 404);
    if (url.startsWith("/api/app/notification/")) return json({ items: [] });
    throw new Error(`unexpected fetch: ${method} ${url}`);
  });
}

describe("projects page — live mode", () => {
  beforeEach(() => localStorage.setItem("dosi-token", "jwt-live"));

  it("a worker sees the projects the server returns, even when the roster doesn't list them yet", async () => {
    liveBackend({ owner: false });
    renderAsRole(<ProjectsPage />, "worker", { route: "/projects" });

    expect(await screen.findByRole("heading", { name: "Apollo" })).toBeInTheDocument();
    expect(screen.queryByText(/haven’t been added to a project/i)).not.toBeInTheDocument();
  });

  it("takes each card's time from the reporting summary and names its members", async () => {
    liveBackend();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });

    const card = (await screen.findByRole("heading", { name: "Apollo" })).closest("a")!;
    await waitFor(() => expect(within(card).getAllByText("4h 30m")).toHaveLength(2)); // this week and the last 30 days
    expect(within(card).getByText("Screenshot every 20 min")).toBeInTheDocument();
    expect(await within(card).findByText("1 member")).toBeInTheDocument();
    const summaries = liveCalls("/api/app/reporting/summary", "GET").map(([u]) => String(u));
    expect(summaries.some((u) => u.includes("TimeZone="))).toBe(true);
  });

  it("creates a project from the workspace defaults, sends no placeholder description, and adds the people picked", async () => {
    liveBackend();
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });
    await screen.findByRole("heading", { name: "Apollo" });
    await waitFor(() => expect(liveCalls("/api/app/workspace/settings", "GET").length).toBeGreaterThan(0));

    const dialog = await openWizard(user);
    await user.type(within(dialog).getByPlaceholderText("e.g. Mobile App Revamp"), "  Orbital Launch  ");
    await user.click(within(dialog).getByRole("button", { name: /continue/i }));
    await user.click(await within(dialog).findByRole("checkbox", { name: /sam lee/i }));
    await user.click(within(dialog).getByRole("button", { name: /continue/i }));
    // The Tracking step starts from the workspace's saved defaults.
    expect(within(dialog).getByRole("radio", { name: "30 min" })).toBeChecked();
    expect(within(dialog).getByLabelText(/idle detection/i)).toHaveValue("10");
    expect(within(dialog).getByText(/starts from your workspace defaults/i)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: /create project/i }));

    await waitFor(() => expect(liveCalls("/api/app/project", "POST")).toHaveLength(1));
    expect(JSON.parse(String(liveCalls("/api/app/project", "POST")[0][1]?.body))).toEqual({
      title: "Orbital Launch",
      description: null,
      color: "#0d9488",
      intervalMinutes: 30,
      allowScreenshot: false,
      allowWebcam: false,
      allowKeyboard: true,
      allowMouse: true,
      allowActiveWindow: false,
      allowRunningPrograms: false,
      idleThresholdMinutes: 10,
      blurScreenshots: true,
    });
    await waitFor(() => expect(liveCalls("/api/app/team/member", "POST")).toHaveLength(1));
    expect(JSON.parse(String(liveCalls("/api/app/team/member", "POST")[0][1]?.body))).toEqual({
      projectId: "p-new",
      userId: SAM,
      role: "Worker",
      hourlyRate: 0,
    });
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("keeps the wizard open with the server's message when creating fails", async () => {
    liveBackend({ createStatus: 400 });
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });
    await screen.findByRole("heading", { name: "Apollo" });

    const dialog = await openWizard(user);
    await user.type(within(dialog).getByPlaceholderText("e.g. Mobile App Revamp"), "Apollo");
    await user.click(within(dialog).getByRole("button", { name: /continue/i }));
    await user.click(within(dialog).getByRole("button", { name: /continue/i }));
    await user.click(within(dialog).getByRole("button", { name: /create project/i }));

    expect(await within(dialog).findByText("Project name already in use.")).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });
});
