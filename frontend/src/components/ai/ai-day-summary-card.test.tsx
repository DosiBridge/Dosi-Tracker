// "Explain this day" (AI assist spec §18.3/§18.5): owners and managers may
// explain any member's day (the "member" scope), a member only their own
// ("self", sent as the caller: userId null). Generated only on Generate,
// cancellable, honest in every state, sample output in a demo with no
// network; every state passes axe.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { AiDaySummaryCard } from "./ai-day-summary-card";
import { useAiPageContext, type AiAssistantValue } from "./ai-provider";
import { requestAiFeature } from "@/hooks/ai-api";
import { AI_COPY } from "@/lib/ai-assist";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { datasetFor } from "@/lib/tenant-data";
import { aiApiError, aiContext, aiStatusWith, dayResult, deferred } from "@/test/ai-fixtures";
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

function Day({ userId = "member-2", date = "2026-07-14" }: { userId?: string; date?: string }) {
  return (
    <main>
      <h1>Monitor</h1>
      <AiDaySummaryCard userId={userId} date={date} timeZone="Europe/Madrid" />
    </main>
  );
}

const card = () => screen.findByRole("region", { name: "Explain this day" });
const generate = async (user: ReturnType<typeof userEvent.setup>) => user.click(await screen.findByRole("button", { name: "Generate" }));
const settle = () => new Promise((r) => setTimeout(r, 20));

beforeEach(() => {
  resetPrototypeState();
  ctx.value = aiContext();
  vi.mocked(requestAiFeature).mockReset();
  vi.mocked(useAiPageContext).mockClear();
});

describe("who sees it", () => {
  it("lets a manager explain another member's day, sending that member", async () => {
    const user = userEvent.setup();
    ctx.value = aiContext({ status: demoAiStatus("admin"), role: "admin" });
    vi.mocked(requestAiFeature).mockResolvedValue(dayResult());
    render(<Day />);
    await generate(user);
    expect(await screen.findByText("Wendy tracked 6 h 52 min on Tue 14 Jul.")).toBeInTheDocument();
    expect(vi.mocked(requestAiFeature).mock.calls[0].slice(0, 2)).toEqual([
      "member-day-summary",
      { userId: "member-2", date: "2026-07-14", timeZone: "Europe/Madrid", regenerate: false },
    ]);
    expect(useAiPageContext).toHaveBeenLastCalledWith({ page: "monitor", memberId: "member-2", day: "2026-07-14", timeZone: "Europe/Madrid" });
  });

  it("lets a member explain their own day, asked as the caller (userId null)", async () => {
    const user = userEvent.setup();
    ctx.value = aiContext({ status: demoAiStatus("worker"), role: "worker", userId: "me" });
    vi.mocked(requestAiFeature).mockResolvedValue(dayResult());
    render(<Day userId="me" />);
    expect(await screen.findByText(/A short summary of your day/)).toBeInTheDocument();
    await generate(user);
    await screen.findByText("Wendy tracked 6 h 52 min on Tue 14 Jul.");
    expect(vi.mocked(requestAiFeature).mock.calls[0][1]).toMatchObject({ userId: null, date: "2026-07-14" });
  });

  it("is hidden for a member looking at someone else's day, a client, and without a status", async () => {
    for (const patch of [
      { status: demoAiStatus("worker"), role: "worker" as const },
      { status: demoAiStatus("client"), role: "client" as const },
      { status: null },
      { status: aiStatusWith("reconsent-required") },
    ]) {
      ctx.value = aiContext(patch);
      const { unmount } = render(<Day />);
      await settle();
      expect(screen.queryByRole("region")).toBeNull();
      unmount();
    }
    expect(requestAiFeature).not.toHaveBeenCalled();
  });
});

describe("states", () => {
  it("a11y: ai card idle has no violations", async () => {
    const { container } = render(<Day />);
    await card();
    expect(await axe(container)).toHaveNoViolations();
  });

  it("loads with Cancel, and Cancel abandons the request", async () => {
    const user = userEvent.setup();
    const pending = deferred<ReturnType<typeof dayResult>>();
    vi.mocked(requestAiFeature).mockReturnValue(pending.promise);
    const { container } = render(<Day />);
    await generate(user);
    expect(screen.getByRole("status")).toHaveTextContent(AI_COPY.loading);
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card loading has no violations
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(vi.mocked(requestAiFeature).mock.calls[0][2]?.signal?.aborted).toBe(true);
    pending.resolve(dayResult());
    expect(await screen.findByRole("button", { name: "Generate" })).toBeInTheDocument();
  });

  it("shows the day's summary, highlights and caveats", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValue(dayResult());
    const { container } = render(<Day />);
    await generate(user);
    expect(await screen.findByText("Work ran from 09:02 to 17:41. Most time went to Apollo in VS Code.")).toBeInTheDocument();
    expect(screen.getByText("The longest continuous span was 2 h 10 min.")).toBeInTheDocument();
    expect(screen.getByText("Some blocks had no keyboard or mouse measurement.")).toBeInTheDocument();
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card ready has no violations
  });

  it("says nothing was tracked on the day", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValue(dayResult("no-data"));
    const { container } = render(<Day />);
    await generate(user);
    const section = await card();
    expect((await within(section).findAllByText("Nothing was tracked on this day.")).length).toBeGreaterThan(0);
    expect(within(section).queryByText("AI", { selector: "span" })).toBeNull();
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card no-data has no violations
  });

  it("labels the fallback", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockResolvedValue(dayResult("fallback"));
    const { container } = render(<Day />);
    await generate(user);
    expect(await screen.findByText(AI_COPY.fallbackBadge, { selector: "span" })).toBeInTheDocument();
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card fallback has no violations
  });

  it("shows a busy refusal with its wait, and Retry", async () => {
    const user = userEvent.setup();
    vi.mocked(requestAiFeature).mockRejectedValueOnce(aiApiError(429, "Tracker:AiRateLimited", { reason: "busy", retryAfterSeconds: 5 })).mockResolvedValueOnce(dayResult());
    const { container } = render(<Day />);
    await generate(user);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Another AI request is still running. Try again in 5 seconds.");
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card error has no violations
    await user.click(within(alert).getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("Wendy tracked 6 h 52 min on Tue 14 Jul.")).toBeInTheDocument();
  });

  it("keeps Generate with the allowance note when the quota is used", async () => {
    ctx.value = aiContext({ status: aiStatusWith("quota-exhausted") });
    const { container } = render(<Day />);
    expect(await screen.findByRole("note")).toHaveTextContent(/AI allowance is used/);
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai card quota has no violations
  });
});

describe("demo", () => {
  it("explains a demo member's day from the demo data, with no network", async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    const user = userEvent.setup();
    const owner = seededUserId("owner");
    const member = datasetFor("w1").activities.find((a) => a.userId !== owner)!;
    const day = member.endedAt.slice(0, 10);
    ctx.value = aiContext({ live: false, userId: owner });
    render(
      <main>
        <AiDaySummaryCard userId={member.userId} date={day} timeZone="UTC" />
      </main>,
    );
    await generate(user);
    expect(await screen.findByText(/tracked .+ on /)).toBeInTheDocument();
    expect(screen.getByText(AI_COPY.sampleBadge, { selector: "span" })).toBeInTheDocument();
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(requestAiFeature).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});
