// "Week review" / "Agent health" (data-quality-review, AI assist spec §18.4):
// shown only when the server allows this caller the review for the scope;
// generated only on Generate with the period, scope, zone and checks the
// page asks for; each finding shows its title, explanation, possible
// reasons, its severity as an icon AND words (never colour alone) and an
// action link only where the viewer may open it. A demo answers from the
// demo data with no network. Every state passes axe.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const ctx = vi.hoisted(() => ({ value: null as unknown as AiAssistantValue }));
vi.mock("@/components/ai/ai-provider", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./ai-provider")>()),
  useAiAssistant: () => ctx.value,
  useAiPageContext: vi.fn(),
}));
vi.mock("@/hooks/ai-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/ai-api")>()),
  requestAiFeature: vi.fn(),
  sendAiFeedback: vi.fn(async () => {}),
}));

import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { DataQualityCard } from "./data-quality-card";
import { useAiPageContext, type AiAssistantValue } from "./ai-provider";
import { requestAiFeature } from "@/hooks/ai-api";
import { AGENT_HEALTH_KINDS, AI_COPY, TIMESHEET_DATA_QUALITY_KINDS } from "@/lib/ai-assist";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { NOW } from "@/lib/demo-clock";
import { aiApiError, aiContext, aiStatusWith, dataQualityResult } from "@/test/ai-fixtures";
import { resetPrototypeState, seededUserId } from "@/test/harness";

const FROM = "2026-07-13T00:00:00.000Z";
const TO = "2026-07-20T00:00:00.000Z";

function Review(props: Partial<React.ComponentProps<typeof DataQualityCard>>) {
  return (
    <main>
      <h1>Timesheet</h1>
      <DataQualityCard
        title="Week review"
        purpose="What in this week's tracking is worth a look."
        page="timesheet"
        scope="team"
        kinds={TIMESHEET_DATA_QUALITY_KINDS}
        from={FROM}
        to={TO}
        timeZone="Asia/Dhaka"
        {...props}
      />
    </main>
  );
}

const card = (name = "Week review") => screen.findByRole("region", { name });
const generate = async (user: ReturnType<typeof userEvent.setup>) => user.click(await screen.findByRole("button", { name: "Generate" }));

beforeEach(() => {
  resetPrototypeState();
  ctx.value = aiContext();
  vi.mocked(requestAiFeature).mockReset();
  vi.mocked(useAiPageContext).mockClear();
});

describe("who sees it", () => {
  it.each<[string, Partial<AiAssistantValue>, "team" | "self"]>([
    ["no status", { status: null }, "team"],
    ["a member asking for the team", { status: demoAiStatus("worker"), role: "worker" }, "team"],
    ["a client", { status: demoAiStatus("client"), role: "client" }, "self"],
    ["a workspace that hasn't opted in", { status: aiStatusWith("workspace-disabled") }, "team"],
  ])("hidden for %s", async (_label, patch, scope) => {
    ctx.value = aiContext(patch);
    render(<Review scope={scope} />);
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByRole("region")).toBeNull();
    expect(useAiPageContext).toHaveBeenLastCalledWith(null);
  });

  it("shows a member their own review, and tells Ask AI which period the page shows", async () => {
    ctx.value = aiContext({ status: demoAiStatus("worker"), role: "worker" });
    render(<Review scope="self" />);
    expect(await card()).toBeInTheDocument();
    expect(useAiPageContext).toHaveBeenLastCalledWith({ page: "timesheet", from: FROM, to: TO, timeZone: "Asia/Dhaka" });
  });

  it("a11y: ai card idle has no violations", async () => {
    const { container } = render(<Review />);
    await card();
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("live", () => {
  it("generates nothing until asked, then sends the period, scope, zone and checks once", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValue(dataQualityResult());
    render(<Review title="Agent health" page="team" kinds={AGENT_HEALTH_KINDS} />);
    await card("Agent health");
    expect(requestAiFeature).not.toHaveBeenCalled();
    await generate(user);
    await screen.findByText("Gap in tracked days");
    expect(requestAiFeature).toHaveBeenCalledTimes(1);
    expect(vi.mocked(requestAiFeature).mock.calls[0].slice(0, 2)).toEqual([
      "data-quality-review",
      { from: FROM, to: TO, scope: "team", projectId: null, timeZone: "Asia/Dhaka", kinds: [...AGENT_HEALTH_KINDS], regenerate: false },
    ]);
  });

  it("shows each finding with its reasons, its severity in words, and an action the viewer may open", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValue(dataQualityResult());
    const { container } = render(<Review />);
    await generate(user);
    const items = within(await card()).getAllByRole("listitem").filter((li) => li.className.includes("rounded-xl"));
    expect(items).toHaveLength(2);
    const [gap, software] = items;
    expect(within(gap).getByText("Worth a look")).toBeInTheDocument();
    expect(within(gap).getByText("Gap in tracked days")).toBeInTheDocument();
    expect(within(gap).getByText("· Wendy Worker")).toBeInTheDocument();
    expect(within(gap).getByText("No time was tracked for 3 weekdays in a row.")).toBeInTheDocument();
    expect(within(gap).getByText("The agent wasn't running")).toBeInTheDocument();
    expect(within(gap).getByText("Weekdays without tracked time:")).toBeInTheDocument();
    expect(within(gap).getByRole("link", { name: "Check the desktop agent" })).toHaveAttribute("href", "/download");
    expect(within(software).getByText("For information")).toBeInTheDocument();
    expect(within(software).queryByRole("link")).toBeNull();
    // More were found than shown.
    expect(screen.getByText(/1 more finding was left out/)).toBeInTheDocument();
    expect(screen.getByText("Review ready")).toBeInTheDocument();
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card ready has no violations
  });

  it("drops an action the viewer can't open, and one with an unknown route", async () => {
    const user = userEvent.setup();
    const result = dataQualityResult();
    result.items[0].actionRouteId = "billing";
    result.items[1].actionRouteId = "https://evil.example";
    ctx.value = aiContext({ role: "admin", status: demoAiStatus("admin") });
    vi.mocked(requestAiFeature).mockResolvedValue(result);
    render(<Review />);
    await generate(user);
    await screen.findByText("Gap in tracked days");
    expect(screen.queryByRole("link", { name: /Check the desktop agent|Open/ })).toBeNull();
  });

  it("says when nothing looks unusual, and when nothing was tracked", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValueOnce({ ...dataQualityResult(), items: [], candidateCount: 0 });
    const first = render(<Review />);
    await generate(user);
    expect(await screen.findByText("Nothing in this period's tracking looks unusual.")).toBeInTheDocument();
    first.unmount();
    vi.mocked(requestAiFeature).mockResolvedValueOnce(dataQualityResult("no-data"));
    const { container } = render(<Review from="2026-07-06T00:00:00.000Z" />);
    await generate(user);
    expect(await within(await card()).findAllByText(AI_COPY.noData)).not.toHaveLength(0);
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card no-data has no violations
  });

  it("shows a refusal as an alert, with Retry when trying again can help", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockRejectedValueOnce(aiApiError(503, "Tracker:AiProviderUnavailable")).mockResolvedValueOnce(dataQualityResult());
    const { container } = render(<Review />);
    await generate(user);
    const alert = await screen.findByRole("alert");
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card error has no violations
    await user.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("Gap in tracked days")).toBeInTheDocument();
  });

  it("copies the findings as text", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    vi.mocked(requestAiFeature).mockResolvedValue(dataQualityResult());
    render(<Review />);
    await generate(user);
    await user.click(await screen.findByRole("button", { name: "Copy" }));
    await waitFor(() => expect(writeText).toHaveBeenCalled());
    expect(writeText.mock.calls[0]).toEqual([
      [
        "- Worth a look: Gap in tracked days (Wendy Worker). No time was tracked for 3 weekdays in a row.",
        "  - Possible reason: Leave or a day off",
        "  - Possible reason: The agent wasn't running",
        "- For information: Software-generated input (you). 4 blocks had input generated by software, which is left out of activity %.",
        "  - Possible reason: Remote desktop tools",
      ].join("\n"),
    ]);
  });
});

describe("demo", () => {
  it("reviews the demo data with a Sample badge and no network", async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    const user = userEvent.setup();
    ctx.value = aiContext({ live: false, userId: seededUserId("owner") });
    const to = new Date(NOW.getTime() + 1).toISOString();
    const from = new Date(NOW.getTime() - 14 * 86_400_000).toISOString();
    render(<Review title="Agent health" page="team" kinds={AGENT_HEALTH_KINDS} from={from} to={to} timeZone="UTC" />);
    await generate(user);
    expect(await screen.findByText(AI_COPY.sampleBadge, { selector: "span" })).toBeInTheDocument();
    expect(requestAiFeature).not.toHaveBeenCalled();
    expect(fetchSpy).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});
