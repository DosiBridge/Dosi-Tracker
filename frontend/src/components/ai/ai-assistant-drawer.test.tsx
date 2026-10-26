// The Ask AI drawer (AI assist spec §9.2, §18.3, §18.4, §18.8), driven through
// the real AiAssistantProvider and the topbar's launcher: it opens with focus
// in the composer and gives it back to the launcher on close (Escape too);
// sends the question with what the page shows; answers into a polite log
// with "Ask AI is answering" / "Answer ready"; says whose activity it looks
// at; handles errors, Stop, New chat and an expired conversation; explains AI
// assist to the owner of a workspace that hasn't opted in; and in the demo
// answers the suggested questions only, with no network. Every state passes
// axe.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const session = vi.hoisted(() => ({ current: null as unknown }));
vi.mock("@/components/session-provider", () => ({ useSession: () => session.current }));
const aiStatus = vi.hoisted(() => ({ current: null as unknown }));
vi.mock("@/hooks/use-ai-status", () => ({ useAiStatus: () => aiStatus.current }));
vi.mock("@/hooks/ai-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/ai-api")>()),
  askAi: vi.fn(),
  sendAiFeedback: vi.fn(async () => {}),
}));

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { AiAssistantMount } from "./ai-assistant-mount";
import { AiLauncher } from "./ai-launcher";
import { AiAssistantProvider, useAiAssistant, useAiPageContext } from "./ai-provider";
import { AiSetupHelpButton } from "./ai-setup-help-button";
import { ToastViewport } from "@/components/toast";
import { askAi } from "@/hooks/ai-api";
import type { AiAnswerDto, AiPageContextDto, AiStatusDto } from "@/lib/ai-contract";
import { AI_PROMPT_CHIPS, AI_SETUP_HELP_CHIPS, quotaNote } from "@/lib/ai-assist";
import { demoAiStatus } from "@/lib/ai-demo-status";
import type { Role } from "@/lib/types";
import { aiApiError, aiMeta, aiStatusWith, deferred } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";
import { __setPathname } from "@/test/next-navigation-stub";

function answer(text: string, conversationId = "c-1"): AiAnswerDto {
  return { meta: aiMeta("ask"), conversationId, answer: text, sources: [], facts: [], isClarification: false };
}

function signIn(role: Role, status: AiStatusDto | null = demoAiStatus(role), live = true) {
  // A demo session reads the seeded demo workspace w1.
  session.current = { isLive: live, user: { id: `${role}-1`, role }, workspace: { id: live ? "ws-1" : "w1" } };
  aiStatus.current = { status, loading: false, refresh: () => {} };
}

/** A page that tells Ask AI what it shows. */
function Page({ context }: { context: AiPageContextDto | null }) {
  useAiPageContext(context);
  return null;
}

/** Opens the drawer the way the command palette does: with a question to ask at once. */
function AskButton() {
  const { openAssistant } = useAiAssistant();
  return (
    <button type="button" onClick={() => openAssistant({ prompt: AI_PROMPT_CHIPS.dashboard[1] })}>
      Palette question
    </button>
  );
}

function Shell({ context = null }: { context?: AiPageContextDto | null }) {
  return (
    <AiAssistantProvider>
      <header>
        <AiLauncher />
      </header>
      <main>
        <h1>Dashboard</h1>
        <Page context={context} />
        <AiSetupHelpButton />
        <AskButton />
      </main>
      <AiAssistantMount />
      <ToastViewport />
    </AiAssistantProvider>
  );
}

const open = async (user: ReturnType<typeof userEvent.setup>) => {
  await user.click(screen.getByRole("button", { name: "Ask AI" }));
  return screen.findByRole("dialog", { name: "Ask AI" });
};

beforeEach(() => {
  resetPrototypeState();
  __setPathname("/dashboard");
  vi.mocked(askAi).mockReset();
  signIn("owner");
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("opening and closing", () => {
  it("moves focus to the composer on open, and back to the launcher on close; Escape closes", async () => {
    const user = userEvent.setup();
    render(<Shell />);
    const launcher = screen.getByRole("button", { name: "Ask AI" });
    expect(launcher).toHaveAttribute("aria-expanded", "false");
    const dialog = await open(user);
    await waitFor(() => expect(within(dialog).getByRole("textbox", { name: "Your question" })).toHaveFocus());
    expect(launcher).toHaveAttribute("aria-expanded", "true");

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Ask AI" })).toBeNull();
    expect(launcher).toHaveFocus();

    // Open again: the Close button closes it too, and focus comes back again.
    await user.click(launcher);
    await user.click(within(await screen.findByRole("dialog", { name: "Ask AI" })).getByRole("button", { name: "Close" }));
    expect(launcher).toHaveFocus();
  });

  it("a11y: ai drawer ready has no violations", async () => {
    const user = userEvent.setup();
    render(<Shell />);
    await open(user);
    expect(await axe(document.body)).toHaveNoViolations();
  });
});

describe("asking (live)", () => {
  it("sends the question with the page's context, and announces the answer without taking focus", async () => {
    const user = userEvent.setup();
    const pending = deferred<AiAnswerDto>();
    vi.mocked(askAi).mockReturnValue(pending.promise);
    const context: AiPageContextDto = { page: "timesheet", from: "2026-07-13T00:00:00.000Z", to: "2026-07-20T00:00:00.000Z", timeZone: "Asia/Dhaka" };
    render(<Shell context={context} />);
    const dialog = await open(user);
    // The page's own questions, and whose activity the assistant looks at.
    const chips = within(dialog).getByRole("group", { name: "Suggested questions" });
    expect(within(chips).getAllByRole("button").map((b) => b.textContent)).toEqual(AI_PROMPT_CHIPS.timesheet.map((c) => c.label));
    expect(within(dialog).getByText("Team")).toBeInTheDocument();

    const composer = within(dialog).getByRole("textbox", { name: "Your question" });
    await user.type(composer, "Who tracked the most?");
    expect(within(dialog).getByText(/^21 \/ 1000/)).toBeInTheDocument();
    await user.keyboard("{Enter}");
    expect(askAi).toHaveBeenCalledTimes(1);
    expect(vi.mocked(askAi).mock.calls[0][0]).toEqual({ conversationId: null, message: "Who tracked the most?", context });
    expect(composer).toHaveValue("");
    expect(within(dialog).getByRole("status")).toHaveTextContent("Ask AI is answering");
    expect(within(dialog).getByRole("button", { name: "Stop" })).toBeInTheDocument();
    expect(await axe(document.body)).toHaveNoViolations(); // a11y: ai drawer sending has no violations

    pending.resolve(answer("Wendy tracked the most.\n- Wendy: 31 h"));
    const log = within(dialog).getByRole("log", { name: "Conversation" });
    expect(await within(log).findByText("Wendy tracked the most.")).toBeInTheDocument();
    expect(within(log).getByText("Wendy: 31 h")).toBeInTheDocument();
    expect(log).toHaveAttribute("aria-live", "polite");
    expect(within(dialog).getByRole("status")).toHaveTextContent("Answer ready");
    expect(composer).toHaveFocus();
    expect(await axe(document.body)).toHaveNoViolations(); // a11y: ai drawer answered has no violations
  });

  it("uses the page's questions from the path when no card registered a context", async () => {
    const user = userEvent.setup();
    __setPathname("/download");
    vi.mocked(askAi).mockResolvedValue(answer("Open Download."));
    render(<Shell />);
    const dialog = await open(user);
    await user.click(within(dialog).getByRole("button", { name: AI_PROMPT_CHIPS.download[0].label }));
    expect(vi.mocked(askAi).mock.calls[0][0]).toMatchObject({ message: AI_PROMPT_CHIPS.download[0].label, context: { page: "download" } });
  });

  it("asks a question the drawer was opened with, once", async () => {
    const user = userEvent.setup();
    vi.mocked(askAi).mockResolvedValue(answer("Apollo took the most time."));
    render(<Shell />);
    await user.click(screen.getByRole("button", { name: "Palette question" }));
    const dialog = await screen.findByRole("dialog", { name: "Ask AI" });
    expect(await within(dialog).findByText("Apollo took the most time.")).toBeInTheDocument();
    expect(askAi).toHaveBeenCalledTimes(1);
    expect(vi.mocked(askAi).mock.calls[0][0].message).toBe(AI_PROMPT_CHIPS.dashboard[1].label);
  });

  it("offers the setup assistant's questions when opened from a setup button", async () => {
    const user = userEvent.setup();
    render(<Shell />);
    await user.click(screen.getByRole("button", { name: "Ask the setup assistant" }));
    const dialog = await screen.findByRole("dialog", { name: "Ask AI" });
    const chips = within(dialog).getByRole("group", { name: "Suggested questions" });
    expect(within(chips).getAllByRole("button").map((b) => b.textContent)).toEqual(AI_SETUP_HELP_CHIPS.map((c) => c.label));
  });

  it("Stop abandons the question; New chat clears the conversation", async () => {
    const user = userEvent.setup();
    vi.mocked(askAi).mockReturnValueOnce(new Promise(() => {})).mockResolvedValueOnce(answer("Second answer"));
    render(<Shell />);
    const dialog = await open(user);
    await user.type(within(dialog).getByRole("textbox", { name: "Your question" }), "First{Enter}");
    await user.click(within(dialog).getByRole("button", { name: "Stop" }));
    expect(vi.mocked(askAi).mock.calls[0][1]?.signal?.aborted).toBe(true);
    await user.type(within(dialog).getByRole("textbox", { name: "Your question" }), "Second{Enter}");
    expect(await within(dialog).findByText("Second answer")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "New chat" }));
    expect(within(dialog).queryByText("Second answer")).toBeNull();
    expect(within(dialog).getByRole("button", { name: "New chat" })).toBeDisabled();
  });

  it("shows a refusal as an alert with Retry when trying again can help", async () => {
    const user = userEvent.setup();
    vi.mocked(askAi).mockRejectedValueOnce(aiApiError(503, "Tracker:AiProviderUnavailable", { kind: "timeout" })).mockResolvedValueOnce(answer("Back again"));
    render(<Shell />);
    const dialog = await open(user);
    await user.type(within(dialog).getByRole("textbox", { name: "Your question" }), "Hello{Enter}");
    const alert = await within(dialog).findByRole("alert");
    expect(alert).toHaveTextContent("The AI service is slow or unavailable. Try again in a minute.");
    expect(await axe(document.body)).toHaveNoViolations(); // a11y: ai drawer error has no violations
    await user.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(await within(dialog).findByText("Back again")).toBeInTheDocument();
    expect(vi.mocked(askAi).mock.calls[1][0].message).toBe("Hello");
  });

  it("starts a new conversation when the old one expired, and says so", async () => {
    const user = userEvent.setup();
    vi.mocked(askAi)
      .mockResolvedValueOnce(answer("First", "c-1"))
      .mockRejectedValueOnce(aiApiError(409, "Tracker:AiConversationExpired"))
      .mockResolvedValueOnce(answer("Asked anew", "c-2"));
    render(<Shell />);
    const dialog = await open(user);
    const composer = within(dialog).getByRole("textbox", { name: "Your question" });
    await user.type(composer, "One{Enter}");
    await within(dialog).findByText("First");
    await user.type(composer, "Two{Enter}");
    expect(await within(dialog).findByText("Asked anew")).toBeInTheDocument();
    expect(await screen.findByText("Started a new conversation")).toBeInTheDocument();
  });

  it("says when this month's allowance is used, and that a member's assistant sees only their own activity", async () => {
    const user = userEvent.setup();
    signIn("owner", aiStatusWith("quota-exhausted"));
    const first = render(<Shell />);
    const dialog = await open(user);
    expect(within(dialog).getByRole("note")).toHaveTextContent(quotaNote(aiStatusWith("quota-exhausted").quota?.resetsAt, "UTC").slice(0, 40));
    first.unmount();

    signIn("worker");
    render(<Shell />);
    expect(within(await open(user)).getByText("Only your activity")).toBeInTheDocument();
  });

  it("links to what is shared, closing the drawer", async () => {
    const user = userEvent.setup();
    render(<Shell />);
    const dialog = await open(user);
    const link = within(dialog).getByRole("link", { name: "What is shared?" });
    expect(link).toHaveAttribute("href", "/settings?tab=privacy");
    await user.click(link);
    expect(screen.queryByRole("dialog", { name: "Ask AI" })).toBeNull();
  });
});

describe("who gets it", () => {
  it("explains AI assist to the owner of a workspace that hasn't opted in, with Review and turn on", async () => {
    const user = userEvent.setup();
    signIn("owner", aiStatusWith("workspace-disabled", { workspaceEnabled: false }));
    render(<Shell />);
    const dialog = await open(user);
    expect(within(dialog).getByRole("link", { name: "Review and turn on" })).toHaveAttribute("href", "/settings?tab=workspace");
    expect(within(dialog).queryByRole("textbox")).toBeNull();
    expect(await axe(document.body)).toHaveNoViolations(); // a11y: ai drawer turn-on has no violations
  });

  it.each<[string, Role, AiStatusDto | null]>([
    ["a client", "client", demoAiStatus("client")],
    ["a manager of a workspace with AI off", "admin", aiStatusWith("workspace-disabled", { canManage: false })],
    ["anyone on a server without AI", "owner", null],
    ["a member whose self-service is off", "worker", aiStatusWith("self-service-off")],
  ])("is hidden for %s", (_label, role, status) => {
    signIn(role, status);
    render(<Shell />);
    expect(screen.queryByRole("button", { name: "Ask AI" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Ask the setup assistant" })).toBeNull();
  });
});

describe("demo", () => {
  it("answers the suggested questions only, with the composer off, and never touches the network", async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    const user = userEvent.setup();
    signIn("owner", demoAiStatus("owner"), false);
    render(<Shell />);
    const dialog = await open(user);
    const composer = within(dialog).getByRole("textbox", { name: "Your question" });
    expect(composer).toBeDisabled();
    expect(within(dialog).getByText("Sample answers from the demo data")).toBeInTheDocument();
    expect(composer).toHaveAccessibleDescription(/answers the suggested questions/);
    // Focus starts on the first suggested question instead.
    const first = within(dialog).getByRole("button", { name: AI_PROMPT_CHIPS.dashboard[0].label });
    await waitFor(() => expect(first).toHaveFocus());
    await user.click(first);
    expect(await within(dialog).findByText(/^The team tracked .+ in the last 7 days/)).toBeInTheDocument();
    expect(within(dialog).getByText("Sample", { selector: "span" })).toBeInTheDocument();
    expect(askAi).not.toHaveBeenCalled();
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(await axe(document.body)).toHaveNoViolations(); // a11y: ai drawer demo has no violations
  });
});
