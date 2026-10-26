// "Suggest" in the note editor (note-suggestions, AI assist spec §18.4):
// offered only while the note may be edited and the server allows the
// feature; asked only on a click; up to three suggestions; choosing one fills
// the draft (and focus goes back to it) — only "Save note" stores anything.
// Errors say why, with Retry when it can help. A demo suggests from the demo
// data with no network. Every state passes axe.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const ctx = vi.hoisted(() => ({ value: null as unknown as AiAssistantValue }));
vi.mock("@/components/ai/ai-provider", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./ai-provider")>()),
  useAiAssistant: () => ctx.value,
}));
vi.mock("@/hooks/ai-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/ai-api")>()),
  requestAiFeature: vi.fn(),
}));

import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import type { AiAssistantValue } from "./ai-provider";
import { NoteEditor } from "@/components/activities/note-editor";
import { requestAiFeature } from "@/hooks/ai-api";
import type { AiNoteSuggestionsDto } from "@/lib/ai-contract";
import { AI_COPY } from "@/lib/ai-assist";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { datasetFor } from "@/lib/tenant-data";
import { aiApiError, aiContext, aiMeta, aiStatusWith, deferred } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";

function suggestions(texts: string[]): AiNoteSuggestionsDto {
  return { meta: aiMeta("note-suggestions"), suggestions: texts.map((text) => ({ text, basis: "apps" as const })) };
}

function Editor({ canEdit = true, suggest = true, onSave = vi.fn(async () => {}) }: { canEdit?: boolean; suggest?: boolean; onSave?: (note: string | null) => Promise<void> }) {
  return (
    <main>
      <h1>Activity</h1>
      <NoteEditor note="Old note" editing={{ canEdit, onSave }} suggest={suggest ? { activityId: "act-1" } : undefined} />
    </main>
  );
}

const startEditing = async (user: ReturnType<typeof userEvent.setup>) => user.click(screen.getByRole("button", { name: "Edit note" }));

beforeEach(() => {
  resetPrototypeState();
  ctx.value = aiContext();
  vi.mocked(requestAiFeature).mockReset();
});

describe("who gets Suggest", () => {
  it("offers it only while editing, and asks nothing until clicked", async () => {
    const user = userEvent.setup();
    render(<Editor />);
    expect(screen.queryByRole("button", { name: "Suggest" })).toBeNull();
    await startEditing(user);
    expect(await screen.findByRole("button", { name: "Suggest" })).toBeInTheDocument();
    expect(requestAiFeature).not.toHaveBeenCalled();
  });

  it.each<[string, Partial<AiAssistantValue>]>([
    ["a server without AI", { status: null }],
    ["a workspace that hasn't opted in", { status: aiStatusWith("workspace-disabled") }],
    ["a client", { status: demoAiStatus("client"), role: "client" }],
  ])("is hidden for %s", async (_label, patch) => {
    const user = userEvent.setup();
    ctx.value = aiContext(patch);
    render(<Editor />);
    await startEditing(user);
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByRole("button", { name: "Suggest" })).toBeNull();
  });

  it("is hidden without a block to suggest for, and for a read-only note", async () => {
    const user = userEvent.setup();
    const first = render(<Editor suggest={false} />);
    await startEditing(user);
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByRole("button", { name: "Suggest" })).toBeNull();
    first.unmount();
    render(<Editor canEdit={false} />);
    expect(screen.queryByRole("button", { name: "Edit note" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Suggest" })).toBeNull();
  });
});

describe("live", () => {
  it("fills the draft with the chosen suggestion; only Save note stores it", async () => {
    const user = userEvent.setup();
    const onSave = vi.fn(async () => {});
    vi.mocked(requestAiFeature).mockResolvedValue(suggestions(["Worked on Apollo in VS Code", "Worked on Apollo", "Release notes"]));
    const { container } = render(<Editor onSave={onSave} />);
    await startEditing(user);
    await user.click(await screen.findByRole("button", { name: "Suggest" }));
    expect(vi.mocked(requestAiFeature).mock.calls[0].slice(0, 2)).toEqual(["note-suggestions", { activityId: "act-1" }]);
    const group = await screen.findByRole("group", { name: "Suggested notes" });
    expect(within(group).getAllByRole("button")).toHaveLength(3);
    expect(screen.getByRole("status")).toHaveTextContent("Suggestions ready");
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card ready has no violations

    await user.click(within(group).getByRole("button", { name: "Worked on Apollo" }));
    const field = screen.getByRole("textbox", { name: "Task note" });
    expect(field).toHaveValue("Worked on Apollo");
    expect(field).toHaveFocus();
    expect(onSave).not.toHaveBeenCalled();

    await user.type(field, " docs");
    await user.click(screen.getByRole("button", { name: "Save note" }));
    expect(onSave).toHaveBeenCalledTimes(1);
    expect(onSave).toHaveBeenCalledWith("Worked on Apollo docs");
  });

  it("shows a cancellable loading state, and says when there is nothing to suggest", async () => {
    const user = userEvent.setup();
    const pending = deferred<AiNoteSuggestionsDto>();
    vi.mocked(requestAiFeature).mockReturnValueOnce(pending.promise).mockResolvedValueOnce(suggestions([]));
    const { container } = render(<Editor />);
    await startEditing(user);
    await user.click(await screen.findByRole("button", { name: "Suggest" }));
    expect(screen.getByRole("status")).toHaveTextContent("Finding suggestions...");
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card loading has no violations
    await user.click(screen.getByRole("button", { name: "Cancel suggestions" }));
    // The editor's own Cancel is still there, and still the editor's.
    expect(screen.getByRole("textbox", { name: "Task note" })).toBeInTheDocument();
    expect(vi.mocked(requestAiFeature).mock.calls[0][2]?.signal?.aborted).toBe(true);
    await user.click(await screen.findByRole("button", { name: "Suggest" }));
    expect(await screen.findByText("No suggestions for this block.")).toBeInTheDocument();
  });

  it("shows a refusal with Retry when trying again can help", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockRejectedValueOnce(aiApiError(503, "Tracker:AiProviderUnavailable")).mockResolvedValueOnce(suggestions(["Worked on Apollo"]));
    const { container } = render(<Editor />);
    await startEditing(user);
    await user.click(await screen.findByRole("button", { name: "Suggest" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("The AI service is slow or unavailable.");
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card error has no violations
    await user.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(await screen.findByRole("button", { name: "Worked on Apollo" })).toBeInTheDocument();
  });

  it("offers no Retry for a refusal that won't change", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockRejectedValue(aiApiError(403, "Tracker:AiFeatureDisabled", { reason: "role" }));
    render(<Editor />);
    await startEditing(user);
    await user.click(await screen.findByRole("button", { name: "Suggest" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("This AI feature isn't available to you.");
    expect(within(alert).queryByRole("button")).toBeNull();
  });
});

describe("demo", () => {
  it("suggests from the demo block with no network", async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    const user = userEvent.setup();
    ctx.value = aiContext({ live: false });
    const block = datasetFor("w1").activities[0];
    render(<NoteEditor note={null} editing={{ canEdit: true, onSave: async () => {} }} suggest={{ activityId: block.id }} />);
    await user.click(screen.getByRole("button", { name: "Add note" }));
    await user.click(await screen.findByRole("button", { name: "Suggest" }));
    const group = await screen.findByRole("group", { name: "Suggested notes" });
    expect(within(group).getAllByRole("button").length).toBeGreaterThan(0);
    expect(screen.getByText(AI_COPY.sampleBadge, { selector: "span" })).toBeInTheDocument();
    await waitFor(() => expect(requestAiFeature).not.toHaveBeenCalled());
    expect(fetchSpy).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});
