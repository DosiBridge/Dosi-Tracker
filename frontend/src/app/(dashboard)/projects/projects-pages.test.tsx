// The project detail page in demo and live mode: editing every tracking
// setting (PUT /api/app/project/{id}), archiving only after a confirmation,
// adding and removing members, totals from the reporting API (never from a
// capped page of rows), and what a worker sees of their teammates (names, not
// their time). Only global fetch and next/navigation are stubbed.
const params = vi.hoisted(() => ({ id: "p1" }));
vi.mock("next/navigation", async () => ({
  ...(await import("@/test/next-navigation-stub")),
  useParams: () => params,
}));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import ProjectDetailPage from "@/app/(dashboard)/projects/[id]/page";
import { renderAsRole, resetPrototypeState } from "@/test/harness";

const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");
const OWNER = "11111111-1111-1111-1111-111111111111";
const SAM = "22222222-2222-2222-2222-222222222222";
const KIM = "44444444-4444-4444-4444-444444444444";
const PROJECT = "33333333-3333-3333-3333-333333333333";

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

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

beforeEach(() => {
  resetPrototypeState();
  params.id = "p1";
  fetchMock = vi.fn<typeof fetch>().mockImplementation(() => new Promise<Response>(() => {}));
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

function calls(path: string, method: string) {
  return fetchMock.mock.calls.filter(([u, i]) => String(u).replace(API, "").startsWith(path) && (i?.method ?? "GET") === method);
}

/** A live tenant with one project; `roles: ["admin"]` signs in its owner, [] a worker. */
function liveBackend(opts: { roles?: string[]; grants?: Record<string, boolean> } = {}) {
  const roles = opts.roles ?? ["admin"];
  const owner = roles.includes("admin");
  const me = owner ? OWNER : SAM;
  let archived = false;
  fetchMock.mockImplementation(async (input, init) => {
    const url = String(input).replace(API, "");
    const method = init?.method ?? "GET";
    if (url.startsWith("/api/abp/application-configuration")) {
      return json({
        currentUser: { isAuthenticated: true, id: me, tenantId: "t-1", userName: owner ? "mira" : "sam", name: owner ? "Mira" : "Sam", surName: owner ? "Okafor" : "Lee", roles },
        currentTenant: { id: "t-1", name: "RealCo", isAvailable: true },
        auth: { grantedPolicies: opts.grants ?? (owner ? { "Tracker.Team.Manage": true, "Tracker.Activities.ViewAll": true } : {}) },
      });
    }
    if (url === `/api/app/project/${PROJECT}` && method === "GET") {
      return json({
        id: PROJECT,
        title: "Apollo",
        description: "Moon shot",
        color: "#0284c7",
        intervalMinutes: 10,
        isArchived: archived,
        allowScreenshot: true,
        allowWebcam: false,
        allowKeyboard: true,
        allowMouse: true,
        allowActiveWindow: true,
        allowRunningPrograms: true,
        idleThresholdMinutes: 5,
        blurScreenshots: false,
        creationTime: "2026-03-01T09:00:00Z",
      });
    }
    if (url === `/api/app/project/${PROJECT}` && method === "PUT") return json({ id: PROJECT, ...JSON.parse(String(init?.body)) });
    if (url === `/api/app/project/${PROJECT}/archive` && method === "POST") {
      archived = true;
      return json(null, 204);
    }
    if (url.startsWith(`/api/app/team/project-members/${PROJECT}`)) {
      return json({
        items: [
          { id: "m-mira", projectId: PROJECT, userId: OWNER, role: "Admin", hourlyRate: 0, userName: "mira", name: "Mira", surname: "Okafor" },
          { id: "m-sam", projectId: PROJECT, userId: SAM, role: "Worker", hourlyRate: 20, userName: "sam", name: "Sam", surname: "Lee" },
          { id: "m-kim", projectId: PROJECT, userId: KIM, role: "Worker", hourlyRate: 22, userName: "kim", name: "Kim", surname: "Ito" },
        ],
      });
    }
    if (url.startsWith("/api/app/team/member/") && method === "DELETE") return json(null, 204);
    if (url.startsWith("/api/app/team/member") && method === "POST") return json({ id: "m-new" });
    if (url.startsWith("/api/app/team/members")) {
      const everyone = [
        { userId: OWNER, userName: "mira", name: "Mira", surname: "Okafor", email: "mira@realco.test", isOwner: true, isManager: true, projectIds: [PROJECT] },
        { userId: SAM, userName: "sam", name: "Sam", surname: "Lee", email: "sam@realco.test", isOwner: false, isManager: false, projectIds: [PROJECT] },
        { userId: KIM, userName: "kim", name: "Kim", surname: "Ito", email: "kim@realco.test", isOwner: false, isManager: false, projectIds: [PROJECT] },
        { userId: "55555555-5555-5555-5555-555555555555", userName: "ana", name: "Ana", surname: "Silva", email: "ana@realco.test", isOwner: false, isManager: false, projectIds: [] },
      ];
      // A worker's roster holds only themselves (the server scopes it).
      return json({ items: owner ? everyone : everyone.filter((m) => m.userId === me) });
    }
    if (url.startsWith("/api/app/reporting/summary")) {
      return json({
        totalActivities: 12,
        totalTrackedMinutes: url.includes("ProjectId") ? 600 : 0,
        averageProductivity: 70,
        perUser: [
          { userId: SAM, activityCount: 6, trackedMinutes: 360, averageProductivity: 81 },
          { userId: KIM, activityCount: 4, trackedMinutes: 240, averageProductivity: null },
        ],
        perProject: [{ projectId: PROJECT, activityCount: 12, trackedMinutes: 600, averageProductivity: 70 }],
      });
    }
    if (url.startsWith("/api/app/activity")) return json({ items: [], totalCount: 0 });
    if (url.startsWith("/api/app/project")) return json({ items: [{ id: PROJECT, title: "Apollo", isArchived: archived }] });
    if (url.startsWith("/api/app/workspace/current-subscription")) return json({ planId: "plan-s", status: "active" });
    if (url.startsWith("/api/app/workspace/available-plans")) return json({ items: [{ id: "plan-s", name: "Starter", pricePerUser: 6, maxSeats: 25 }] });
    if (url.startsWith("/api/app/workspace/")) return json({}, 404);
    if (url.startsWith("/api/app/notification/")) return json({ items: [] });
    throw new Error(`unexpected fetch: ${method} ${url}`);
  });
}

describe("project page — demo mode", () => {
  it("shows every tracking setting, idle detection and blur included", async () => {
    renderAsRole(<ProjectDetailPage />, "owner");

    expect(await screen.findByRole("heading", { name: "Dosi Web Platform" })).toBeInTheDocument();
    const settings = screen.getByRole("heading", { name: "Tracking settings" }).closest("div.rounded-2xl")! as HTMLElement;
    expect(within(settings).getByText("Block length")).toBeInTheDocument();
    expect(within(settings).getByText("10 min")).toBeInTheDocument();
    expect(within(settings).getByText("Idle detection")).toBeInTheDocument();
    expect(within(settings).getByText("Blur screenshots")).toBeInTheDocument();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("edits the project's tracking settings and shows them at once", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectDetailPage />, "owner", { withToasts: true });

    await user.click(await screen.findByRole("button", { name: /edit project/i }));
    const dialog = screen.getByRole("dialog", { name: "Edit project" });
    expect(within(dialog).getByLabelText(/project name/i)).toHaveValue("Dosi Web Platform");
    await user.click(within(dialog).getByRole("tab", { name: "Tracking" }));
    await user.click(within(dialog).getByRole("radio", { name: "30 min" }));
    await user.click(within(dialog).getByRole("switch", { name: /blur screenshots/i }));
    await user.click(within(dialog).getByRole("button", { name: /save changes/i }));

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(await screen.findByText("30 min")).toBeInTheDocument();
    expect(screen.getByText(/demo workspace — this change lasts until you reload/i)).toBeInTheDocument();
  });

  it("archives only after a confirmation that says what stops", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectDetailPage />, "owner", { withToasts: true });

    await user.click(await screen.findByRole("button", { name: /^archive$/i }));
    const confirm = screen.getByRole("dialog", { name: /archive dosi web platform\?/i });
    expect(within(confirm).getByText(/can no longer track time on it/i)).toBeInTheDocument();
    await user.click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByText("Archived")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /^archive$/i }));
    await user.click(within(screen.getByRole("dialog", { name: /archive dosi web platform\?/i })).getByRole("button", { name: "Archive project" }));
    expect(await screen.findByText("Archived")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /unarchive/i })).toBeInTheDocument();
  });

  it("a worker sees their own time and teammates by name — not the team's hours or rings", async () => {
    renderAsRole(<ProjectDetailPage />, "worker");

    expect(await screen.findByText("Your time this week")).toBeInTheDocument();
    expect(screen.getByText("Your total")).toBeInTheDocument();
    const team = screen.getByRole("heading", { name: "Team members" }).closest("div.rounded-2xl")! as HTMLElement;
    expect(within(team).getByText("Ayesha Rahman")).toBeInTheDocument();
    expect(within(team).queryByText("this month")).not.toBeInTheDocument();
    expect(within(team).queryByRole("img", { name: /activity/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /edit project/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^archive$/i })).not.toBeInTheDocument();
  });

  it("a11y: the project page has no axe violations", async () => {
    const { container } = renderAsRole(<ProjectDetailPage />, "owner");
    await screen.findByRole("heading", { name: "Dosi Web Platform" });
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("project page — live mode", () => {
  beforeEach(() => {
    params.id = PROJECT;
    localStorage.setItem("dosi-token", "jwt-live");
  });

  it("takes this week, this month and the total from the reporting API, scoped to the project and the reporting zone", async () => {
    liveBackend();
    renderAsRole(<ProjectDetailPage />, "owner");

    expect(await screen.findByRole("heading", { name: "Apollo" })).toBeInTheDocument();
    // This page's own figures: three summaries, each scoped to the project and the reporting zone.
    const projectSummaries = () => calls("/api/app/reporting/summary", "GET").filter(([u]) => String(u).includes(`ProjectId=${PROJECT}`));
    await waitFor(() => expect(projectSummaries().length).toBeGreaterThanOrEqual(3));
    for (const [url] of projectSummaries()) expect(String(url)).toContain("TimeZone=");
    expect(await screen.findAllByText("10h")).toHaveLength(3); // week, month and total
    expect(screen.getByText("Since 1 Mar 2026")).toBeInTheDocument();
    // Per-member time and activity for people who see everyone's time.
    const team = screen.getByRole("heading", { name: "Team members" }).closest("div.rounded-2xl")! as HTMLElement;
    expect(within(team).getAllByText("6h").length).toBeGreaterThan(0);
    expect(within(team).getByRole("img", { name: "Activity 81%" })).toBeInTheDocument();
    // Kim's blocks weren't measured (input capture off) and Mira tracked nothing: no percentage is invented.
    expect(within(team).getAllByRole("img", { name: "Activity not measured" })).toHaveLength(2);
    // This page reads only the newest few blocks of the project (for "Recent activity"); totals never come from rows.
    const projectRows = calls("/api/app/activity", "GET").filter(([u]) => String(u).includes(`ProjectId=${PROJECT}`));
    expect(projectRows.length).toBeGreaterThan(0);
    expect(projectRows.every(([u]) => String(u).includes("MaxResultCount=8"))).toBe(true);
  });

  it("saves every tracking setting with PUT /api/app/project/{id}", async () => {
    liveBackend();
    const user = userEvent.setup();
    renderAsRole(<ProjectDetailPage />, "owner", { withToasts: true });

    await user.click(await screen.findByRole("button", { name: /edit project/i }));
    const dialog = screen.getByRole("dialog", { name: "Edit project" });
    const title = within(dialog).getByLabelText(/project name/i);
    await user.clear(title);
    await user.type(title, "  Apollo II  ");
    await user.click(within(dialog).getByRole("tab", { name: "Tracking" }));
    await user.click(within(dialog).getByRole("radio", { name: "15 min" }));
    await user.selectOptions(within(dialog).getByLabelText(/idle detection/i), "10");
    await user.click(within(dialog).getByRole("switch", { name: /blur screenshots/i }));
    await user.click(within(dialog).getByRole("switch", { name: /^webcam/i }));
    await user.click(within(dialog).getByRole("button", { name: /save changes/i }));

    await waitFor(() => expect(calls(`/api/app/project/${PROJECT}`, "PUT")).toHaveLength(1));
    expect(JSON.parse(String(calls(`/api/app/project/${PROJECT}`, "PUT")[0][1]?.body))).toEqual({
      title: "Apollo II",
      description: "Moon shot",
      color: "#0284c7",
      intervalMinutes: 15,
      allowScreenshot: true,
      allowWebcam: true,
      allowKeyboard: true,
      allowMouse: true,
      allowActiveWindow: true,
      allowRunningPrograms: true,
      idleThresholdMinutes: 10,
      blurScreenshots: true,
    });
    expect(await screen.findByText("Project updated")).toBeInTheDocument();
    expect(await screen.findByRole("heading", { name: "Apollo II" })).toBeInTheDocument();
  });

  it("archives after confirming, through the project's archive action", async () => {
    liveBackend();
    const user = userEvent.setup();
    renderAsRole(<ProjectDetailPage />, "owner", { withToasts: true });

    await user.click(await screen.findByRole("button", { name: /^archive$/i }));
    expect(calls(`/api/app/project/${PROJECT}/archive`, "POST")).toHaveLength(0);
    await user.click(within(screen.getByRole("dialog", { name: /archive apollo\?/i })).getByRole("button", { name: "Archive project" }));

    await waitFor(() => expect(calls(`/api/app/project/${PROJECT}/archive`, "POST")).toHaveLength(1));
    expect(await screen.findByText("Archived")).toBeInTheDocument();
  });

  it("adds a workspace member to the project and removes one after confirming", async () => {
    liveBackend();
    const user = userEvent.setup();
    renderAsRole(<ProjectDetailPage />, "owner", { withToasts: true });

    await user.click(await screen.findByRole("button", { name: /add member/i }));
    const add = screen.getByRole("dialog", { name: "Add to a project" });
    // Already on the project: not pickable.
    expect(within(add).getByRole("radio", { name: /sam lee/i })).toBeDisabled();
    await user.click(within(add).getByRole("radio", { name: /ana silva/i }));
    await user.click(within(add).getByRole("button", { name: /^add to project$/i }));
    await waitFor(() => expect(calls("/api/app/team/member", "POST")).toHaveLength(1));
    expect(JSON.parse(String(calls("/api/app/team/member", "POST")[0][1]?.body))).toMatchObject({
      projectId: PROJECT,
      userId: "55555555-5555-5555-5555-555555555555",
      role: "Worker",
    });

    await user.click(screen.getByRole("button", { name: "Remove Kim Ito from Apollo" }));
    const confirm = screen.getByRole("dialog", { name: /remove kim ito from apollo\?/i });
    expect(within(confirm).getByText(/seat is freed/i)).toBeInTheDocument();
    await user.click(within(confirm).getByRole("button", { name: "Remove from project" }));
    await waitFor(() => expect(calls("/api/app/team/member/m-kim", "DELETE")).toHaveLength(1));
  });

  it("a worker sees teammates by their real names (from the membership), with no time or status", async () => {
    liveBackend({ roles: [] });
    renderAsRole(<ProjectDetailPage />, "worker");

    expect(await screen.findByRole("heading", { name: "Apollo" })).toBeInTheDocument();
    const team = screen.getByRole("heading", { name: "Team members" }).closest("div.rounded-2xl")! as HTMLElement;
    expect(await within(team).findByText("Kim Ito")).toBeInTheDocument();
    expect(within(team).getByText("Mira Okafor")).toBeInTheDocument();
    expect(within(team).queryByText(/^Member [0-9a-f]{8}$/)).not.toBeInTheDocument();
    expect(within(team).queryByText("this month")).not.toBeInTheDocument();
    expect(screen.getByText("Your time this week")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /add member/i })).not.toBeInTheDocument();
  });
});
