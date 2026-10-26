// The AI card shell (AI assist spec §18.3/§18.4/§18.8): every state renders
// what the person needs and nothing more, the badge is visible text, loading
// and completion are announced politely, errors are alerts with Retry when it
// helps, feedback reaches the server only for live results, links appear only
// where the viewer's role may go, and every state passes axe.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { AiInsightCard, AI_SLOW_AFTER_MS } from "./ai-insight-card";
import { AiResultBody, aiResultText } from "./ai-result-body";
import { AiUnavailableNote } from "./ai-unavailable-note";
import type { AiAssistantValue } from "./ai-provider";
import type { AiRequest, AiRequestState } from "@/hooks/use-ai-request";
import { useAiRequest } from "@/hooks/use-ai-request";
import { sendAiFeedback } from "@/hooks/ai-api";
import type { AiPeriodSummaryDto, AiResultStatus } from "@/lib/ai-contract";
import { AI_COPY, DEMO_NOTE, DISCLAIMER, aiAvailability } from "@/lib/ai-assist";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { DAY_MS, NOW } from "@/lib/demo-clock";
import { browserTimeZone, formatInZone } from "@/lib/time-zone";
import { resetPrototypeState } from "@/test/harness";

vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const ctx = vi.hoisted(() => ({ value: null as unknown as AiAssistantValue }));
vi.mock("@/components/ai/ai-provider", async (importOriginal) => {
  const mod = await importOriginal<typeof import("./ai-provider")>();
  return { ...mod, useAiAssistant: () => ctx.value };
});
vi.mock("@/hooks/ai-api", async (importOriginal) => {
  const mod = await importOriginal<typeof import("@/hooks/ai-api")>();
  return { ...mod, sendAiFeedback: vi.fn(async () => {}) };
});

function context(patch: Partial<AiAssistantValue> = {}): AiAssistantValue {
  return {
    status: demoAiStatus("owner"),
    statusLoading: false,
    refreshStatus: () => {},
    live: true,
    role: "owner",
    userId: "u1",
    workspaceId: "w1",
    cache: { get: () => undefined, set: () => {} },
    drawerOpen: false,
    setDrawerOpen: () => {},
    openAssistant: () => {},
    drawerRequest: { seq: 0, chips: "page", prompt: null },
    pageContext: null,
    ...patch,
  };
}

function summary(status: AiResultStatus = "generated"): AiPeriodSummaryDto {
  return {
    meta: {
      resultId: "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
      featureId: "period-summary",
      status,
      generatedAt: "2026-07-14T09:05:00Z",
      model: "m",
      promptVersion: "period-summary@1",
      dataCategories: ["durations", "app-names"],
      disclaimer: DISCLAIMER,
    },
    headline: "The team tracked 38 h 12 min this week.",
    insights: [
      { kind: "distribution", text: "Apollo took the most time.", facts: [{ id: "F1", label: "Time on Apollo", display: "12 h 5 min" }], routeId: "team" },
      { kind: "coverage", text: "Activity was 71%.", facts: [{ id: "F2", label: "Activity %", display: "71%" }, { id: "F1", label: "Time on Apollo", display: "12 h 5 min" }], routeId: "nowhere" },
    ],
    nextSteps: [
      { label: "Open the weekly summary", routeId: "reports.weekly" },
      { label: "Review billing", routeId: "billing" },
    ],
    caveats: ["This period includes today."],
  };
}

function request(state: AiRequestState<AiPeriodSummaryDto>) {
  return { state, run: vi.fn<AiRequest<AiPeriodSummaryDto>["run"]>(), cancel: vi.fn<AiRequest<AiPeriodSummaryDto>["cancel"]>() };
}

function Card({ req, availability }: { req: AiRequest<AiPeriodSummaryDto>; availability?: ReturnType<typeof aiAvailability> }) {
  return (
    <main>
      <h1>Dashboard</h1>
      <AiInsightCard
        title="This week at a glance"
        purpose="A short summary of this week's tracked time."
        request={req}
        availability={availability}
        renderResult={(r) => <AiResultBody headline={r.headline} items={r.insights} nextSteps={r.nextSteps} caveats={r.caveats} />}
        resultText={(r) => aiResultText({ headline: r.headline, items: r.insights, nextSteps: r.nextSteps, caveats: r.caveats })}
      />
    </main>
  );
}

const badge = (label: string) => screen.getByText(label, { selector: "span" });

beforeEach(() => {
  resetPrototypeState();
  ctx.value = context();
  vi.mocked(sendAiFeedback).mockClear();
  vi.mocked(sendAiFeedback).mockResolvedValue(undefined);
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("idle", () => {
  it("says what it does, offers What's sent? and Generate, and runs only when pressed", async () => {
    const user = userEvent.setup();
    const req = request({ phase: "idle" });
    render(<Card req={req} />);
    const section = screen.getByRole("region", { name: "This week at a glance" });
    expect(within(section).getByRole("heading", { name: "This week at a glance" })).toBeInTheDocument();
    expect(badge("AI")).toBeInTheDocument();
    expect(screen.getByText("A short summary of this week's tracked time.")).toBeInTheDocument();
    expect(req.run).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Generate" }));
    expect(req.run).toHaveBeenCalledWith(false);
  });

  it("shows what is and isn't sent", async () => {
    const user = userEvent.setup();
    render(<Card req={request({ phase: "idle" })} />);
    await user.click(screen.getByRole("button", { name: "What's sent?" }));
    const panel = await screen.findByRole("dialog", { name: "What AI assist sends" });
    expect(within(panel).getByText("Never sent")).toBeInTheDocument();
    expect(within(panel).getByText("Screenshots and webcam images")).toBeInTheDocument();
    expect(within(panel).getByRole("link", { name: "Privacy details" })).toHaveAttribute("href", "/settings?tab=privacy");
  });

  it("marks a demo card as a sample before anything is generated", () => {
    ctx.value = context({ live: false });
    render(<Card req={request({ phase: "idle" })} />);
    expect(badge("Sample")).toBeInTheDocument();
    expect(screen.queryByText("AI", { selector: "span" })).toBeNull();
  });

  it("keeps Generate on with the allowance note when the quota is used", () => {
    const status = { ...demoAiStatus("owner"), available: false, reason: "quota-exhausted" as const };
    ctx.value = context({ status });
    render(<Card req={request({ phase: "idle" })} availability={aiAvailability(status, "period-summary", "team")} />);
    expect(screen.getByRole("note")).toHaveTextContent(/This month's AI allowance is used; only summaries generated earlier can be shown\. Renews /);
    expect(screen.getByRole("button", { name: "Generate" })).toBeEnabled();
  });

  it("a11y: ai card idle has no violations", async () => {
    const { container } = render(<Card req={request({ phase: "idle" })} />);
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("loading", () => {
  it("shows a hidden skeleton, announces politely, and offers Cancel", async () => {
    const user = userEvent.setup();
    const req = request({ phase: "loading" });
    const { container } = render(<Card req={req} />);
    const status = screen.getByRole("status");
    expect(status).toHaveTextContent("Writing summary...");
    expect(status).toHaveAttribute("aria-live", "polite");
    expect(container.querySelector('[aria-hidden="true"].space-y-2')).not.toBeNull();
    expect(screen.queryByRole("button", { name: "Generate" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(req.cancel).toHaveBeenCalled();
  });

  it("says it can still be cancelled after 10 seconds", () => {
    vi.useFakeTimers();
    render(<Card req={request({ phase: "loading" })} />);
    act(() => vi.advanceTimersByTime(AI_SLOW_AFTER_MS - 1));
    expect(screen.getByRole("status")).toHaveTextContent(/^Writing summary\.\.\.$/);
    act(() => vi.advanceTimersByTime(1));
    expect(screen.getByRole("status")).toHaveTextContent("Writing summary... Still working - you can cancel.");
  });

  it("a11y: ai card loading has no violations", async () => {
    const { container } = render(<Card req={request({ phase: "loading" })} />);
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("ready", () => {
  it("shows the result, its figures and footer, and announces it without moving focus", async () => {
    const user = userEvent.setup();
    const req = request({ phase: "ready", result: summary(), reused: false });
    render(<Card req={req} />);
    expect(badge("AI")).toBeInTheDocument();
    expect(screen.getByText("The team tracked 38 h 12 min this week.")).toBeInTheDocument();
    expect(screen.getByText("This period includes today.")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Summary ready");
    expect(document.activeElement).toBe(document.body);
    expect(screen.getByText(DISCLAIMER)).toBeInTheDocument();
    const at = formatInZone(Date.parse("2026-07-14T09:05:00Z"), browserTimeZone(), { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
    expect(screen.getByText(`Generated ${at}`)).toBeInTheDocument();

    // The figures, once each.
    await user.click(screen.getByText("Show the figures used"));
    expect(screen.getAllByText("Time on Apollo")).toHaveLength(1);
    expect(screen.getByText("71%", { selector: "dd" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Regenerate" }));
    expect(req.run).toHaveBeenCalledWith(true);
  });

  it("links only where the viewer may go, and drops unknown route ids", () => {
    render(<Card req={request({ phase: "ready", result: summary(), reused: false })} />);
    expect(screen.getAllByRole("link", { name: "Open" })).toHaveLength(1);
    expect(screen.getByRole("link", { name: "Open" })).toHaveAttribute("href", "/team");
    expect(screen.getByRole("link", { name: "Open the weekly summary" })).toHaveAttribute("href", "/reports/weekly");
    expect(screen.getByRole("link", { name: "Review billing" })).toHaveAttribute("href", "/billing");
  });

  it("hides links a member's role can't open", () => {
    ctx.value = context({ role: "worker", status: demoAiStatus("worker") });
    render(<Card req={request({ phase: "ready", result: summary(), reused: false })} />);
    expect(screen.queryByRole("link", { name: "Open" })).toBeNull();
    expect(screen.queryByRole("link", { name: "Open the weekly summary" })).toBeNull();
    expect(screen.queryByRole("link", { name: "Review billing" })).toBeNull();
    expect(screen.queryByText("Next steps")).toBeNull();
  });

  it("says when a result was reused", () => {
    const { rerender } = render(<Card req={request({ phase: "ready", result: summary("cached"), reused: false })} />);
    expect(screen.getByText(/Generated \d\d:\d\d, reused, no new data/)).toBeInTheDocument();
    rerender(<Card req={request({ phase: "ready", result: summary(), reused: true })} />);
    expect(screen.getByText(/reused, no new data/)).toBeInTheDocument();
  });

  it("a11y: ai card ready has no violations", async () => {
    const { container } = render(<Card req={request({ phase: "ready", result: summary(), reused: false })} />);
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("no-data, fallback and demo results", () => {
  it("says nothing was tracked, with no AI badge and no footer", () => {
    render(<Card req={request({ phase: "ready", result: summary("no-data"), reused: false })} />);
    expect(screen.getAllByText(AI_COPY.noData)).not.toHaveLength(0);
    expect(screen.queryByText("AI", { selector: "span" })).toBeNull();
    expect(screen.queryByText(DISCLAIMER)).toBeNull();
    expect(screen.queryByRole("button", { name: "Regenerate" })).toBeNull();
  });

  it("a11y: ai card no-data has no violations", async () => {
    const { container } = render(<Card req={request({ phase: "ready", result: summary("no-data"), reused: false })} />);
    expect(await axe(container)).toHaveNoViolations();
  });

  it("labels the fallback as a basic summary built from the data", () => {
    render(<Card req={request({ phase: "ready", result: summary("fallback"), reused: false })} />);
    expect(badge("Basic summary")).toBeInTheDocument();
    expect(screen.getByText(AI_COPY.fallbackNote)).toBeInTheDocument();
  });

  it("a11y: ai card fallback has no violations", async () => {
    const { container } = render(<Card req={request({ phase: "ready", result: summary("fallback"), reused: false })} />);
    expect(await axe(container)).toHaveNoViolations();
  });

  it("labels demo output as a sample, says nothing was sent, and keeps feedback local", async () => {
    const user = userEvent.setup();
    ctx.value = context({ live: false });
    render(<Card req={request({ phase: "ready", result: summary("demo"), reused: false })} />);
    expect(badge("Sample")).toBeInTheDocument();
    expect(screen.getByText(DEMO_NOTE)).toBeInTheDocument();
    expect(screen.queryByText(DISCLAIMER)).toBeNull();
    await user.click(screen.getByRole("button", { name: /Helpful/ }));
    expect(sendAiFeedback).not.toHaveBeenCalled();
    expect(screen.getByText("Thanks for the feedback.")).toBeInTheDocument();
  });

  it("a11y: ai card demo has no violations", async () => {
    ctx.value = context({ live: false });
    const { container } = render(<Card req={request({ phase: "ready", result: summary("demo"), reused: false })} />);
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("error", () => {
  const retryable = { kind: "rate-limited" as const, message: "Too many AI requests. Try again in 20 seconds.", retryAfterSeconds: 20, resetsAt: null, retryable: true };

  it("is an alert with Retry when trying again can help", async () => {
    const user = userEvent.setup();
    const req = request({ phase: "error", error: retryable });
    render(<Card req={req} />);
    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent(retryable.message);
    await user.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(req.run).toHaveBeenCalledWith(false);
  });

  it("has no Retry when retrying can't help", () => {
    render(<Card req={request({ phase: "error", error: { ...retryable, kind: "feature-disabled", message: "This AI feature isn't available to you.", retryable: false } })} />);
    expect(screen.getByRole("alert")).toHaveTextContent("This AI feature isn't available to you.");
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
  });

  it("a11y: ai card error has no violations", async () => {
    const { container } = render(<Card req={request({ phase: "error", error: retryable })} />);
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("footer", () => {
  it("sends Helpful and Not helpful (with a reason) for a live result, with pressed states", async () => {
    const user = userEvent.setup();
    render(<Card req={request({ phase: "ready", result: summary(), reused: false })} />);
    const helpful = screen.getByRole("button", { name: "Helpful" });
    const notHelpful = screen.getByRole("button", { name: "Not helpful" });
    expect(helpful).toHaveAttribute("aria-pressed", "false");
    await user.click(helpful);
    expect(helpful).toHaveAttribute("aria-pressed", "true");
    expect(sendAiFeedback).toHaveBeenLastCalledWith({ resultId: summary().meta.resultId, rating: "up", reason: null });

    expect(screen.queryByRole("group", { name: "What was wrong?" })).toBeNull();
    await user.click(notHelpful);
    expect(notHelpful).toHaveAttribute("aria-pressed", "true");
    expect(helpful).toHaveAttribute("aria-pressed", "false");
    expect(sendAiFeedback).toHaveBeenLastCalledWith({ resultId: summary().meta.resultId, rating: "down", reason: null });
    const reasons = screen.getByRole("group", { name: "What was wrong?" });
    await user.click(within(reasons).getByRole("button", { name: "Tone" }));
    expect(within(reasons).getByRole("button", { name: "Tone" })).toHaveAttribute("aria-pressed", "true");
    expect(sendAiFeedback).toHaveBeenLastCalledWith({ resultId: summary().meta.resultId, rating: "down", reason: "tone" });
  });

  it("says when feedback couldn't be sent", async () => {
    const user = userEvent.setup();
    vi.mocked(sendAiFeedback).mockRejectedValueOnce(new Error("offline"));
    render(<Card req={request({ phase: "ready", result: summary(), reused: false })} />);
    await user.click(screen.getByRole("button", { name: "Helpful" }));
    expect(await screen.findByText("Your feedback couldn't be sent.")).toBeInTheDocument();
  });

  it("a11y: ai card feedback reasons have no violations", async () => {
    const user = userEvent.setup();
    const { container } = render(<Card req={request({ phase: "ready", result: summary(), reused: false })} />);
    await user.click(screen.getByRole("button", { name: "Not helpful" }));
    expect(await axe(container)).toHaveNoViolations();
  });

  it("copies the result as plain text", async () => {
    const user = userEvent.setup();
    render(<Card req={request({ phase: "ready", result: summary(), reused: false })} />);
    await user.click(screen.getByRole("button", { name: "Copy" }));
    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
    expect(await navigator.clipboard.readText()).toBe(
      [
        "The team tracked 38 h 12 min this week.",
        "- Apollo took the most time.",
        "- Activity was 71%.",
        "Next steps:",
        "- Open the weekly summary",
        "- Review billing",
        "Keep in mind:",
        "- This period includes today.",
      ].join("\n"),
    );
  });

  it("says when the browser blocks the clipboard", async () => {
    const user = userEvent.setup();
    render(<Card req={request({ phase: "ready", result: summary(), reused: false })} />);
    vi.spyOn(navigator.clipboard, "writeText").mockRejectedValueOnce(new Error("denied"));
    await user.click(screen.getByRole("button", { name: "Copy" }));
    expect(await screen.findByText("Couldn't copy: your browser blocked clipboard access.")).toBeInTheDocument();
  });
});

describe("with the real request hook in demo mode", () => {
  function DemoCard() {
    const from = new Date(NOW.getTime() - 7 * DAY_MS).toISOString();
    const req = useAiRequest("period-summary", { from, to: NOW.toISOString(), scope: "team" }, { live: false, workspaceId: "w1" });
    return <Card req={req} />;
  }

  it("generates a sample from the demo data without any request", async () => {
    const user = userEvent.setup();
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    ctx.value = context({ live: false });
    render(<DemoCard />);
    await user.click(screen.getByRole("button", { name: "Generate" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Summary ready"));
    expect(screen.getByText(/^The team tracked /)).toBeInTheDocument();
    expect(badge("Sample")).toBeInTheDocument();
    expect(fetchSpy).not.toHaveBeenCalled();
  });
});

describe("AiUnavailableNote", () => {
  it("says the server has no AI assist, as a note", async () => {
    const { container } = render(<AiUnavailableNote />);
    expect(screen.getByRole("note")).toHaveTextContent("AI assist isn't available on this server right now.");
    expect(await axe(container)).toHaveNoViolations();
  });
});
