// The dashboard's prompt about AI assist itself (AI assist spec §18.3): the
// owner of a workspace that hasn't opted in (or must re-accept a changed
// disclosure) gets "Turn on AI assist" → Settings > Workspace; a manager
// learns the owner can turn it on; members, clients, a server without AI and
// a workspace already on see nothing. Dismissible, remembered per viewer
// through use-stored-choice. Also: the cards download only when usable.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { AI_CTA_KEY, AiEnableCta } from "./ai-enable-cta";
import { AiCardsPreload } from "./ai-cards-preload";
import type { AiAssistantValue } from "./ai-provider";
import { clearStoredChoices } from "@/hooks/use-stored-choice";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { aiContext, aiStatusWith } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";

vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const ctx = vi.hoisted(() => ({ value: null as unknown as AiAssistantValue }));
vi.mock("@/components/ai/ai-provider", async (importOriginal) => {
  const mod = await importOriginal<typeof import("./ai-provider")>();
  return { ...mod, useAiAssistant: () => ctx.value };
});
const cards = vi.hoisted(() => ({ load: vi.fn(async () => ({})) }));
vi.mock("@/components/ai/use-ai-cards", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./use-ai-cards")>()),
  loadAiCards: cards.load,
}));

beforeEach(() => {
  resetPrototypeState();
  clearStoredChoices();
  cards.load.mockClear();
});

describe("AiEnableCta", () => {
  it("invites the owner to turn AI assist on, linking to Settings > Workspace", async () => {
    ctx.value = aiContext({ status: aiStatusWith("workspace-disabled", { workspaceEnabled: false }) });
    const { container } = render(<AiEnableCta />);
    expect(screen.getByText("Turn on AI assist", { selector: "p" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Turn on AI assist" })).toHaveAttribute("href", "/settings?tab=workspace");
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai cta owner has no violations
  });

  it("asks the owner to review a changed disclosure", () => {
    ctx.value = aiContext({ status: aiStatusWith("reconsent-required") });
    render(<AiEnableCta />);
    expect(screen.getByText("Review the updated AI disclosure")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Review and turn on" })).toHaveAttribute("href", "/settings?tab=workspace");
  });

  it("tells a manager the owner can turn it on, with no link they can't use", async () => {
    ctx.value = aiContext({ role: "admin", status: aiStatusWith("workspace-disabled", { canManage: false }) });
    const { container } = render(<AiEnableCta />);
    expect(screen.getByText("Your workspace owner can turn on AI assist")).toBeInTheDocument();
    expect(screen.queryByRole("link")).toBeNull();
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai cta manager has no violations
  });

  it.each<[string, Partial<AiAssistantValue>]>([
    ["AI assist is on", { status: demoAiStatus("owner") }],
    ["the server has no AI", { status: aiStatusWith("not-configured") }],
    ["no status", { status: null }],
    ["a member", { role: "worker", status: aiStatusWith("workspace-disabled", { canManage: false }) }],
    ["a client", { role: "client", status: aiStatusWith("workspace-disabled", { canManage: false }) }],
  ])("says nothing when %s", (_label, patch) => {
    ctx.value = aiContext(patch);
    const { container } = render(<AiEnableCta />);
    expect(container).toBeEmptyDOMElement();
  });

  it("stays dismissed for this viewer once dismissed", async () => {
    const user = userEvent.setup();
    ctx.value = aiContext({ status: aiStatusWith("workspace-disabled") });
    const { unmount } = render(<AiEnableCta />);
    await user.click(screen.getByRole("button", { name: "Dismiss the AI assist note" }));
    expect(screen.queryByText("Turn on AI assist", { selector: "p" })).toBeNull();
    expect(localStorage.getItem(AI_CTA_KEY)).toBe("dismissed");
    unmount();
    render(<AiEnableCta />);
    expect(screen.queryByText("Turn on AI assist", { selector: "p" })).toBeNull();
  });
});

describe("AiCardsPreload", () => {
  it("downloads the cards once AI assist is usable here, and renders nothing", () => {
    ctx.value = aiContext();
    const { container } = render(<AiCardsPreload />);
    expect(container).toBeEmptyDOMElement();
    expect(cards.load).toHaveBeenCalledTimes(1);
  });

  it("keeps downloading the cards while only the allowance is used up", () => {
    ctx.value = aiContext({ status: aiStatusWith("quota-exhausted") });
    render(<AiCardsPreload />);
    expect(cards.load).toHaveBeenCalledTimes(1);
  });

  it.each<[string, Partial<AiAssistantValue>]>([
    ["no status", { status: null }],
    ["a workspace that hasn't opted in", { status: aiStatusWith("workspace-disabled") }],
    ["a client", { status: demoAiStatus("client"), role: "client" }],
  ])("downloads nothing for %s", (_label, patch) => {
    ctx.value = aiContext(patch);
    render(<AiCardsPreload />);
    expect(cards.load).not.toHaveBeenCalled();
  });
});
