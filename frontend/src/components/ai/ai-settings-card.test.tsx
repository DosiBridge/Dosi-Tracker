// The owner's AI assist settings (AI assist spec §8, §18.6): nothing is turned
// on — and no window titles or task notes are included — until the owner has
// read the server's disclosure and ticked the acknowledgement; the save then
// carries the version they read, and a refusal says why in the dialog. Other
// changes save as they are. A server without AI says so instead of offering
// switches; a demo keeps its changes on the page and calls nothing.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { AiSettingsCard } from "./ai-settings-card";
import type { AiAssistantValue } from "./ai-provider";
import { ToastViewport } from "@/components/toast";
import { getAiSettings, getAiUsage, updateAiSettings } from "@/hooks/ai-api";
import type { AiUsageSummaryDto, AiWorkspaceSettingsDto, UpdateAiWorkspaceSettingsDto } from "@/lib/ai-contract";
import { demoAiSettings } from "@/lib/ai-demo-settings";
import { aiApiError, aiContext, aiStatusWith } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";

vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const ctx = vi.hoisted(() => ({ value: null as unknown as AiAssistantValue }));
vi.mock("@/components/ai/ai-provider", async (importOriginal) => {
  const mod = await importOriginal<typeof import("./ai-provider")>();
  return { ...mod, useAiAssistant: () => ctx.value };
});
vi.mock("@/hooks/ai-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/ai-api")>()),
  getAiSettings: vi.fn(),
  getAiUsage: vi.fn(),
  updateAiSettings: vi.fn(),
}));

const DEMO_NOTE = "Demo workspace — nothing was saved.";

function settings(patch: Partial<AiWorkspaceSettingsDto> = {}): AiWorkspaceSettingsDto {
  const base = demoAiSettings();
  return {
    ...base,
    isEnabled: false,
    consentVersion: null,
    enabledAt: null,
    disclosure: {
      ...base.disclosure,
      version: "1.0",
      providerDisplayName: "Example AI",
      processingRegion: "the EU",
      retentionNote: "Requests are kept for 30 days by the processor.",
    },
    ...patch,
  };
}

const usage: AiUsageSummaryDto = {
  monthStart: "2026-07-01T00:00:00Z",
  resetsAt: "2026-08-01T00:00:00Z",
  requestsUsed: 42,
  requestsLimit: 2000,
  tokensUsed: 61_480,
  tokensLimit: null,
  estimatedCostUsd: 1.25,
  byFeature: [{ featureId: "period-summary", requests: 30, tokens: 40_000 }],
  myRequestsToday: 2,
  myDailyLimit: 50,
};

function serve(current: AiWorkspaceSettingsDto) {
  vi.mocked(getAiSettings).mockResolvedValue(current);
  vi.mocked(getAiUsage).mockResolvedValue(usage);
  vi.mocked(updateAiSettings).mockImplementation(async (input: UpdateAiWorkspaceSettingsDto) => ({
    ...current,
    ...input,
    consentVersion: input.acknowledgedDisclosureVersion ?? current.consentVersion,
  }));
}

const refreshStatus = vi.fn();

function renderCard() {
  return render(
    <>
      <AiSettingsCard demoNote={DEMO_NOTE} />
      <ToastViewport />
    </>,
  );
}

const sw = (name: string) => screen.getByRole("switch", { name: new RegExp(name) });

beforeEach(() => {
  resetPrototypeState();
  refreshStatus.mockClear();
  vi.mocked(getAiSettings).mockReset();
  vi.mocked(getAiUsage).mockReset();
  vi.mocked(updateAiSettings).mockReset();
  ctx.value = aiContext({ status: aiStatusWith("workspace-disabled", { workspaceEnabled: false }), refreshStatus });
});

describe("a server without AI", () => {
  it.each([
    ["not configured", aiStatusWith("not-configured")],
    ["switched off for the platform", aiStatusWith("platform-disabled")],
    ["an older server (no status)", null],
  ])("says AI assist isn't available when %s, with no switches", async (_label, status) => {
    ctx.value = aiContext({ status });
    const { container } = renderCard();
    expect(screen.getByRole("note")).toHaveTextContent("AI assist isn't available on this server right now.");
    expect(screen.queryByRole("switch")).toBeNull();
    expect(getAiSettings).not.toHaveBeenCalled();
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai settings unavailable has no violations
  });

  it("waits for the status before saying anything", () => {
    ctx.value = aiContext({ status: null, statusLoading: true });
    renderCard();
    expect(screen.getByRole("status")).toHaveTextContent("Checking AI assist…");
    expect(screen.queryByRole("note")).toBeNull();
  });
});

describe("turning AI assist on", () => {
  it("shows the server's disclosure, keeps Enable off until acknowledged, and sends the version read", async () => {
    const user = userEvent.setup();
    serve(settings());
    renderCard();
    await user.click(await screen.findByRole("switch", { name: /Use AI assist in this workspace/ }));

    const dialog = await screen.findByRole("dialog", { name: "Turn on AI assist" });
    expect(within(dialog).getByText("Example AI")).toBeInTheDocument();
    expect(within(dialog).getByText(/processed in the EU/)).toBeInTheDocument();
    expect(within(dialog).getByText("Requests are kept for 30 days by the processor.")).toBeInTheDocument();
    for (const heading of ["Always sent", "Sent only when you allow it", "Never sent", "Who can use it"]) {
      expect(within(dialog).getByRole("heading", { name: heading })).toBeInTheDocument();
    }
    expect(within(dialog).getByText("Screenshots and webcam images")).toBeInTheDocument();
    expect(within(dialog).getByText(/Every active member gets a notice/)).toBeInTheDocument();
    expect(within(dialog).getByText(/AI output can be wrong/)).toBeInTheDocument();

    const enable = within(dialog).getByRole("button", { name: "Turn on AI assist" });
    expect(enable).toBeDisabled();
    await user.click(enable);
    expect(updateAiSettings).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("checkbox", { name: "I've read this and our organisation may process this data with Example AI" }));
    expect(enable).toBeEnabled();
    await user.click(enable);

    await waitFor(() => expect(updateAiSettings).toHaveBeenCalledTimes(1));
    expect(updateAiSettings).toHaveBeenCalledWith({
      isEnabled: true,
      allowManagers: true,
      allowMemberSelfService: true,
      allowWindowTitles: false,
      allowNotes: false,
      acknowledgedDisclosureVersion: "1.0",
    });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(refreshStatus).toHaveBeenCalledTimes(1);
    expect(sw("Use AI assist")).toHaveAttribute("aria-checked", "true");
    expect(await screen.findByText("AI settings saved")).toBeInTheDocument();
  });

  it("a11y: ai settings enable dialog has no violations", async () => {
    const user = userEvent.setup();
    serve(settings());
    renderCard();
    await user.click(await screen.findByRole("switch", { name: /Use AI assist/ }));
    const dialog = await screen.findByRole("dialog", { name: "Turn on AI assist" });
    expect(await axe(dialog)).toHaveNoViolations();
  });

  it("shows a refused acknowledgement in the dialog, and loads the current disclosure", async () => {
    const user = userEvent.setup();
    serve(settings());
    vi.mocked(updateAiSettings).mockRejectedValue(aiApiError(400, "Tracker:AiDisclosureNotAccepted", { version: "1.1" }));
    renderCard();
    await user.click(await screen.findByRole("switch", { name: /Use AI assist/ }));
    const dialog = await screen.findByRole("dialog", { name: "Turn on AI assist" });
    await user.click(within(dialog).getByRole("checkbox"));
    await user.click(within(dialog).getByRole("button", { name: "Turn on AI assist" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("Review and accept the AI disclosure (version 1.1) to turn on AI assist.");
    await waitFor(() => expect(getAiSettings).toHaveBeenCalledTimes(2));
    expect(refreshStatus).not.toHaveBeenCalled();
    expect(sw("Use AI assist")).toHaveAttribute("aria-checked", "false");
  });

  it("saves nothing when the dialog is cancelled", async () => {
    const user = userEvent.setup();
    serve(settings());
    renderCard();
    await user.click(await screen.findByRole("switch", { name: /Use AI assist/ }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(updateAiSettings).not.toHaveBeenCalled();
  });

  it("asks a changed disclosure to be re-accepted before AI assist runs again", async () => {
    const user = userEvent.setup();
    ctx.value = aiContext({ status: aiStatusWith("reconsent-required"), refreshStatus });
    serve(settings({ isEnabled: true, consentVersion: "0.9" }));
    renderCard();
    const note = await screen.findByText(/The AI disclosure changed \(now version 1\.0\)/);
    expect(sw("Use AI assist")).toHaveAttribute("aria-checked", "false");
    await user.click(within(note.closest("div")!).getByRole("button", { name: "Review and turn on" }));
    const dialog = await screen.findByRole("dialog", { name: "Review the updated AI disclosure" });
    await user.click(within(dialog).getByRole("checkbox"));
    await user.click(within(dialog).getByRole("button", { name: "Accept and turn on" }));
    await waitFor(() => expect(updateAiSettings).toHaveBeenCalledWith(expect.objectContaining({ isEnabled: true, acknowledgedDisclosureVersion: "1.0" })));
  });
});

describe("other changes", () => {
  it("asks before including window titles or task notes, and says what that adds", async () => {
    const user = userEvent.setup();
    serve(settings({ isEnabled: true, consentVersion: "1.0" }));
    ctx.value = aiContext({ refreshStatus });
    renderCard();
    await user.click(await screen.findByRole("switch", { name: /Include window titles/ }));
    const dialog = await screen.findByRole("dialog", { name: "Send more to AI assist" });
    expect(within(dialog).getByRole("note")).toHaveTextContent("This change adds window titles to what may be sent.");
    await user.click(within(dialog).getByRole("checkbox"));
    await user.click(within(dialog).getByRole("button", { name: "Accept and save" }));
    await waitFor(() =>
      expect(updateAiSettings).toHaveBeenCalledWith(expect.objectContaining({ allowWindowTitles: true, acknowledgedDisclosureVersion: "1.0" })),
    );
  });

  it("saves turning off and narrowing who may use it straight away, with no acknowledgement", async () => {
    const user = userEvent.setup();
    serve(settings({ isEnabled: true, consentVersion: "1.0", allowNotes: true }));
    ctx.value = aiContext({ refreshStatus });
    renderCard();
    await user.click(await screen.findByRole("switch", { name: /Managers can use AI assist/ }));
    await waitFor(() => expect(updateAiSettings).toHaveBeenCalledTimes(1));
    expect(vi.mocked(updateAiSettings).mock.calls[0][0]).toMatchObject({ allowManagers: false, acknowledgedDisclosureVersion: null });
    expect(screen.queryByRole("dialog")).toBeNull();

    await user.click(sw("Include task notes"));
    await waitFor(() => expect(updateAiSettings).toHaveBeenCalledTimes(2));
    expect(vi.mocked(updateAiSettings).mock.calls[1][0]).toMatchObject({ allowNotes: false, acknowledgedDisclosureVersion: null });

    await user.click(sw("Use AI assist"));
    await waitFor(() => expect(updateAiSettings).toHaveBeenCalledTimes(3));
    expect(vi.mocked(updateAiSettings).mock.calls[2][0]).toMatchObject({ isEnabled: false, acknowledgedDisclosureVersion: null });
    expect(await screen.findByText("AI assist is off. Nothing is sent to an AI provider.")).toBeInTheDocument();
  });

  it("says when a save failed, in the card", async () => {
    const user = userEvent.setup();
    serve(settings({ isEnabled: true, consentVersion: "1.0" }));
    vi.mocked(updateAiSettings).mockRejectedValue(aiApiError(403, null));
    renderCard();
    await user.click(await screen.findByRole("switch", { name: /Members can use AI on their own data/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("You don't have permission to do that.");
  });
});

describe("loading", () => {
  it("shows the switches, the status line and this month's usage", async () => {
    serve(settings({ isEnabled: true, consentVersion: "1.0", enabledAt: "2026-07-01T09:00:00Z", enabledByName: "Ayesha Rahman" }));
    ctx.value = aiContext({ refreshStatus });
    const { container } = renderCard();
    expect(await screen.findByRole("switch", { name: /Use AI assist/ })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByText(/On since .+, turned on by Ayesha Rahman\./)).toBeInTheDocument();
    expect(sw("Include window titles")).toHaveAttribute("aria-checked", "false");
    expect(screen.getByText("On", { selector: "span" })).toBeInTheDocument();
    expect(await screen.findByText("42 of 2,000")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "AI requests used this month" })).toHaveAttribute("aria-valuenow", "2");
    expect(screen.getByText("61,480 (no monthly limit)")).toBeInTheDocument();
    expect(screen.getByText("$1.25")).toBeInTheDocument();
    expect(screen.getByText("Week summaries")).toBeInTheDocument();
    expect(screen.getByText(/^Renews /)).toBeInTheDocument();
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai settings loaded has no violations
  });

  it("offers Try again when the settings can't be read", async () => {
    const user = userEvent.setup();
    vi.mocked(getAiSettings).mockRejectedValueOnce(aiApiError(500, null)).mockResolvedValueOnce(settings());
    vi.mocked(getAiUsage).mockRejectedValue(aiApiError(500, null));
    renderCard();
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Couldn't load the AI settings. (HTTP 500)");
    await user.click(within(alert).getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("switch", { name: /Use AI assist/ })).toBeInTheDocument();
    expect(screen.queryByText("This month")).toBeNull();
  });
});

describe("demo", () => {
  it("keeps changes on the page, says nothing was saved, and calls nothing", async () => {
    const user = userEvent.setup();
    ctx.value = aiContext({ live: false, refreshStatus });
    const { container } = renderCard();
    expect(screen.getByText("Sample", { selector: "span" })).toBeInTheDocument();
    expect(screen.getByRole("note")).toHaveTextContent(DEMO_NOTE);
    expect(sw("Use AI assist")).toHaveAttribute("aria-checked", "true");
    expect(screen.getByText("42 of 2,000")).toBeInTheDocument();
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai settings demo has no violations

    await user.click(sw("Use AI assist"));
    expect(sw("Use AI assist")).toHaveAttribute("aria-checked", "false");
    expect(await screen.findByText(DEMO_NOTE)).toBeInTheDocument();

    // Turning it back on still shows the disclosure first.
    await user.click(sw("Use AI assist"));
    const dialog = await screen.findByRole("dialog", { name: "Turn on AI assist" });
    await user.click(within(dialog).getByRole("checkbox"));
    await user.click(within(dialog).getByRole("button", { name: "Turn on AI assist" }));
    await waitFor(() => expect(sw("Use AI assist")).toHaveAttribute("aria-checked", "true"));
    expect(getAiSettings).not.toHaveBeenCalled();
    expect(updateAiSettings).not.toHaveBeenCalled();
    expect(refreshStatus).not.toHaveBeenCalled();
  });
});
