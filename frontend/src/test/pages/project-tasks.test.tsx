const params = vi.hoisted(() => ({ id: "p1" }));
vi.mock("next/navigation", async () => ({
  ...(await import("@/test/next-navigation-stub")),
  useParams: () => params,
}));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import ProjectDetailPage from "@/app/(dashboard)/projects/[id]/page";
import { ProjectTasksSection } from "@/components/projects/project-tasks";
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

  it("workspace change with same projectId discards in-flight list and isolates state", async () => {
    const WS_A = "11111111-1111-1111-1111-111111111111";
    const WS_B = "22222222-2222-2222-2222-222222222222";
    let resolveWsA: ((res: Response) => void) | null = null;

    localStorage.setItem("dosi-workspace", WS_A);

    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input);
      const wsHeader = (init?.headers as Record<string, string>)?.[ "X-Workspace-Id" ];

      if (url.includes("/api/app/project-task")) {
        if (wsHeader === WS_A) {
          return new Promise<Response>((resolve) => {
            resolveWsA = resolve;
          });
        }
        if (wsHeader === WS_B) {
          return json({
            items: [{ id: "task-ws-b", title: "Workspace B Task", isCompleted: false }],
            totalCount: 1,
          });
        }
      }
      return json({ items: [], totalCount: 0 });
    });

    const { rerender } = render(
      <ProjectTasksSection
        projectId="p-shared"
        workspaceId={WS_A}
        projectArchived={false}
        isLive={true}
        canMutate={true}
      />
    );

    // Switch to Workspace B with the SAME projectId
    localStorage.setItem("dosi-workspace", WS_B);
    rerender(
      <ProjectTasksSection
        projectId="p-shared"
        workspaceId={WS_B}
        projectArchived={false}
        isLive={true}
        canMutate={true}
      />
    );

    expect(await screen.findByText("Workspace B Task")).toBeInTheDocument();

    // Resolve delayed Workspace A response
    if (resolveWsA) {
      (resolveWsA as (res: Response) => void)(
        json({
          items: [{ id: "task-ws-a", title: "Workspace A Leaked Task", isCompleted: false }],
          totalCount: 1,
        })
      );
    }

    // Workspace A must NOT contaminate Workspace B
    expect(screen.queryByText("Workspace A Leaked Task")).not.toBeInTheDocument();
    expect(screen.getByText("Workspace B Task")).toBeInTheDocument();
  });

  it("post-mutation refresh arriving after filter change does not overwrite new filter results", async () => {
    let resolveBackgroundRefresh: ((res: Response) => void) | null = null;
    let refreshCount = 0;

    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input);
      const method = init?.method ?? "GET";

      if (url.includes("/api/app/project-task")) {
        if (method === "PUT") {
          return json({
            id: "t1",
            projectId: PROJECT_A,
            title: "Initial Task",
            isCompleted: true,
          });
        }
        if (method === "GET") {
          refreshCount++;
          // Initial load for "all"
          if (refreshCount === 1) {
            return json({
              items: [{ id: "t1", title: "Initial Task", isCompleted: false }],
              totalCount: 1,
            });
          }
          // Background refresh triggered by toggle
          if (refreshCount === 2) {
            return new Promise<Response>((resolve) => {
              resolveBackgroundRefresh = resolve;
            });
          }
          // Load triggered by switching to "completed" tab
          if (url.includes("IsCompleted=true")) {
            return json({
              items: [{ id: "completed-fresh", title: "Completed Filter Task", isCompleted: true }],
              totalCount: 1,
            });
          }
        }
      }
      return json({ items: [], totalCount: 0 });
    });

    const user = userEvent.setup();
    render(
      <ProjectTasksSection
        projectId={PROJECT_A}
        workspaceId={WORKSPACE_ID}
        projectArchived={false}
        isLive={true}
        canMutate={true}
      />
    );

    expect(await screen.findByText("Initial Task")).toBeInTheDocument();

    // Toggle completion -> fires PUT, then fires background refresh GET (refreshCount = 2, held pending)
    const toggleBtn = screen.getByRole("button", { name: /mark "initial task" as completed/i });
    await user.click(toggleBtn);

    // While background refresh is pending, switch filter tab to "Completed"
    const completedTab = screen.getByRole("tab", { name: "Completed" });
    await user.click(completedTab);

    // "Completed" tab loads its fresh items
    expect(await screen.findByText("Completed Filter Task")).toBeInTheDocument();

    // Now resolve the late background refresh from the earlier mutation (which was for filter="all")
    if (resolveBackgroundRefresh) {
      (resolveBackgroundRefresh as (res: Response) => void)(
        json({
          items: [{ id: "stale-all-item", title: "Stale All Item", isCompleted: true }],
          totalCount: 1,
        })
      );
    }

    // Stale background refresh must NOT overwrite the completed tab items
    expect(screen.queryByText("Stale All Item")).not.toBeInTheDocument();
    expect(screen.getByText("Completed Filter Task")).toBeInTheDocument();
  });

  it("GET-only retry resolving after scope change is discarded without repeat mutation", async () => {
    let resolveRetry: ((res: Response) => void) | null = null;
    let getCallCount = 0;

    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input);
      const method = init?.method ?? "GET";

      if (url.includes("/api/app/project-task")) {
        if (method === "GET") {
          getCallCount++;
          // Initial load
          if (getCallCount === 1) {
            return json({ items: [{ id: "t1", title: "Task 1", isCompleted: false }], totalCount: 1 });
          }
          // Background refresh fails after toggle -> causes retry warning
          if (getCallCount === 2) {
            return json({ message: "Network timeout" }, 500);
          }
          // Explicit Retry click -> held on barrier
          if (getCallCount === 3) {
            return new Promise<Response>((resolve) => {
              resolveRetry = resolve;
            });
          }
          // New scope load
          return json({ items: [{ id: "t2", title: "Project B Task", isCompleted: false }], totalCount: 1 });
        }
        if (method === "PUT") {
          return json({ id: "t1", title: "Task 1", isCompleted: true });
        }
      }
      return json({ items: [], totalCount: 0 });
    });

    const user = userEvent.setup();
    const { rerender } = render(
      <ProjectTasksSection
        projectId="p1"
        workspaceId={WORKSPACE_ID}
        projectArchived={false}
        isLive={true}
        canMutate={true}
      />
    );

    expect(await screen.findByText("Task 1")).toBeInTheDocument();

    // Toggle -> PUT succeeds, background GET fails -> alert warning appears
    await user.click(screen.getByRole("button", { name: /mark "task 1" as completed/i }));
    const retryBtn = await screen.findByRole("button", { name: /retry/i });

    // Click Retry -> getCallCount = 3, held pending
    await user.click(retryBtn);

    // Before retry resolves, change scope to project "p2"
    rerender(
      <ProjectTasksSection
        projectId="p2"
        workspaceId={WORKSPACE_ID}
        projectArchived={false}
        isLive={true}
        canMutate={true}
      />
    );

    expect(await screen.findByText("Project B Task")).toBeInTheDocument();

    // Resolve late retry GET
    if (resolveRetry) {
      (resolveRetry as (res: Response) => void)(
        json({
          items: [{ id: "t1-late-retry", title: "Late Retry Task", isCompleted: true }],
          totalCount: 1,
        })
      );
    }

    // Late retry must NOT overwrite Project B's task list
    expect(screen.queryByText("Late Retry Task")).not.toBeInTheDocument();
    expect(screen.getByText("Project B Task")).toBeInTheDocument();

    // Verify zero repeated PUTs
    const putCalls = fetchMock.mock.calls.filter(([, i]) => i?.method === "PUT");
    expect(putCalls).toHaveLength(1);
  });

  it("pending mutation settlement after scope change does not leak into new scope", async () => {
    let resolveCreate: ((res: Response) => void) | null = null;

    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input);
      const method = init?.method ?? "GET";

      if (url.includes("/api/app/project-task")) {
        if (method === "POST") {
          return new Promise<Response>((resolve) => {
            resolveCreate = resolve;
          });
        }
        if (url.includes("ProjectId=p2")) {
          return json({ items: [{ id: "p2-task", title: "P2 Existing", isCompleted: false }], totalCount: 1 });
        }
      }
      return json({ items: [], totalCount: 0 });
    });

    const user = userEvent.setup();
    const { rerender } = render(
      <ProjectTasksSection
        projectId="p1"
        workspaceId={WORKSPACE_ID}
        projectArchived={false}
        isLive={true}
        canMutate={true}
      />
    );

    // Open create modal
    const addBtn = await screen.findByRole("button", { name: /add task/i });
    await user.click(addBtn);

    // Fill in title
    await user.type(screen.getByLabelText(/^title/i), "Orphaned Task");
    await user.click(screen.getByRole("button", { name: /save task/i }));

    // POST is in flight; now scope changes to "p2"
    rerender(
      <ProjectTasksSection
        projectId="p2"
        workspaceId={WORKSPACE_ID}
        projectArchived={false}
        isLive={true}
        canMutate={true}
      />
    );

    expect(await screen.findByText("P2 Existing")).toBeInTheDocument();

    // Now resolve late POST
    if (resolveCreate) {
      (resolveCreate as (res: Response) => void)(
        json({
          id: "created-p1",
          projectId: "p1",
          title: "Orphaned Task",
          isCompleted: false,
        })
      );
    }

    // Must not insert into p2 or increment p2 totalCount
    expect(screen.queryByText("Orphaned Task")).not.toBeInTheDocument();
    expect(screen.getByText("P2 Existing")).toBeInTheDocument();
    expect(screen.getByText("1 task")).toBeInTheDocument();
  });

  it("repeated same-query requests reject older in-flight responses", async () => {
    let resolveQuery1: ((res: Response) => void) | null = null;
    let resolveQueryFinal: ((res: Response) => void) | null = null;
    let queryCount = 0;

    fetchMock.mockImplementation(async (input) => {
      const url = String(input);
      if (url.includes("/api/app/project-task")) {
        queryCount++;
        if (queryCount === 1) {
          // Query 1 for All
          return new Promise<Response>((resolve) => {
            resolveQuery1 = resolve;
          });
        }
        if (url.includes("IsCompleted=false")) {
          // Open tab response
          return json({ items: [], totalCount: 0 });
        }
        if (queryCount >= 3) {
          // Query 3 for All again (A-B-A sequence)
          return new Promise<Response>((resolve) => {
            resolveQueryFinal = resolve;
          });
        }
      }
      return json({ items: [], totalCount: 0 });
    });

    const user = userEvent.setup();
    render(
      <ProjectTasksSection
        projectId="p1"
        workspaceId={WORKSPACE_ID}
        projectArchived={false}
        isLive={true}
        canMutate={true}
      />
    );

    // Query 1 in flight for "all".
    // Toggle filter: Open -> All to issue Query 3 for "all" (A-B-A sequence)
    const openTab = await screen.findByRole("tab", { name: "Open" });
    const allTab = screen.getByRole("tab", { name: "All" });

    await user.click(openTab);
    await user.click(allTab);

    // Resolve Query 3 first with fresh data
    await waitFor(() => expect(resolveQueryFinal).not.toBeNull());
    if (resolveQueryFinal) {
      (resolveQueryFinal as (res: Response) => void)(
        json({
          items: [{ id: "fresh-task", title: "Fresh Task Item", isCompleted: false }],
          totalCount: 1,
        })
      );
    }

    expect(await screen.findByText("Fresh Task Item")).toBeInTheDocument();

    // Now resolve older Query 1 with stale data
    if (resolveQuery1) {
      (resolveQuery1 as (res: Response) => void)(
        json({
          items: [{ id: "stale-task", title: "Stale Task Item", isCompleted: false }],
          totalCount: 1,
        })
      );
    }

    // Stale Query 1 must NOT overwrite Fresh Task Item
    expect(screen.queryByText("Stale Task Item")).not.toBeInTheDocument();
    expect(screen.getByText("Fresh Task Item")).toBeInTheDocument();
  });

  it("creating an open task while on completed filter does not increment completed totalCount, and local insertion is bounded", async () => {
    let getCall = 0;
    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input);
      const method = init?.method ?? "GET";

      if (url.includes("/api/app/project-task")) {
        if (method === "GET") {
          getCall++;
          if (getCall <= 2) {
            // initial load (first on All, then on Completed)
            return json({
              items: Array.from({ length: 10 }, (_, i) => ({
                id: `c-${i}`,
                title: `Completed Task ${i + 1}`,
                isCompleted: true,
              })),
              totalCount: 10,
            });
          }
          // Background refresh after create fails
          return json({ message: "Network failure" }, 500);
        }
        if (method === "POST") {
          return json({
            id: "new-open-task",
            projectId: PROJECT_A,
            title: "New Open Task",
            isCompleted: false,
          });
        }
      }
      return json({ items: [], totalCount: 0 });
    });

    const user = userEvent.setup();
    render(
      <ProjectTasksSection
        projectId={PROJECT_A}
        workspaceId={WORKSPACE_ID}
        projectArchived={false}
        isLive={true}
        canMutate={true}
      />
    );

    // Switch to Completed tab
    const completedTab = await screen.findByRole("tab", { name: "Completed" });
    await user.click(completedTab);

    expect(await screen.findByText("Completed Task 1")).toBeInTheDocument();
    expect(screen.getByText("10 tasks")).toBeInTheDocument();

    // Create task (which is open)
    const addBtn = screen.getByRole("button", { name: /add task/i });
    await user.click(addBtn);
    await user.type(screen.getByLabelText(/^title/i), "New Open Task");
    await user.click(screen.getByRole("button", { name: /save task/i }));

    // Refresh fails -> alert banner appears
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/couldn't refresh/i);

    // totalCount MUST NOT increment to 11! It must remain 10!
    expect(screen.getByText("10 tasks")).toBeInTheDocument();

    // The open task must NOT be in the completed list
    expect(screen.queryByText("New Open Task")).not.toBeInTheDocument();
    expect(screen.getAllByRole("listitem")).toHaveLength(10);
  });

  it("pending toggle mutation settlement after filter change does not overwrite current query results", async () => {
    let resolvePut: ((res: Response) => void) | null = null;

    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input);
      const method = init?.method ?? "GET";

      if (url.includes("/api/app/project-task")) {
        if (method === "PUT") {
          return new Promise<Response>((resolve) => {
            resolvePut = resolve;
          });
        }
        if (method === "GET") {
          // Completed tab query
          if (url.includes("IsCompleted=true")) {
            return json({
              items: [{ id: "completed-1", title: "Completed Task 1", isCompleted: true }],
              totalCount: 1,
            });
          }
          // Obsolete refresh query (all tasks, without IsCompleted)
          if (!url.includes("IsCompleted=")) {
            return json({
              items: [
                { id: "t1", title: "Initial Open Task", isCompleted: true },
                { id: "t2", title: "Other Open Task", isCompleted: false },
              ],
              totalCount: 2,
            });
          }
        }
      }
      return json({ items: [], totalCount: 0 });
    });

    // Provide initial state with t1 as open
    fetchMock.mockImplementationOnce(async () => {
      return json({
        items: [
          { id: "t1", title: "Initial Open Task", isCompleted: false },
          { id: "t2", title: "Other Open Task", isCompleted: false },
        ],
        totalCount: 2,
      });
    });

    const user = userEvent.setup();
    render(
      <ProjectTasksSection
        projectId={PROJECT_A}
        workspaceId={WORKSPACE_ID}
        projectArchived={false}
        isLive={true}
        canMutate={true}
      />
    );

    // Initial load displays the task
    expect(await screen.findByText("Initial Open Task")).toBeInTheDocument();
    expect(screen.getByText("2 tasks")).toBeInTheDocument();

    // Click toggle on t1 to complete it: PUT is dispatched and HELD by barrier
    const toggleBtn = screen.getByRole("button", { name: /mark "initial open task" as completed/i });
    await user.click(toggleBtn);

    expect(resolvePut).not.toBeNull();

    // While PUT is STILL PENDING, user clicks the "Completed" tab
    const completedTab = screen.getByRole("tab", { name: "Completed" });
    await user.click(completedTab);

    // The current query establishes results for Completed tab
    expect(await screen.findByText("Completed Task 1")).toBeInTheDocument();
    expect(screen.getByText("1 task")).toBeInTheDocument();
    expect(screen.queryByText("Other Open Task")).not.toBeInTheDocument();

    // Now release the held PUT write settlement
    (resolvePut as unknown as (res: Response) => void)(
      json({
        id: "t1",
        projectId: PROJECT_A,
        title: "Initial Open Task",
        isCompleted: true,
      })
    );

    // Give time for any microtasks/promises to settle
    await waitFor(() => {
      // Completed tab results must NOT be overwritten by the obsolete refresh
      expect(screen.getByText("Completed Task 1")).toBeInTheDocument();
    });

    // The other open task from the obsolete "all" query must NOT appear in Completed tab!
    expect(screen.queryByText("Other Open Task")).not.toBeInTheDocument();
    // Total count must remain 1 task, not 2 tasks!
    expect(screen.getByText("1 task")).toBeInTheDocument();
  });

  it("toggling completion on open filter removes task from view and decrements count when refresh fails", async () => {
    let backgroundRefreshFailed = false;

    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input);
      const method = init?.method ?? "GET";

      if (url.includes("/api/app/project-task")) {
        if (method === "GET") {
          // If background refresh after PUT
          if (backgroundRefreshFailed) {
            return json({ message: "Server error" }, 500);
          }
          if (url.includes("IsCompleted=false")) {
            return json({
              items: [{ id: "open-1", title: "Open Work Item", isCompleted: false }],
              totalCount: 1,
            });
          }
          // initial load on "all"
          return json({
            items: [{ id: "open-1", title: "Open Work Item", isCompleted: false }],
            totalCount: 1,
          });
        }
        if (method === "PUT") {
          backgroundRefreshFailed = true;
          return json({
            id: "open-1",
            projectId: PROJECT_A,
            title: "Open Work Item",
            isCompleted: true, // Now completed!
          });
        }
      }
      return json({ items: [], totalCount: 0 });
    });

    const user = userEvent.setup();
    render(
      <ProjectTasksSection
        projectId={PROJECT_A}
        workspaceId={WORKSPACE_ID}
        projectArchived={false}
        isLive={true}
        canMutate={true}
      />
    );

    // Switch to Open filter
    const openTab = await screen.findByRole("tab", { name: "Open" });
    await user.click(openTab);

    expect(await screen.findByText("Open Work Item")).toBeInTheDocument();
    expect(screen.getByText("1 task")).toBeInTheDocument();

    // Toggle complete
    const toggleBtn = screen.getByRole("button", { name: /mark "open work item" as completed/i });
    await user.click(toggleBtn);

    // Refresh fails -> alert banner shown
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/couldn't refresh/i);

    // The completed item MUST NOT remain visible in the Open filter list!
    expect(screen.queryByText("Open Work Item")).not.toBeInTheDocument();
    // Count should be decremented to 0
    expect(screen.getByText("0 tasks")).toBeInTheDocument();
  });

  it("pending create mutation settlement after filter change does not contaminate new filter", async () => {
    let resolveCreate: ((res: Response) => void) | null = null;

    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input);
      const method = init?.method ?? "GET";

      if (url.includes("/api/app/project-task")) {
        if (method === "POST") {
          return new Promise<Response>((resolve) => {
            resolveCreate = resolve;
          });
        }
        if (method === "GET") {
          if (url.includes("IsCompleted=true")) {
            return json({
              items: [{ id: "comp-1", title: "Existing Completed", isCompleted: true }],
              totalCount: 1,
            });
          }
          if (url.includes("IsCompleted=false")) {
            return json({
              items: [{ id: "open-1", title: "Existing Open", isCompleted: false }],
              totalCount: 1,
            });
          }
        }
      }
      return json({ items: [], totalCount: 0 });
    });

    const user = userEvent.setup();
    render(
      <ProjectTasksSection
        projectId={PROJECT_A}
        workspaceId={WORKSPACE_ID}
        projectArchived={false}
        isLive={true}
        canMutate={true}
      />
    );

    // Switch to Open filter
    const openTab = await screen.findByRole("tab", { name: "Open" });
    await user.click(openTab);
    expect(await screen.findByText("Existing Open")).toBeInTheDocument();

    // Click Add Task and submit
    const addBtn = screen.getByRole("button", { name: /add task/i });
    await user.click(addBtn);
    await user.type(screen.getByLabelText(/^title/i), "Orphaned Create Task");
    await user.click(screen.getByRole("button", { name: /save task/i }));

    expect(resolveCreate).not.toBeNull();

    // While POST is STILL PENDING, user clicks the "Completed" tab
    const completedTab = screen.getByRole("tab", { name: "Completed" });
    await user.click(completedTab);

    // Establish Completed tab results
    expect(await screen.findByText("Existing Completed")).toBeInTheDocument();
    expect(screen.getByText("1 task")).toBeInTheDocument();

    // Release held POST
    (resolveCreate as unknown as (res: Response) => void)(
      json({
        id: "created-new",
        projectId: PROJECT_A,
        title: "Orphaned Create Task",
        isCompleted: false,
      })
    );

    await waitFor(() => {
      expect(screen.getByText("Existing Completed")).toBeInTheDocument();
    });

    // Orphaned Create Task must NOT be added to Completed tab!
    expect(screen.queryByText("Orphaned Create Task")).not.toBeInTheDocument();
    expect(screen.getByText("1 task")).toBeInTheDocument();
  });

  it("page transition while toggle mutation is pending does not overwrite current page items", async () => {
    let resolvePut: ((res: Response) => void) | null = null;

    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input);
      const method = init?.method ?? "GET";

      if (url.includes("/api/app/project-task")) {
        if (method === "PUT") {
          return new Promise<Response>((resolve) => {
            resolvePut = resolve;
          });
        }
        if (method === "GET") {
          const skipCount = Number(new URL(url, "https://localhost").searchParams.get("SkipCount") || "0");
          if (skipCount === 0) {
            return json({
              items: Array.from({ length: 10 }, (_, i) => ({
                id: `p0-${i}`,
                title: `Page 0 Item ${i + 1}`,
                isCompleted: false,
              })),
              totalCount: 15,
            });
          }
          if (skipCount === 10) {
            return json({
              items: Array.from({ length: 5 }, (_, i) => ({
                id: `p1-${i}`,
                title: `Page 1 Item ${i + 1}`,
                isCompleted: false,
              })),
              totalCount: 15,
            });
          }
        }
      }
      return json({ items: [], totalCount: 0 });
    });

    const user = userEvent.setup();
    render(
      <ProjectTasksSection
        projectId={PROJECT_A}
        workspaceId={WORKSPACE_ID}
        projectArchived={false}
        isLive={true}
        canMutate={true}
      />
    );

    expect(await screen.findByText("Page 0 Item 1")).toBeInTheDocument();
    expect(screen.getByText("Page 1 of 2")).toBeInTheDocument();

    // Toggle item on Page 0: PUT dispatched and held
    const toggleBtn = screen.getByRole("button", { name: /mark "page 0 item 1" as completed/i });
    await user.click(toggleBtn);
    expect(resolvePut).not.toBeNull();

    // While PUT is pending, user clicks Next button to navigate to Page 1
    const nextBtn = screen.getByRole("button", { name: /next/i });
    await user.click(nextBtn);

    // Page 1 items established
    expect(await screen.findByText("Page 1 Item 1")).toBeInTheDocument();
    expect(screen.getByText("Page 2 of 2")).toBeInTheDocument();
    expect(screen.queryByText("Page 0 Item 1")).not.toBeInTheDocument();

    // Release held PUT from Page 0
    (resolvePut as unknown as (res: Response) => void)(
      json({
        id: "p0-0",
        projectId: PROJECT_A,
        title: "Page 0 Item 1",
        isCompleted: true,
      })
    );

    await waitFor(() => {
      expect(screen.getByText("Page 1 Item 1")).toBeInTheDocument();
    });

    // Page 1 must not be overwritten by Page 0 items
    expect(screen.queryByText("Page 0 Item 1")).not.toBeInTheDocument();
    expect(screen.getByText("Page 2 of 2")).toBeInTheDocument();
  });

  it("query ABA transition (All -> Open -> All) rejects delayed old mutation settlement", async () => {
    let resolvePut1: ((res: Response) => void) | null = null;
    let allQueryCount = 0;

    fetchMock.mockImplementation(async (input, init) => {
      const url = String(input);
      const method = init?.method ?? "GET";

      if (url.includes("/api/app/project-task")) {
        if (method === "PUT") {
          return new Promise<Response>((resolve) => {
            resolvePut1 = resolve;
          });
        }
        if (method === "GET") {
          if (url.includes("IsCompleted=false")) {
            return json({
              items: [{ id: "open-only", title: "Open Only Task", isCompleted: false }],
              totalCount: 1,
            });
          }
          if (!url.includes("IsCompleted=")) {
            allQueryCount++;
            if (allQueryCount === 1) {
              return json({
                items: [{ id: "t1", title: "Initial Query 1 Item", isCompleted: false }],
                totalCount: 1,
              });
            }
            // Second time visiting "All" (Query 3 in ABA)
            return json({
              items: [{ id: "t3", title: "Fresh Query 3 Item", isCompleted: false }],
              totalCount: 1,
            });
          }
        }
      }
      return json({ items: [], totalCount: 0 });
    });

    const user = userEvent.setup();
    render(
      <ProjectTasksSection
        projectId={PROJECT_A}
        workspaceId={WORKSPACE_ID}
        projectArchived={false}
        isLive={true}
        canMutate={true}
      />
    );

    // Initial load for All (Query 1)
    expect(await screen.findByText("Initial Query 1 Item")).toBeInTheDocument();

    // Toggle on Query 1: PUT held by barrier
    const toggleBtn = screen.getByRole("button", { name: /mark "initial query 1 item" as completed/i });
    await user.click(toggleBtn);
    expect(resolvePut1).not.toBeNull();

    // Switch to Open (Query 2 in ABA)
    const openTab = screen.getByRole("tab", { name: "Open" });
    await user.click(openTab);
    expect(await screen.findByText("Open Only Task")).toBeInTheDocument();

    // Switch back to All (Query 3 in ABA)
    const allTab = screen.getByRole("tab", { name: "All" });
    await user.click(allTab);
    expect(await screen.findByText("Fresh Query 3 Item")).toBeInTheDocument();

    // Now release held PUT from Query 1
    (resolvePut1 as unknown as (res: Response) => void)(
      json({
        id: "t1",
        projectId: PROJECT_A,
        title: "Initial Query 1 Item",
        isCompleted: true,
      })
    );

    await waitFor(() => {
      expect(screen.getByText("Fresh Query 3 Item")).toBeInTheDocument();
    });

    // Query 3 items must remain active; Query 1 obsolete refresh must not overwrite
    expect(screen.queryByText("Initial Query 1 Item")).not.toBeInTheDocument();
    expect(screen.getByText("Fresh Query 3 Item")).toBeInTheDocument();
  });
});

