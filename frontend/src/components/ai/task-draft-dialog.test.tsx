// "Draft tasks with AI" (task-drafts, AI assist spec §18.4): a goal (500 max,
// counted) and how many tasks; drafts in a "Suggested tasks" fieldset, each
// with a checkbox, an editable title and description, and a flag when it may
// duplicate an existing task (which starts unticked); the selection count is
// announced; nothing is created until "Add N tasks", and then only through
// the ordinary task endpoint, one by one. Focus stays trapped in the dialog
// and Escape closes it. A demo drafts from the demo data and adds nothing.
// Every state passes axe.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const ctx = vi.hoisted(() => ({ value: null as unknown as AiAssistantValue }));
vi.mock("@/components/ai/ai-provider", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./ai-provider")>()),
  useAiAssistant: () => ctx.value,
}));
vi.mock("@/hooks/ai-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/ai-api")>()),
  requestAiFeature: vi.fn(),
  sendAiFeedback: vi.fn(async () => {}),
}));
vi.mock("@/hooks/useApi", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/useApi")>()),
  createProjectTaskApi: vi.fn(),
}));

import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { TaskDraftDialog } from "./task-draft-dialog";
import type { AiAssistantValue } from "./ai-provider";
import { ToastViewport } from "@/components/toast";
import { requestAiFeature } from "@/hooks/ai-api";
import { createProjectTaskApi } from "@/hooks/useApi";
import type { AiTaskDraftsDto } from "@/lib/ai-contract";
import type { ProjectTaskDto } from "@/lib/api-contract";
import { datasetFor } from "@/lib/tenant-data";
import { aiApiError, aiContext, aiMeta, deferred } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";

const EXISTING: ProjectTaskDto = { id: "t-1", projectId: "p-1", title: "Write the onboarding copy", description: null, isCompleted: false } as ProjectTaskDto;

function drafts(): AiTaskDraftsDto {
  return {
    meta: aiMeta("task-drafts"),
    drafts: [
      { title: "Define the scope", description: "What the launch covers.", rationale: "Keeps the rest small.", possibleDuplicateOfTaskId: null },
      { title: "Write the onboarding copy", description: null, rationale: "Users need words.", possibleDuplicateOfTaskId: "t-1" },
      { title: "Review with the team", description: "Walk through it.", rationale: "A second view.", possibleDuplicateOfTaskId: null },
    ],
  };
}

function Dialog({ onClose = vi.fn(), onCreated = vi.fn() }: { onClose?: () => void; onCreated?: (n: number) => void }) {
  return (
    <>
      <button type="button">Draft tasks with AI</button>
      <TaskDraftDialog projectId="p-1" existingTasks={[EXISTING]} onClose={onClose} onCreated={onCreated} />
      <ToastViewport />
    </>
  );
}

async function draft(user: ReturnType<typeof userEvent.setup>, goal = "Launch the new onboarding") {
  await user.type(screen.getByRole("textbox", { name: "Goal" }), goal);
  await user.click(screen.getByRole("button", { name: "Draft tasks" }));
  return screen.findByRole("group", { name: "Suggested tasks" });
}

beforeEach(() => {
  resetPrototypeState();
  ctx.value = aiContext();
  vi.mocked(requestAiFeature).mockReset();
  vi.mocked(createProjectTaskApi).mockReset();
});

describe("asking for drafts", () => {
  it("counts the goal, needs 3 characters, and sends the goal and how many tasks", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValue(drafts());
    render(<Dialog />);
    const goal = screen.getByRole("textbox", { name: "Goal" });
    expect(goal).toHaveAttribute("maxLength", "500");
    expect(screen.getByRole("button", { name: "Draft tasks" })).toBeDisabled();
    await user.type(goal, "Go");
    expect(screen.getByText(/^2 \/ 500/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Draft tasks" })).toBeDisabled();
    await user.type(goal, "!");
    await user.selectOptions(screen.getByRole("combobox", { name: "How many tasks" }), "3");
    await user.click(screen.getByRole("button", { name: "Draft tasks" }));
    await screen.findByRole("group", { name: "Suggested tasks" });
    expect(vi.mocked(requestAiFeature).mock.calls[0].slice(0, 2)).toEqual(["task-drafts", { projectId: "p-1", goal: "Go!", maxTasks: 3 }]);
  });

  it("shows a cancellable loading state, and an error with Retry", async () => {
    const user = userEvent.setup();
    const pending = deferred<AiTaskDraftsDto>();
    vi.mocked(requestAiFeature)
      .mockReturnValueOnce(pending.promise)
      .mockRejectedValueOnce(aiApiError(502, "Tracker:AiOutputInvalid"))
      .mockResolvedValueOnce(drafts());
    render(<Dialog />);
    await user.type(screen.getByRole("textbox", { name: "Goal" }), "Launch it");
    await user.click(screen.getByRole("button", { name: "Draft tasks" }));
    expect(screen.getAllByRole("status")[0]).toHaveTextContent("Drafting tasks...");
    expect(await axe(document.body)).toHaveNoViolations(); // a11y: ai card loading has no violations
    await user.click(screen.getAllByRole("button", { name: "Cancel" })[0]);
    expect(vi.mocked(requestAiFeature).mock.calls[0][2]?.signal?.aborted).toBe(true);

    await user.click(screen.getByRole("button", { name: "Draft tasks" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("The AI couldn't produce a reliable answer. Try again.");
    expect(await axe(document.body)).toHaveNoViolations(); // a11y: ai card error has no violations
    await user.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(await screen.findByRole("group", { name: "Suggested tasks" })).toBeInTheDocument();
  });
});

describe("reviewing and adding", () => {
  it("lists editable drafts, flags a likely duplicate (unticked), and announces the selection", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValue(drafts());
    render(<Dialog />);
    const group = await draft(user);
    const boxes = within(group).getAllByRole("checkbox");
    expect(boxes.map((b) => (b as HTMLInputElement).checked)).toEqual([true, false, true]);
    expect(within(group).getByText("Similar to an existing task: Write the onboarding copy")).toBeInTheDocument();
    expect(within(group).getByRole("textbox", { name: "Task 1 title" })).toHaveValue("Define the scope");
    expect(within(group).getByRole("textbox", { name: /Task 1 description/ })).toHaveValue("What the launch covers.");
    expect(screen.getByText("2 tasks selected")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add 2 tasks" })).toBeEnabled();
    await user.click(boxes[1]);
    expect(screen.getByText("3 tasks selected")).toBeInTheDocument();
    expect(await axe(document.body)).toHaveNoViolations(); // a11y: ai card ready has no violations
  });

  it("creates nothing until Add, then each ticked draft as edited, one by one", async () => {
    const user = userEvent.setup();
    const onCreated = vi.fn();
    const onClose = vi.fn();
    vi.mocked(requestAiFeature).mockResolvedValue(drafts());
    vi.mocked(createProjectTaskApi).mockImplementation(async (input) => ({ ...EXISTING, id: input.title, title: input.title }));
    render(<Dialog onCreated={onCreated} onClose={onClose} />);
    const group = await draft(user);
    expect(createProjectTaskApi).not.toHaveBeenCalled();
    const title = within(group).getByRole("textbox", { name: "Task 3 title" });
    await user.clear(title);
    await user.type(title, "  Review the flow with the team ");
    await user.clear(within(group).getByRole("textbox", { name: /Task 1 description/ }));
    await user.click(screen.getByRole("button", { name: "Add 2 tasks" }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(vi.mocked(createProjectTaskApi).mock.calls.map(([input]) => input)).toEqual([
      { projectId: "p-1", title: "Define the scope", description: null, isCompleted: false },
      { projectId: "p-1", title: "Review the flow with the team", description: "Walk through it.", isCompleted: false },
    ]);
    expect(onCreated).toHaveBeenCalledWith(2);
    expect(await screen.findByText("2 tasks added")).toBeInTheDocument();
  });

  it("checks every ticked draft before sending any", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValue(drafts());
    render(<Dialog />);
    const group = await draft(user);
    await user.clear(within(group).getByRole("textbox", { name: "Task 1 title" }));
    await user.click(screen.getByRole("button", { name: "Add 2 tasks" }));
    expect(createProjectTaskApi).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent("Fix the highlighted tasks first.");
    expect(within(group).getByRole("textbox", { name: "Task 1 title" })).toHaveAccessibleDescription("The task needs a title.");
  });

  it("keeps a draft that couldn't be created, with the reason, and adds the rest", async () => {
    const user = userEvent.setup();
    const onCreated = vi.fn();
    const onClose = vi.fn();
    vi.mocked(requestAiFeature).mockResolvedValue(drafts());
    vi.mocked(createProjectTaskApi)
      .mockResolvedValueOnce(EXISTING)
      .mockRejectedValueOnce(aiApiError(409, "Tracker:ProjectArchived"));
    render(<Dialog onCreated={onCreated} onClose={onClose} />);
    await draft(user);
    await user.click(screen.getByRole("button", { name: "Add 2 tasks" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Added 1 of 2 tasks. 1 couldn't be created");
    expect(onCreated).toHaveBeenCalledWith(1);
    expect(onClose).not.toHaveBeenCalled();
    const group = screen.getByRole("group", { name: "Suggested tasks" });
    // The created one is gone; the failed one stays ticked with its reason; the unticked one stays.
    expect(within(group).getAllByRole("textbox", { name: /title$/ }).map((t) => (t as HTMLInputElement).value)).toEqual([
      "Write the onboarding copy",
      "Review with the team",
    ]);
    expect(screen.getByText("1 task selected")).toBeInTheDocument();
    expect(await axe(document.body)).toHaveNoViolations(); // a11y: ai card partial failure has no violations
  });

  it("says so when none could be created", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValue(drafts());
    vi.mocked(createProjectTaskApi).mockRejectedValue(new TypeError("Failed to fetch"));
    const onCreated = vi.fn();
    render(<Dialog onCreated={onCreated} />);
    await draft(user);
    await user.click(screen.getByRole("button", { name: "Add 2 tasks" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("None of the 2 tasks could be created");
    expect(onCreated).not.toHaveBeenCalled();
  });

  it("traps focus and closes on Escape", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<Dialog onClose={onClose} />);
    const dialog = screen.getByRole("dialog", { name: "Draft tasks with AI" });
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));
    for (let i = 0; i < 12; i++) {
      await user.tab();
      expect(dialog.contains(document.activeElement)).toBe(true);
    }
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });

  it("a11y: ai card idle has no violations", async () => {
    render(<Dialog />);
    expect(await axe(document.body)).toHaveNoViolations();
  });
});

describe("demo", () => {
  it("drafts from the demo data and adds nothing, with no network", async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    const user = userEvent.setup();
    const onClose = vi.fn();
    const onCreated = vi.fn();
    ctx.value = aiContext({ live: false });
    const project = datasetFor("w1").projects[0];
    render(
      <>
        <TaskDraftDialog projectId={project.id} existingTasks={[]} onClose={onClose} onCreated={onCreated} />
        <ToastViewport />
      </>,
    );
    const group = await draft(user, "Ship the beta");
    expect(within(group).getByRole("textbox", { name: "Task 1 title" })).toHaveValue("Define the scope: Ship the beta");
    expect(screen.getByText("Sample", { selector: "span" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Add 5 tasks" }));
    expect(onClose).toHaveBeenCalled();
    expect(onCreated).not.toHaveBeenCalled();
    expect(await screen.findByText("5 tasks drafted")).toBeInTheDocument();
    expect(createProjectTaskApi).not.toHaveBeenCalled();
    expect(requestAiFeature).not.toHaveBeenCalled();
    expect(fetchSpy).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});
