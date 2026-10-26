// "Explain this report" (AI assist spec §18.3/§18.5): a collapsed, never
// printed panel under a report's header, for owners and managers only. It
// explains exactly the period and filters on screen (a filter left on "all"
// names nothing), generates only on Generate, and is honest in every state.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { ReportExplainer } from "./report-explainer";
import { useAiPageContext, type AiAssistantValue } from "./ai-provider";
import { requestAiFeature } from "@/hooks/ai-api";
import { AI_COPY } from "@/lib/ai-assist";
import { ALL, type ReportingQuery } from "@/lib/api-contract";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { NOW } from "@/lib/demo-clock";
import { aiApiError, aiContext, aiStatusWith, deferred, reportResult } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";

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

const QUERY: ReportingQuery = {
  from: new Date("2026-07-06T00:00:00.000Z"),
  to: new Date("2026-07-13T00:00:00.000Z"),
  projectId: "p-1",
  userId: ALL,
  timeZone: "Asia/Dhaka",
};

function Report({ query = QUERY }: { query?: ReportingQuery }) {
  return (
    <main>
      <h1>Weekly summary</h1>
      <ReportExplainer slug="weekly" query={query} />
    </main>
  );
}

const toggle = () => screen.getByRole("button", { name: "Explain this report" });

async function openAndGenerate(user: ReturnType<typeof userEvent.setup>) {
  await user.click(toggle());
  await user.click(await screen.findByRole("button", { name: "Generate" }));
}

beforeEach(() => {
  resetPrototypeState();
  ctx.value = aiContext();
  vi.mocked(requestAiFeature).mockReset();
  vi.mocked(useAiPageContext).mockClear();
});

describe("placement", () => {
  it("starts collapsed, never prints, and tells Ask AI which report and period are on screen", () => {
    const { container } = render(<Report />);
    expect(toggle()).toHaveAttribute("aria-expanded", "false");
    expect(container.querySelector(".no-print")).toContainElement(toggle());
    expect(screen.queryByRole("region", { name: "Report explanation" })).toBeNull();
    expect(useAiPageContext).toHaveBeenLastCalledWith({
      page: "reports",
      reportSlug: "weekly",
      from: "2026-07-06T00:00:00.000Z",
      to: "2026-07-13T00:00:00.000Z",
      projectId: "p-1",
      memberId: null,
      timeZone: "Asia/Dhaka",
    });
  });

  it("opens and closes on its toggle", async () => {
    const user = userEvent.setup();
    render(<Report />);
    await user.click(toggle());
    expect(toggle()).toHaveAttribute("aria-expanded", "true");
    expect(await screen.findByRole("region", { name: "Report explanation" })).toBeInTheDocument();
    await user.click(toggle());
    expect(screen.queryByRole("region", { name: "Report explanation" })).toBeNull();
  });

  it.each<[string, Partial<AiAssistantValue>]>([
    ["a member", { status: demoAiStatus("worker"), role: "worker" }],
    ["a client", { status: demoAiStatus("client"), role: "client" }],
    ["no status", { status: null }],
    ["a workspace that hasn't opted in", { status: aiStatusWith("workspace-disabled") }],
  ])("is hidden for %s", (_label, patch) => {
    ctx.value = aiContext(patch);
    render(<Report />);
    expect(screen.queryByRole("button", { name: "Explain this report" })).toBeNull();
    expect(useAiPageContext).toHaveBeenLastCalledWith(null);
  });

  it("a11y: ai card collapsed has no violations", async () => {
    const { container } = render(<Report />);
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("live", () => {
  it("explains exactly the report's period and filters", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValue(reportResult());
    const { container } = render(<Report />);
    await openAndGenerate(user);
    expect(await screen.findByText("This report covers 38 h 12 min of tracked time from 4 members over 5 days.")).toBeInTheDocument();
    expect(vi.mocked(requestAiFeature).mock.calls[0].slice(0, 2)).toEqual([
      "report-explanation",
      {
        reportSlug: "weekly",
        from: "2026-07-06T00:00:00.000Z",
        to: "2026-07-13T00:00:00.000Z",
        projectId: "p-1",
        userId: null,
        timeZone: "Asia/Dhaka",
        dayBasis: null,
        regenerate: false,
      },
    ]);
    expect(screen.getByText("Apollo took 32% of the time.")).toBeInTheDocument();
    expect(screen.getByText("App time counts the window in focus.")).toBeInTheDocument();
    expect(screen.getByText("Questions to explore")).toBeInTheDocument();
    expect(screen.getByText("Explanation ready")).toBeInTheDocument();
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card ready has no violations
  });

  it("loads with Cancel", async () => {
    const user = userEvent.setup();
    const pending = deferred<ReturnType<typeof reportResult>>();
    vi.mocked(requestAiFeature).mockReturnValue(pending.promise);
    const { container } = render(<Report />);
    await openAndGenerate(user);
    expect(screen.getByRole("status")).toHaveTextContent(AI_COPY.loading);
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card loading has no violations
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(vi.mocked(requestAiFeature).mock.calls[0][2]?.signal?.aborted).toBe(true);
  });

  it("says nothing was tracked, and labels a fallback", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValueOnce(reportResult("no-data"));
    const first = render(<Report />);
    await openAndGenerate(user);
    expect((await screen.findAllByText("Nothing was tracked for this report's period.")).length).toBeGreaterThan(0);
    expect(await axe(first.container)).toHaveNoViolations(); // a11y: ai card no-data has no violations
    first.unmount();

    vi.mocked(requestAiFeature).mockResolvedValueOnce(reportResult("fallback"));
    const second = render(<Report />);
    await openAndGenerate(user);
    expect(await screen.findByText(AI_COPY.fallbackBadge, { selector: "span" })).toBeInTheDocument();
    expect(await axe(second.container)).toHaveNoViolations(); // a11y: ai card fallback has no violations
  });

  it("shows an error with Retry, and the allowance note when the quota is used", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockRejectedValueOnce(aiApiError(502, "Tracker:AiOutputInvalid")).mockResolvedValueOnce(reportResult());
    ctx.value = aiContext({ status: aiStatusWith("quota-exhausted") });
    const { container } = render(<Report />);
    await openAndGenerate(user);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("The AI couldn't produce a reliable answer. Try again.");
    expect(screen.getByRole("note")).toHaveTextContent(/AI allowance is used/);
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card error has no violations
    await user.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("Apollo took 32% of the time.")).toBeInTheDocument();
  });
});

describe("demo", () => {
  it("explains the demo report with no network", async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    const user = userEvent.setup();
    ctx.value = aiContext({ live: false });
    const to = new Date(NOW.getTime() + 86_400_000);
    render(<Report query={{ from: new Date(to.getTime() - 14 * 86_400_000), to, projectId: ALL, userId: ALL, timeZone: "UTC" }} />);
    await openAndGenerate(user);
    expect(await screen.findByText(/^This report covers .+ of tracked time/)).toBeInTheDocument();
    expect(screen.getByText(AI_COPY.sampleBadge, { selector: "span" })).toBeInTheDocument();
    expect(fetchSpy).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});
