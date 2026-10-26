// "This week at a glance" / "My week" / "Project pulse" (AI assist spec
// §18.3/§18.5): shown only when the server allows this caller the period
// summary for the scope, generated only on Generate with the period, scope
// and zone the page shows, cancellable, and honest in every state — no data,
// the fallback, errors with Retry, an exhausted allowance. A demo answers
// from the demo data with no network. Every state passes axe.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { AiPeriodSummaryCard } from "./ai-period-summary-card";
import { useAiPageContext, type AiAssistantValue } from "./ai-provider";
import { requestAiFeature } from "@/hooks/ai-api";
import { AI_COPY } from "@/lib/ai-assist";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { NOW } from "@/lib/demo-clock";
import { zonedPeriod } from "@/lib/time-zone";
import { aiApiError, aiContext, aiStatusWith, deferred, periodResult } from "@/test/ai-fixtures";
import { resetPrototypeState, seededUserId } from "@/test/harness";

vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const ctx = vi.hoisted(() => ({ value: null as unknown as AiAssistantValue }));
vi.mock("@/components/ai/ai-provider", async (importOriginal) => {
  const mod = await importOriginal<typeof import("./ai-provider")>();
  return { ...mod, useAiAssistant: () => ctx.value, useAiPageContext: vi.fn() };
});
vi.mock("@/hooks/ai-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/ai-api")>()),
  requestAiFeature: vi.fn(),
  sendAiFeedback: vi.fn(async () => {}),
}));

const FROM = "2026-07-13T00:00:00.000Z";
const TO = "2026-07-15T00:00:00.000Z";

function Week(props: Partial<React.ComponentProps<typeof AiPeriodSummaryCard>>) {
  return (
    <main>
      <h1>Dashboard</h1>
      <AiPeriodSummaryCard scope="team" from={FROM} to={TO} timeZone="Asia/Dhaka" {...props} />
    </main>
  );
}

const card = (name = "This week at a glance") => screen.findByRole("region", { name });
const generate = async (user: ReturnType<typeof userEvent.setup>) => user.click(await screen.findByRole("button", { name: "Generate" }));

beforeEach(() => {
  resetPrototypeState();
  ctx.value = aiContext();
  vi.mocked(requestAiFeature).mockReset();
  vi.mocked(useAiPageContext).mockClear();
});

describe("who sees it", () => {
  it.each<[string, Partial<AiAssistantValue>, "team" | "self"]>([
    ["no status (older server, failed read)", { status: null }, "team"],
    ["a member asking for the team", { status: demoAiStatus("worker"), role: "worker" }, "team"],
    ["a client", { status: demoAiStatus("client"), role: "client" }, "self"],
    ["a workspace that hasn't opted in", { status: aiStatusWith("workspace-disabled") }, "team"],
    ["a server without AI", { status: aiStatusWith("not-configured") }, "team"],
  ])("hidden for %s", async (_label, patch, scope) => {
    ctx.value = aiContext(patch);
    render(<Week scope={scope} />);
    // Give a lazily loaded card every chance to appear.
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByRole("region")).toBeNull();
    expect(screen.queryByRole("button", { name: "Generate" })).toBeNull();
    expect(useAiPageContext).toHaveBeenLastCalledWith(null);
  });

  it("titles itself by scope: the team's week, my week, a project's pulse", async () => {
    const { unmount } = render(<Week />);
    expect(await card()).toBeInTheDocument();
    unmount();
    ctx.value = aiContext({ status: demoAiStatus("worker"), role: "worker" });
    const second = render(<Week scope="self" />);
    expect(await card("My week")).toBeInTheDocument();
    second.unmount();
    ctx.value = aiContext();
    render(<Week projectId="p-1" />);
    expect(await card("Project pulse")).toBeInTheDocument();
  });

  it("tells Ask AI which period the page shows", async () => {
    render(<Week projectId="p-1" />);
    await card("Project pulse");
    expect(useAiPageContext).toHaveBeenLastCalledWith({ page: "projects", from: FROM, to: TO, projectId: "p-1", timeZone: "Asia/Dhaka" });
  });
});

describe("live", () => {
  it("generates nothing until asked, then sends the period, scope and zone once, with a signal", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValue(periodResult());
    render(<Week />);
    await card();
    expect(requestAiFeature).not.toHaveBeenCalled();
    await generate(user);
    expect(await screen.findByText("The team tracked 38 h 12 min this week.")).toBeInTheDocument();
    expect(requestAiFeature).toHaveBeenCalledTimes(1);
    const [feature, body, init] = vi.mocked(requestAiFeature).mock.calls[0];
    expect(feature).toBe("period-summary");
    expect(body).toEqual({ from: FROM, to: TO, scope: "team", projectId: null, timeZone: "Asia/Dhaka", regenerate: false });
    expect(init?.signal).toBeInstanceOf(AbortSignal);
    // Ready: the AI badge, the next step as a link the owner may open, the caveat, the announcement.
    expect(screen.getByText("AI", { selector: "span" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Open the weekly summary/ })).toHaveAttribute("href", "/reports/weekly");
    expect(screen.getByText("This period includes today.")).toBeInTheDocument();
    expect(screen.getByText(AI_COPY.ready)).toBeInTheDocument();
  });

  it("a11y: ai card ready has no violations", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValue(periodResult());
    const { container } = render(<Week />);
    await generate(user);
    await screen.findByText("The team tracked 38 h 12 min this week.");
    expect(await axe(container)).toHaveNoViolations();
  });

  it("asks a fresh answer on Regenerate", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValue(periodResult());
    render(<Week />);
    await generate(user);
    await user.click(await screen.findByRole("button", { name: "Regenerate" }));
    await waitFor(() => expect(requestAiFeature).toHaveBeenCalledTimes(2));
    expect(vi.mocked(requestAiFeature).mock.calls[1][1]).toMatchObject({ regenerate: true });
  });

  it("shows loading with Cancel, and Cancel abandons the request", async () => {
    const user = userEvent.setup();
    const pending = deferred<ReturnType<typeof periodResult>>();
    vi.mocked(requestAiFeature).mockReturnValue(pending.promise);
    const { container } = render(<Week />);
    await generate(user);
    expect(screen.getByRole("status")).toHaveTextContent(AI_COPY.loading);
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card loading has no violations
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(vi.mocked(requestAiFeature).mock.calls[0][2]?.signal?.aborted).toBe(true);
    pending.resolve(periodResult());
    expect(await screen.findByRole("button", { name: "Generate" })).toBeInTheDocument();
    expect(screen.queryByText("The team tracked 38 h 12 min this week.")).toBeNull();
  });

  it("says nothing was tracked, without the AI badge or Regenerate", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValue(periodResult("no-data"));
    const { container } = render(<Week />);
    await generate(user);
    const section = await card();
    expect(await within(section).findAllByText(AI_COPY.noData)).not.toHaveLength(0);
    expect(within(section).queryByText("AI", { selector: "span" })).toBeNull();
    expect(within(section).queryByRole("button", { name: "Regenerate" })).toBeNull();
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card no-data has no violations
  });

  it("labels the fallback as a basic summary built from the data", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValue(periodResult("fallback"));
    const { container } = render(<Week />);
    await generate(user);
    expect(await screen.findByText(AI_COPY.fallbackBadge, { selector: "span" })).toBeInTheDocument();
    expect(screen.getByText(AI_COPY.fallbackNote)).toBeInTheDocument();
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card fallback has no violations
  });

  it("shows a refusal as an alert, and retries when trying again can help", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockRejectedValueOnce(aiApiError(503, "Tracker:AiProviderUnavailable", { kind: "timeout" })).mockResolvedValueOnce(periodResult());
    const { container } = render(<Week />);
    await generate(user);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("The AI service is slow or unavailable. Try again in a minute.");
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card error has no violations
    await user.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("The team tracked 38 h 12 min this week.")).toBeInTheDocument();
  });

  it("offers no Retry for a refusal that won't change (the range is too long)", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockRejectedValue(aiApiError(400, "Tracker:AiRangeTooLarge", { maxDays: 31 }));
    render(<Week />);
    await generate(user);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Choose 31 days or fewer for AI summaries.");
    expect(within(alert).queryByRole("button", { name: "Retry" })).toBeNull();
  });

  it("keeps Generate on with the allowance note when this month's quota is used", async () => {
    ctx.value = aiContext({ status: aiStatusWith("quota-exhausted") });
    const { container } = render(<Week />);
    expect(await screen.findByRole("note")).toHaveTextContent(/This month's AI allowance is used/);
    expect(screen.getByRole("button", { name: "Generate" })).toBeEnabled();
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card quota has no violations
  });

  it("a11y: ai card idle has no violations", async () => {
    const { container } = render(<Week />);
    await card();
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("demo", () => {
  it("answers from the demo data with a Sample badge, and never touches the network", async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    const user = userEvent.setup();
    const week = zonedPeriod("week", NOW, "UTC");
    ctx.value = aiContext({ live: false, userId: seededUserId("owner") });
    render(<Week from={week.from} to={week.to} timeZone="UTC" />);
    await generate(user);
    expect(await screen.findByText(/^The team tracked .+ in this period/)).toBeInTheDocument();
    expect(screen.getByText(AI_COPY.sampleBadge, { selector: "span" })).toBeInTheDocument();
    expect(requestAiFeature).not.toHaveBeenCalled();
    expect(fetchSpy).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});
