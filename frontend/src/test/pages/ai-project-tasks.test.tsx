// "Draft tasks with AI" on a project's task list in a LIVE session (AI assist
// spec §3.1, §18.5): offered only where tasks can be added (canMutate, not
// archived) and the server allows this caller task drafts; choosing 2 of 4
// drafts creates exactly those 2 through the ordinary task endpoint, nothing
// is created without the click, a partial failure says which one failed and
// why, and the list reloads afterwards. HTTP and AI calls are mocked; no
// request ever reaches an AI provider.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));
vi.mock("@/hooks/useApi", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/useApi")>()),
  getProjectTasksApi: vi.fn(),
  createProjectTaskApi: vi.fn(),
  updateProjectTaskApi: vi.fn(),
}));
const ai = vi.hoisted(() => ({ value: null as unknown }));
vi.mock("@/components/ai/ai-provider", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/components/ai/ai-provider")>()),
  useAiAssistant: () => ai.value,
}));
vi.mock("@/hooks/ai-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/ai-api")>()),
  requestAiFeature: vi.fn(),
  sendAiFeedback: vi.fn(async () => {}),
}));

import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ProjectTasksSection } from "@/components/projects/project-tasks";
import { ToastViewport } from "@/components/toast";
import { ApiError, createProjectTaskApi, getProjectTasksApi } from "@/hooks/useApi";
import { requestAiFeature } from "@/hooks/ai-api";
import type { AiStatusDto, AiTaskDraftsDto } from "@/lib/ai-contract";
import type { ProjectTaskDto } from "@/lib/api-contract";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { aiContext, aiMeta } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";

const WS = "99999999-9999-9999-9999-999999999999";
const PROJECT = "aaaa1111-1111-1111-1111-111111111111";

const task = (id: string, title: string): ProjectTaskDto =>
  ({ id, title, description: null, isCompleted: false, projectId: PROJECT, workspaceId: WS, creationTime: "2026-10-24T00:00:00Z" }) as ProjectTaskDto;

const FOUR: AiTaskDraftsDto = {
  meta: aiMeta("task-drafts"),
  drafts: [
    { title: "Define the scope", description: "What the launch covers.", rationale: "Keeps the rest small.", possibleDuplicateOfTaskId: null },
    { title: "List the open questions", description: null, rationale: "Unknowns early.", possibleDuplicateOfTaskId: null },
    { title: "Draft a first version", description: "A first working version.", rationale: "Something concrete.", possibleDuplicateOfTaskId: null },
    { title: "Review with the team", description: null, rationale: "A second view.", possibleDuplicateOfTaskId: null },
  ],
};

function withTaskDrafts(allowed: boolean, base: AiStatusDto = demoAiStatus("admin")): AiStatusDto {
  return { ...base, features: base.features.map((f) => (f.featureId === "task-drafts" ? { ...f, allowed, reason: allowed ? null : "permission" } : f)) };
}

function Section(props: Partial<React.ComponentProps<typeof ProjectTasksSection>>) {
  return (
    <>
      <ProjectTasksSection projectId={PROJECT} projectArchived={false} isLive canMutate workspaceId={WS} {...props} />
      <ToastViewport />
    </>
  );
}

async function draftFour(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("button", { name: "Draft tasks with AI" }));
  const dialog = await screen.findByRole("dialog", { name: "Draft tasks with AI" });
  await user.type(within(dialog).getByRole("textbox", { name: "Goal" }), "Launch the new onboarding");
  await user.click(within(dialog).getByRole("button", { name: "Draft tasks" }));
  await within(dialog).findByRole("group", { name: "Suggested tasks" });
  return dialog;
}

beforeEach(() => {
  resetPrototypeState();
  localStorage.setItem("dosi-token", "jwt-live");
  ai.value = aiContext({ role: "admin", status: demoAiStatus("admin"), workspaceId: WS });
  vi.mocked(getProjectTasksApi).mockReset();
  vi.mocked(getProjectTasksApi).mockResolvedValue({ items: [task("task-1", "Setup auth")], totalCount: 1 });
  vi.mocked(createProjectTaskApi).mockReset();
  vi.mocked(requestAiFeature).mockReset();
  vi.mocked(requestAiFeature).mockResolvedValue(FOUR);
});

describe("who gets Draft tasks with AI", () => {
  it("is offered next to Add task when tasks can be added and the server allows it", async () => {
    render(<Section />);
    expect(await screen.findByRole("button", { name: "Draft tasks with AI" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Add task/ })).toBeInTheDocument();
  });

  it.each<[string, Partial<React.ComponentProps<typeof ProjectTasksSection>>, AiStatusDto | null]>([
    ["an archived project", { projectArchived: true }, demoAiStatus("admin")],
    ["someone who can't add tasks", { canMutate: false }, demoAiStatus("admin")],
    ["a caller the server refuses task drafts", {}, withTaskDrafts(false)],
    ["a server without AI", {}, null],
  ])("is hidden for %s", async (_label, props, status) => {
    ai.value = aiContext({ role: "admin", status, workspaceId: WS });
    render(<Section {...props} />);
    await screen.findByText("Setup auth");
    expect(screen.queryByRole("button", { name: "Draft tasks with AI" })).toBeNull();
  });
});

describe("drafting and adding (live)", () => {
  it("creates exactly the 2 drafts chosen of 4, only on the click, then reloads the list", async () => {
    const user = userEvent.setup();
    vi.mocked(createProjectTaskApi).mockImplementation(async (input) => task(`new-${input.title}`, input.title));
    render(<Section />);
    await screen.findByText("Setup auth");
    const dialog = await draftFour(user);
    expect(vi.mocked(requestAiFeature).mock.calls[0].slice(0, 2)).toEqual(["task-drafts", { projectId: PROJECT, goal: "Launch the new onboarding", maxTasks: 5 }]);

    const boxes = within(dialog).getAllByRole("checkbox");
    expect(boxes).toHaveLength(4);
    await user.click(boxes[1]);
    await user.click(boxes[3]);
    expect(within(dialog).getByText("2 tasks selected")).toBeInTheDocument();
    expect(createProjectTaskApi).not.toHaveBeenCalled();

    vi.mocked(getProjectTasksApi).mockClear();
    await user.click(within(dialog).getByRole("button", { name: "Add 2 tasks" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Draft tasks with AI" })).toBeNull());
    expect(createProjectTaskApi).toHaveBeenCalledTimes(2);
    expect(vi.mocked(createProjectTaskApi).mock.calls.map(([input]) => input.title)).toEqual(["Define the scope", "Draft a first version"]);
    expect(vi.mocked(createProjectTaskApi).mock.calls[0][0]).toEqual({ projectId: PROJECT, title: "Define the scope", description: "What the launch covers.", isCompleted: false });
    expect(await screen.findByText("2 tasks added")).toBeInTheDocument();
    await waitFor(() => expect(getProjectTasksApi).toHaveBeenCalled());
  });

  it("closing the dialog creates nothing", async () => {
    const user = userEvent.setup();
    render(<Section />);
    const dialog = await draftFour(user);
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog", { name: "Draft tasks with AI" })).toBeNull();
    expect(createProjectTaskApi).not.toHaveBeenCalled();
  });

  it("shows which draft failed and why, keeps the dialog open, and still reloads for the one added", async () => {
    const user = userEvent.setup();
    vi.mocked(createProjectTaskApi)
      .mockResolvedValueOnce(task("new-1", "Define the scope"))
      .mockRejectedValueOnce(new ApiError("HTTP 403", 403, { code: "Tracker:RoleNotAllowed", message: "Clients can't add tasks to this project.", details: null, validationErrors: [] }));
    render(<Section />);
    const dialog = await draftFour(user);
    const boxes = within(dialog).getAllByRole("checkbox");
    await user.click(boxes[2]);
    await user.click(boxes[3]);
    vi.mocked(getProjectTasksApi).mockClear();
    await user.click(within(dialog).getByRole("button", { name: "Add 2 tasks" }));

    expect(await within(dialog).findByRole("alert")).toHaveTextContent("Added 1 of 2 tasks. 1 couldn't be created");
    expect(within(dialog).getByText("Clients can't add tasks to this project.")).toBeInTheDocument();
    expect(within(dialog).getByRole("textbox", { name: "Task 1 title" })).toHaveValue("List the open questions");
    expect(createProjectTaskApi).toHaveBeenCalledTimes(2);
    await waitFor(() => expect(getProjectTasksApi).toHaveBeenCalled());
  });
});
