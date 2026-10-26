// The host console's AI assist card (AI assist spec §3.1, §18.5): the
// provider's state in words that never name a credential, the switches, the
// breaker, this month's usage against the budgets, the top workspaces, and
// the runtime kill switch — which always asks first, because it stops every
// workspace's AI requests. A demo shows a fixed sample and changes nothing.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { PlatformAiCard } from "./platform-ai-card";
import { ToastViewport } from "@/components/toast";
import { getPlatformAiOverview, updatePlatformAiStatus } from "@/hooks/ai-api";
import type { PlatformAiOverviewDto } from "@/lib/ai-contract";
import { demoWorkspaces } from "@/lib/saas-data";
import { aiApiError } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";

vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const session = vi.hoisted(() => ({ current: null as unknown }));
vi.mock("@/components/session-provider", () => ({ useSession: () => session.current }));
vi.mock("@/hooks/ai-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/ai-api")>()),
  getPlatformAiOverview: vi.fn(),
  updatePlatformAiStatus: vi.fn(),
}));

function overview(patch: Partial<PlatformAiOverviewDto> = {}): PlatformAiOverviewDto {
  return {
    providerState: "configured",
    providerDisplayName: "Example AI",
    defaultModel: "example-small",
    configEnabled: true,
    runtimeEnabled: true,
    breakerState: "closed",
    monthStart: "2026-07-01T00:00:00Z",
    requests: 1234,
    inputTokens: 800_000,
    outputTokens: 200_000,
    estimatedCostUsd: 12.4,
    budgetTokens: 4_000_000,
    budgetUsd: 50,
    fallbackRate: 0.02,
    errorRate: 0.004,
    topWorkspaces: [
      { workspaceId: "w-a", workspaceName: "Acme Studio", requests: 900, tokens: 700_000, estimatedCostUsd: 8.5 },
      { workspaceId: "w-b", workspaceName: "Nimbus Co", requests: 334, tokens: 300_000, estimatedCostUsd: null },
    ],
    ...patch,
  };
}

function renderCard() {
  return render(
    <main>
      <h1>Platform Overview</h1>
      <PlatformAiCard />
      <ToastViewport />
    </main>,
  );
}

const killSwitch = () => screen.getByRole("switch", { name: /Allow AI requests on this platform/ });

beforeEach(() => {
  resetPrototypeState();
  session.current = { isLive: true, workspaces: [] };
  vi.mocked(getPlatformAiOverview).mockReset();
  vi.mocked(updatePlatformAiStatus).mockReset();
});

describe("live", () => {
  it("shows the provider, switches, breaker, usage against budget, and the top workspaces", async () => {
    vi.mocked(getPlatformAiOverview).mockResolvedValue(overview());
    const { container } = renderCard();
    const card = await screen.findByRole("region", { name: "AI assist" });
    expect(await within(card).findByText("Example AI")).toBeInTheDocument();
    expect(within(card).getByText("Connected")).toBeInTheDocument();
    expect(within(card).getByText("example-small")).toBeInTheDocument();
    expect(within(card).getByText("Normal")).toBeInTheDocument();
    expect(within(card).getByText("This month, since 1 Jul")).toBeInTheDocument();
    expect(within(card).getByText("1,234")).toBeInTheDocument();
    expect(within(card).getByText("1,000,000")).toBeInTheDocument();
    expect(within(card).getByText("$12.40")).toBeInTheDocument();
    expect(within(card).getByText("2% · 0.4%")).toBeInTheDocument();
    expect(within(card).getByRole("progressbar", { name: "Token budget used this month" })).toHaveAttribute("aria-valuenow", "25");
    expect(within(card).getByRole("progressbar", { name: "Cost budget used this month" })).toHaveAttribute("aria-valuenow", "25");
    const table = within(card).getByRole("table", { name: "Workspaces using AI assist most this month" });
    expect(within(table).getByRole("rowheader", { name: "Acme Studio" })).toBeInTheDocument();
    expect(within(table).getByText("—")).toBeInTheDocument(); // an unpriced workspace
    expect(killSwitch()).toHaveAttribute("aria-checked", "true");
    expect(await axe(container)).toHaveNoViolations(); // a11y: platform ai card has no violations
  });

  it("asks before turning AI assist off for every workspace, and does nothing on Cancel", async () => {
    const user = userEvent.setup();
    vi.mocked(getPlatformAiOverview).mockResolvedValue(overview());
    vi.mocked(updatePlatformAiStatus).mockResolvedValue({ enabled: false });
    renderCard();
    await screen.findByText("Example AI");
    await user.click(killSwitch());
    const dialog = await screen.findByRole("dialog", { name: "Turn off AI assist for the platform?" });
    expect(within(dialog).getByText(/Every workspace's AI requests stop on their next call/)).toBeInTheDocument();
    expect(await axe(dialog)).toHaveNoViolations(); // a11y: platform ai confirm has no violations
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(updatePlatformAiStatus).not.toHaveBeenCalled();
    expect(killSwitch()).toHaveAttribute("aria-checked", "true");

    await user.click(killSwitch());
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Turn off AI assist" }));
    await waitFor(() => expect(updatePlatformAiStatus).toHaveBeenCalledWith({ enabled: false }));
    await waitFor(() => expect(killSwitch()).toHaveAttribute("aria-checked", "false"));
    expect(screen.getByText("Off", { selector: "span" })).toBeInTheDocument();
    expect(await screen.findByText("AI assist is off for the platform")).toBeInTheDocument();
  });

  it("asks before turning it back on, too", async () => {
    const user = userEvent.setup();
    vi.mocked(getPlatformAiOverview).mockResolvedValue(overview({ runtimeEnabled: false }));
    vi.mocked(updatePlatformAiStatus).mockResolvedValue({ enabled: true });
    renderCard();
    await screen.findByText("Example AI");
    await user.click(killSwitch());
    await user.click(within(await screen.findByRole("dialog", { name: "Turn AI assist back on?" })).getByRole("button", { name: "Turn on AI assist" }));
    await waitFor(() => expect(updatePlatformAiStatus).toHaveBeenCalledWith({ enabled: true }));
    await waitFor(() => expect(killSwitch()).toHaveAttribute("aria-checked", "true"));
  });

  it("says when the switch couldn't be changed", async () => {
    const user = userEvent.setup();
    vi.mocked(getPlatformAiOverview).mockResolvedValue(overview());
    vi.mocked(updatePlatformAiStatus).mockRejectedValue(aiApiError(500, null));
    renderCard();
    await screen.findByText("Example AI");
    await user.click(killSwitch());
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Turn off AI assist" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't change the platform's AI switch.");
    expect(killSwitch()).toHaveAttribute("aria-checked", "true");
  });

  it.each<[PlatformAiOverviewDto["providerState"], string]>([
    ["missing-api-key", "The AI provider connection isn't set up"],
    ["missing-model", "The AI provider connection names no model"],
    ["invalid-endpoint", "The AI provider connection's address isn't valid"],
    ["unsupported-provider", "This AI provider isn't supported"],
    ["disabled", "Turned off in the server configuration"],
  ])("explains the provider state %s without naming a credential", async (providerState, words) => {
    vi.mocked(getPlatformAiOverview).mockResolvedValue(overview({ providerState, configEnabled: providerState === "configured", providerDisplayName: null }));
    renderCard();
    const label = await screen.findByText(words);
    expect(label.textContent).not.toMatch(/secret|password|api[_-]?key/i);
    expect(screen.getByText("None")).toBeInTheDocument();
    expect(screen.getByText("Has no effect until AI assist is enabled in the server configuration.")).toBeInTheDocument();
  });

  it("reads the breaker and an empty month plainly", async () => {
    vi.mocked(getPlatformAiOverview).mockResolvedValue(overview({ breakerState: "open", topWorkspaces: [], budgetTokens: null, budgetUsd: null }));
    renderCard();
    expect(await screen.findByText("Paused after repeated provider failures")).toBeInTheDocument();
    expect(screen.getByText("No workspace has used AI assist this month.")).toBeInTheDocument();
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("says a server without AI assist has none, and offers Try again on a failed read", async () => {
    const user = userEvent.setup();
    vi.mocked(getPlatformAiOverview).mockRejectedValueOnce(aiApiError(404, null));
    const first = renderCard();
    expect(await screen.findByText("This server has no AI assist.")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
    first.unmount();

    vi.mocked(getPlatformAiOverview).mockRejectedValueOnce(aiApiError(500, null)).mockResolvedValueOnce(overview());
    renderCard();
    const alert = await screen.findByRole("alert");
    await user.click(within(alert).getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("Example AI")).toBeInTheDocument();
  });

  it("says it is loading", () => {
    vi.mocked(getPlatformAiOverview).mockReturnValue(new Promise(() => {}));
    renderCard();
    expect(screen.getByRole("status")).toHaveTextContent("Loading AI assist's figures…");
  });
});

describe("demo", () => {
  it("shows a fixed sample, changes only the page, and calls nothing", async () => {
    const user = userEvent.setup();
    session.current = { isLive: false, workspaces: demoWorkspaces() };
    const { container } = renderCard();
    expect(screen.getByText("Sample", { selector: "span" })).toBeInTheDocument();
    const table = screen.getByRole("table");
    expect(within(table).getAllByRole("row").length).toBeGreaterThan(1);
    expect(await axe(container)).toHaveNoViolations(); // a11y: platform ai demo has no violations
    await user.click(killSwitch());
    await user.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Turn off AI assist" }));
    await waitFor(() => expect(killSwitch()).toHaveAttribute("aria-checked", "false"));
    expect(await screen.findByText("Nothing was changed: the demo has no AI provider.")).toBeInTheDocument();
    expect(getPlatformAiOverview).not.toHaveBeenCalled();
    expect(updatePlatformAiStatus).not.toHaveBeenCalled();
  });
});
