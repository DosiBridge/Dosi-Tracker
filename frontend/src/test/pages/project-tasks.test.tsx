const params = vi.hoisted(() => ({ id: "p1" }));
vi.mock("next/navigation", async () => ({
  ...(await import("@/test/next-navigation-stub")),
  useParams: () => params,
}));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import ProjectDetailPage from "@/app/(dashboard)/projects/[id]/page";
import { SessionProvider } from "@/components/session-provider";
import { renderAsRole, resetPrototypeState } from "@/test/harness";

const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");
const WORKSPACE_ID = "99999999-9999-9999-9999-999999999999";
const OWNER = "11111111-1111-1111-1111-111111111111";
const CLIENT_USER = "33333333-3333-3333-3333-333333333333";
const PROJECT_A = "aaaa1111-1111-1111-1111-111111111111";
const PROJECT_B = "bbbb2222-2222-2222-2222-222222222222";

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
  params.id = PROJECT_A;
  localStorage.setItem("dosi-token", "jwt-live");
  localStorage.setItem("dosi-workspace", WORKSPACE_ID);
  fetchMock = vi.fn<typeof fetch>().mockImplementation(() => new Promise<Response>(() => {}));
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

function setupMockBackend(opts: {
  userId?: string;
  roles?: string[];
  projectArchived?: boolean;
  memberRole?: string;
  initialTasks?: Array<{ id: string; title: string; description?: string | null; isCompleted: boolean }>;
  totalCount?: number;
} = {}) {
  const userId = opts.userId ?? OWNER;
  const roles = opts.roles ?? ["admin"];
  const projectArchived = opts.projectArchived ?? false;
  const memberRole = opts.memberRole ?? (roles.includes("admin") ? "Admin" : "Worker");

  const tasks = (opts.initialTasks ?? [
    { id: "task-1", title: "Setup auth", description: "Integrate OAuth", isCompleted: false },
    { id: "task-2", title: "Write tests", description: null, isCompleted: true },
  ]).map((t) => ({
    ...t,
    workspaceId: WORKSPACE_ID,
    projectId: PROJECT_A,
    creationTime: "2026-10-24T00:00:00Z",
  }));

  fetchMock.mockImplementation(async (input, init) => {
    const url = String(input).replace(API, "");
    const method = init?.method ?? "GET";

    if (url.startsWith("/api/abp/application-configuration")) {
      return json({
        currentUser: {
          isAuthenticated: true,
          id: userId,
          tenantId: WORKSPACE_ID,
          userName: "testuser",
          name: "Test",
          surName: "User",
          roles,
        },
        currentTenant: { id: WORKSPACE_ID, name: "TestCo", isAvailable: true },
        auth: {
          grantedPolicies: roles.includes("admin")
            ? { "Tracker.Projects.Edit": true, "Tracker.Team.Manage": true }
            : {},
        },
      });
    }

    if (url.startsWith("/api/app/project/") && method === "GET") {
      const pid = url.split("?")[0].replace("/api/app/project/", "");
      return json({
        id: pid,
        title: pid === PROJECT_B ? "Project Beta" : "Apollo",
        description: "Task management test project",
        color: "#0284c7",
        intervalMinutes: 10,
        isArchived: projectArchived,
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

    if (url.startsWith("/api/app/team/project-members/")) {
      return json({
        items: [
          {
            id: "m-user",
            projectId: PROJECT_A,
            userId,
            role: memberRole,
            hourlyRate: 25,
            userName: "testuser",
            name: "Test",
            surname: "User",
          },
        ],
      });
    }

    if (url.startsWith("/api/app/team/members")) {
      return json({ items: [] });
    }

    if (url.startsWith("/api/app/reporting/summary")) {
      return json({ totalActivities: 0, totalTrackedMinutes: 0, perUser: [], perProject: [] });
    }

    if (url.startsWith("/api/app/activity")) {
      return json({ items: [], totalCount: 0 });
    }

    // Project Task endpoints
    if (url.startsWith("/api/app/project-task") && method === "GET") {
      const parsedUrl = new URL(String(input), "https://localhost");
      const pid = parsedUrl.searchParams.get("ProjectId");
      const isCompleted = parsedUrl.searchParams.get("IsCompleted");
      const skipCount = Number(parsedUrl.searchParams.get("SkipCount") || "0");
      const maxResultCount = Number(parsedUrl.searchParams.get("MaxResultCount") || "10");

      let filtered = tasks.filter((t) => t.projectId === pid);
      if (isCompleted === "true") filtered = filtered.filter((t) => t.isCompleted);
      if (isCompleted === "false") filtered = filtered.filter((t) => !t.isCompleted);

      const pageItems = filtered.slice(skipCount, skipCount + maxResultCount);
      return json({
        items: pageItems,
        totalCount: opts.totalCount ?? filtered.length,
      });
    }

    if (url === "/api/app/project-task" && method === "POST") {
      const body = JSON.parse(String(init?.body));
      const created = {
        id: `task-${Date.now()}`,
        workspaceId: WORKSPACE_ID,
        projectId: body.projectId,
        title: body.title,
        description: body.description ?? null,
        isCompleted: body.isCompleted ?? false,
        creationTime: new Date().toISOString(),
      };
      tasks.push(created);
      return json(created);
    }

    if (url.startsWith("/api/app/project-task/") && method === "PUT") {
      const id = url.replace("/api/app/project-task/", "");
      const body = JSON.parse(String(init?.body));
      const existing = tasks.find((t) => t.id === id);
      if (!existing) return json({ error: { message: "Task not found" } }, 404);
      existing.title = body.title;
      existing.description = body.description ?? null;
      existing.isCompleted = body.isCompleted;
      return json(existing);
    }

    return json({ items: [] });
  });

  return {
    getTasks: () => tasks,
  };
}

describe("Project Tasks Section", () => {
  it("fetches scoped task list with ProjectId, pagination and X-Workspace-Id header", async () => {
    setupMockBackend();
    renderAsRole(<ProjectDetailPage />, "owner");

    expect(await screen.findByRole("heading", { name: "Tasks" })).toBeInTheDocument();
    expect(await screen.findByText("Setup auth")).toBeInTheDocument();
    expect(screen.getByText("Integrate OAuth")).toBeInTheDocument();
    expect(screen.getByText("Write tests")).toBeInTheDocument();

    const taskCalls = fetchMock.mock.calls.filter(([u, i]) =>
      String(u).includes("/api/app/project-task") && (i?.method ?? "GET") === "GET"
    );
    expect(taskCalls.length).toBeGreaterThan(0);
    const [firstCallUrl, firstCallInit] = taskCalls[0];
    expect(String(firstCallUrl)).toContain(`ProjectId=${PROJECT_A}`);
    expect(String(firstCallUrl)).toContain("SkipCount=0");
    expect(String(firstCallUrl)).toContain("MaxResultCount=10");
    const headers = firstCallInit?.headers as Record<string, string>;
    expect(headers["X-Workspace-Id"]).toBe(WORKSPACE_ID);
  });

  it("filters tasks by open/completed and resets pagination", async () => {
    setupMockBackend();
    const user = userEvent.setup();
    renderAsRole(<ProjectDetailPage />, "owner");

    expect(await screen.findByText("Setup auth")).toBeInTheDocument();
    expect(screen.getByText("Write tests")).toBeInTheDocument();

    // Switch to Open filter
    await user.click(screen.getByRole("tab", { name: "Open" }));
    await waitFor(() => {
      const openCalls = fetchMock.mock.calls.filter(([u]) =>
        String(u).includes("/api/app/project-task") && String(u).includes("IsCompleted=false")
      );
      expect(openCalls.length).toBeGreaterThan(0);
      expect(String(openCalls[0][0])).toContain("SkipCount=0");
    });

    expect(screen.getByText("Setup auth")).toBeInTheDocument();
    expect(screen.queryByText("Write tests")).not.toBeInTheDocument();

    // Switch to Completed filter
    await user.click(screen.getByRole("tab", { name: "Completed" }));
    await waitFor(() => {
      const completedCalls = fetchMock.mock.calls.filter(([u]) =>
        String(u).includes("/api/app/project-task") && String(u).includes("IsCompleted=true")
      );
      expect(completedCalls.length).toBeGreaterThan(0);
      expect(String(completedCalls[0][0])).toContain("SkipCount=0");
    });

    expect(screen.getByText("Write tests")).toBeInTheDocument();
    expect(screen.queryByText("Setup auth")).not.toBeInTheDocument();
  });

  it("supports pagination controls and applies successive page items", async () => {
    const manyTasks = Array.from({ length: 15 }, (_, i) => ({
      id: `task-${i + 1}`,
      title: `Work item ${i + 1}`,
      description: `Description ${i + 1}`,
      isCompleted: false,
    }));
    setupMockBackend({ initialTasks: manyTasks });

    const user = userEvent.setup();
    renderAsRole(<ProjectDetailPage />, "owner");

    expect(await screen.findByText("Work item 1")).toBeInTheDocument();
    expect(screen.getByText("Work item 10")).toBeInTheDocument();
    expect(screen.queryByText("Work item 11")).not.toBeInTheDocument();
    expect(screen.getByText("Page 1 of 2")).toBeInTheDocument();

    // Click Next
    await user.click(screen.getByRole("button", { name: /next/i }));
    await waitFor(() => {
      const p2Calls = fetchMock.mock.calls.filter(([u]) =>
        String(u).includes("/api/app/project-task") && String(u).includes("SkipCount=10")
      );
      expect(p2Calls.length).toBeGreaterThan(0);
    });

    expect(await screen.findByText("Work item 11")).toBeInTheDocument();
    expect(screen.getByText("Work item 15")).toBeInTheDocument();
    expect(screen.queryByText("Work item 1")).not.toBeInTheDocument();
    expect(screen.getByText("Page 2 of 2")).toBeInTheDocument();

    // Previous button takes back to page 1
    await user.click(screen.getByRole("button", { name: /previous/i }));
    await waitFor(() => {
      expect(screen.getByText("Work item 1")).toBeInTheDocument();
      expect(screen.getByText("Page 1 of 2")).toBeInTheDocument();
    });
  });

  it("creates a task with title and description, validating required title and duplicate submit prevention", async () => {
    setupMockBackend();
    const user = userEvent.setup();
    renderAsRole(<ProjectDetailPage />, "owner", { withToasts: true });

    expect(await screen.findByText("Setup auth")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /add task/i }));
    const modal = screen.getByRole("dialog", { name: "New task" });
    const titleInput = within(modal).getByLabelText(/^title$/i);
    const descInput = within(modal).getByLabelText(/description/i);

    // Empty title validation
    await user.click(within(modal).getByRole("button", { name: /save task/i }));
    expect(within(modal).getByRole("alert")).toHaveTextContent("The task needs a title.");

    // Fill valid data
    await user.type(titleInput, "Deploy to cluster");
    await user.type(descInput, "Configure ingress and TLS certs");
    await user.click(within(modal).getByRole("button", { name: /save task/i }));

    await waitFor(() => {
      const postCalls = fetchMock.mock.calls.filter(([u, i]) =>
        String(u).endsWith("/api/app/project-task") && (i?.method ?? "GET") === "POST"
      );
      expect(postCalls).toHaveLength(1);
      const body = JSON.parse(String(postCalls[0][1]?.body));
      expect(body).toEqual({
        projectId: PROJECT_A,
        title: "Deploy to cluster",
        description: "Configure ingress and TLS certs",
        isCompleted: false,
      });
    });

    expect(await screen.findByText("Deploy to cluster")).toBeInTheDocument();
    expect(screen.getByText("Configure ingress and TLS certs")).toBeInTheDocument();
  });

  it("edits task title and description while preserving completed status", async () => {
    setupMockBackend();
    const user = userEvent.setup();
    renderAsRole(<ProjectDetailPage />, "owner", { withToasts: true });

    expect(await screen.findByText("Setup auth")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /edit task "setup auth"/i }));
    const modal = screen.getByRole("dialog", { name: "Edit task" });
    const titleInput = within(modal).getByLabelText(/^title$/i);
    const descInput = within(modal).getByLabelText(/description/i);

    expect(titleInput).toHaveValue("Setup auth");
    expect(descInput).toHaveValue("Integrate OAuth");

    await user.clear(titleInput);
    await user.type(titleInput, "Setup OAuth & OIDC");
    await user.clear(descInput);
    await user.type(descInput, "PKCE flow verification");
    await user.click(within(modal).getByRole("button", { name: /save task/i }));

    await waitFor(() => {
      const putCalls = fetchMock.mock.calls.filter(([u, i]) =>
        String(u).includes("/api/app/project-task/task-1") && (i?.method ?? "GET") === "PUT"
      );
      expect(putCalls).toHaveLength(1);
      const body = JSON.parse(String(putCalls[0][1]?.body));
      expect(body).toEqual({
        title: "Setup OAuth & OIDC",
        description: "PKCE flow verification",
        isCompleted: false, // preserved!
      });
    });

    expect(await screen.findByText("Setup OAuth & OIDC")).toBeInTheDocument();
    expect(screen.getByText("PKCE flow verification")).toBeInTheDocument();
  });

  it("toggles task complete and reopen while preserving title and description", async () => {
    setupMockBackend();
    const user = userEvent.setup();
    renderAsRole(<ProjectDetailPage />, "owner", { withToasts: true });

    expect(await screen.findByText("Setup auth")).toBeInTheDocument();

    // Mark open task as completed
    await user.click(screen.getByRole("button", { name: /mark "setup auth" as completed/i }));

    await waitFor(() => {
      const putCalls = fetchMock.mock.calls.filter(([u, i]) =>
        String(u).includes("/api/app/project-task/task-1") && (i?.method ?? "GET") === "PUT"
      );
      expect(putCalls).toHaveLength(1);
      const body = JSON.parse(String(putCalls[0][1]?.body));
      expect(body).toEqual({
        title: "Setup auth",
        description: "Integrate OAuth",
        isCompleted: true,
      });
    });

    // Reopen completed task "Write tests"
    await user.click(screen.getByRole("button", { name: /mark "write tests" as open/i }));
    await waitFor(() => {
      const putCalls = fetchMock.mock.calls.filter(([u, i]) =>
        String(u).includes("/api/app/project-task/task-2") && (i?.method ?? "GET") === "PUT"
      );
      expect(putCalls).toHaveLength(1);
      const body = JSON.parse(String(putCalls[0][1]?.body));
      expect(body).toEqual({
        title: "Write tests",
        description: null,
        isCompleted: false,
      });
    });
  });

  it("client member sees NO misleading mutation controls and has disabled completion toggles", async () => {
    setupMockBackend({
      userId: CLIENT_USER,
      roles: ["client"],
      memberRole: "Client",
    });

    renderAsRole(<ProjectDetailPage />, "client");

    expect(await screen.findByRole("heading", { name: "Tasks" })).toBeInTheDocument();
    expect(await screen.findByText("Setup auth")).toBeInTheDocument();

    // No Add task button
    expect(screen.queryByRole("button", { name: /add task/i })).not.toBeInTheDocument();

    // No Edit buttons
    expect(screen.queryByRole("button", { name: /edit task/i })).not.toBeInTheDocument();

    // Toggle button is disabled
    const toggleBtn = screen.getByRole("button", { name: /mark "setup auth" as completed/i });
    expect(toggleBtn).toBeDisabled();
  });

  it("archived project forbids mutation controls and disables completion toggles", async () => {
    setupMockBackend({ projectArchived: true });

    renderAsRole(<ProjectDetailPage />, "owner");

    expect(await screen.findByRole("heading", { name: "Tasks" })).toBeInTheDocument();
    expect(await screen.findByText("Setup auth")).toBeInTheDocument();

    // No Add task button
    expect(screen.queryByRole("button", { name: /add task/i })).not.toBeInTheDocument();

    // No Edit buttons
    expect(screen.queryByRole("button", { name: /edit task/i })).not.toBeInTheDocument();

    // Toggle button is disabled
    const toggleBtn = screen.getByRole("button", { name: /mark "setup auth" as completed/i });
    expect(toggleBtn).toBeDisabled();
  });

  it("mutation rejection (e.g. 403 Forbidden) preserves state and displays error", async () => {
    setupMockBackend();
    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input).replace(API, "");
      const method = init?.method ?? "GET";
      if (url.startsWith("/api/app/project-task") && method === "POST") {
        return json({ error: { message: "Permission denied", code: "Tracker:RoleNotAllowed" } }, 403);
      }
      if (url.startsWith("/api/abp/application-configuration")) {
        return json({
          currentUser: { id: OWNER, roles: ["admin"], isAuthenticated: true },
          currentTenant: { id: WORKSPACE_ID },
          auth: { grantedPolicies: { "Tracker.Projects.Edit": true } },
        });
      }
      if (url.startsWith("/api/app/project/")) {
        return json({ id: PROJECT_A, title: "Apollo", isArchived: false, color: "#0284c7", intervalMinutes: 10 });
      }
      if (url.startsWith("/api/app/project-task") && method === "GET") {
        return json({
          items: [{ id: "task-1", title: "Setup auth", description: null, isCompleted: false }],
          totalCount: 1,
        });
      }
      return json({ items: [] });
    });

    const user = userEvent.setup();
    renderAsRole(<ProjectDetailPage />, "owner", { withToasts: true });

    expect(await screen.findByText("Setup auth")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /add task/i }));
    const modal = screen.getByRole("dialog", { name: "New task" });
    await user.type(within(modal).getByLabelText(/^title$/i), "Unauthorized Task");
    await user.click(within(modal).getByRole("button", { name: /save task/i }));

    expect(await within(modal).findByRole("alert")).toBeInTheDocument();
    // Modal stays open and existing task list remains intact
    expect(screen.getByText("Setup auth")).toBeInTheDocument();
  });

  it("mutation success then refresh rejection: reconciles DTO locally, shows retry alert, and explicit retry issues ONLY GET", async () => {
    let taskGetCount = 0;
    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input).replace(API, "");
      const method = init?.method ?? "GET";
      if (url.startsWith("/api/abp/application-configuration")) {
        return json({
          currentUser: { id: OWNER, roles: ["admin"], isAuthenticated: true },
          currentTenant: { id: WORKSPACE_ID },
          auth: { grantedPolicies: { "Tracker.Projects.Edit": true } },
        });
      }
      if (url.startsWith("/api/app/project/")) {
        return json({ id: PROJECT_A, title: "Apollo", isArchived: false, color: "#0284c7", intervalMinutes: 10 });
      }
      if (url.startsWith("/api/app/project-task") && method === "GET") {
        taskGetCount++;
        if (taskGetCount === 1) {
          return json({
            items: [{ id: "task-1", title: "Setup auth", description: "OAuth", isCompleted: false }],
            totalCount: 1,
          });
        }
        if (taskGetCount === 2) {
          // Refresh call after toggle rejects with 500
          return json({ error: { message: "Internal server error" } }, 500);
        }
        // Explicit retry (taskGetCount === 3) succeeds
        return json({
          items: [{ id: "task-1", title: "Setup auth", description: "OAuth", isCompleted: true }],
          totalCount: 1,
        });
      }
      if (url.startsWith("/api/app/project-task/task-1") && method === "PUT") {
        return json({ id: "task-1", title: "Setup auth", description: "OAuth", isCompleted: true });
      }
      return json({ items: [] });
    });

    const user = userEvent.setup();
    renderAsRole(<ProjectDetailPage />, "owner", { withToasts: true });

    expect(await screen.findByText("Setup auth")).toBeInTheDocument();

    // Toggle complete
    await user.click(screen.getByRole("button", { name: /mark "setup auth" as completed/i }));

    // Reconciles DTO locally immediately
    const list = screen.getByRole("list", { name: "Project tasks" });
    await waitFor(() => {
      const setupLi = within(list).getByText("Setup auth").closest("li")!;
      expect(within(setupLi).getByText("Completed")).toBeInTheDocument();
    });

    // Refresh alert appears with retry button
    const retryBtn = await screen.findByRole("button", { name: /retry/i });
    expect(screen.getByRole("alert")).toHaveTextContent(/couldn't refresh/i);

    // Clicking retry sends ONLY GET /api/app/project-task
    await user.click(retryBtn);

    await waitFor(() => {
      expect(taskGetCount).toBe(3);
    });

    const putCalls = fetchMock.mock.calls.filter(([, i]) => i?.method === "PUT");
    expect(putCalls).toHaveLength(1); // exactly the initial PUT, zero repeated PUTs!
  });

  it("scope switch: switching projects discards stale in-flight responses and resets state", async () => {
    let resolveProjectA: ((res: Response) => void) | null = null;
    fetchMock.mockImplementation(async (input) => {
      const url = String(input).replace(API, "");
      if (url.startsWith("/api/abp/application-configuration")) {
        return json({
          currentUser: { id: OWNER, roles: ["admin"], isAuthenticated: true },
          currentTenant: { id: WORKSPACE_ID },
          auth: { grantedPolicies: { "Tracker.Projects.Edit": true } },
        });
      }
      if (url.startsWith("/api/app/project/")) {
        const pid = url.split("?")[0].replace("/api/app/project/", "");
        return json({
          id: pid,
          title: pid === PROJECT_B ? "Project Beta" : "Apollo",
          isArchived: false,
          color: "#0284c7",
          intervalMinutes: 10,
        });
      }
      if (url.includes(`ProjectId=${PROJECT_A}`)) {
        return new Promise<Response>((resolve) => {
          resolveProjectA = resolve;
        });
      }
      if (url.includes(`ProjectId=${PROJECT_B}`)) {
        return json({
          items: [{ id: "task-b1", title: "Beta specific task", description: null, isCompleted: false }],
          totalCount: 1,
        });
      }
      return json({ items: [] });
    });

    const { rerender } = renderAsRole(<ProjectDetailPage />, "owner");

    // Before Project A settles, switch to Project B
    params.id = PROJECT_B;
    rerender(
      <SessionProvider>
        <ProjectDetailPage />
      </SessionProvider>
    );

    // Project B renders its own task
    expect(await screen.findByText("Beta specific task")).toBeInTheDocument();

    // Now resolve late Project A response
    if (typeof resolveProjectA === "function") {
      (resolveProjectA as (res: Response) => void)(
        json({
          items: [{ id: "task-a-late", title: "Late Project A Task", isCompleted: false }],
          totalCount: 1,
        })
      );
    }

    // Late response from Project A must NOT contaminate Project B
    expect(screen.queryByText("Late Project A Task")).not.toBeInTheDocument();
    expect(screen.getByText("Beta specific task")).toBeInTheDocument();
  });

  it("demo mode dispatches ZERO requests to /api/app/project-task", async () => {
    params.id = "p1";
    localStorage.removeItem("dosi-token");

    renderAsRole(<ProjectDetailPage />, "owner");

    expect(await screen.findByRole("heading", { name: "Tasks" })).toBeInTheDocument();
    expect(screen.getByText(/no tasks in demo mode/i)).toBeInTheDocument();

    const taskCalls = fetchMock.mock.calls.filter(([u]) => String(u).includes("/api/app/project-task"));
    expect(taskCalls).toHaveLength(0);
  });
});
