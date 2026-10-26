// An Ask AI answer (AI assist spec §18.4): text nodes only, "- " lines as a
// list, source chips only for allowlisted routes the viewer may open (an
// unknown route id is dropped, a forbidden page stays plain text, and no link
// ever comes from the answer's own text), "Figures used", and the label that
// it is AI-generated — or a demo sample.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const ctx = vi.hoisted(() => ({ value: null as unknown as AiAssistantValue }));
vi.mock("@/components/ai/ai-provider", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./ai-provider")>()),
  useAiAssistant: () => ctx.value,
}));
vi.mock("@/hooks/ai-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/hooks/ai-api")>()),
  sendAiFeedback: vi.fn(async () => {}),
}));

import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import { AiAnswer } from "./ai-answer";
import type { AiAssistantValue } from "./ai-provider";
import { sendAiFeedback } from "@/hooks/ai-api";
import { AI_ANSWER_LABEL, DEMO_NOTE } from "@/lib/ai-assist";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { aiContext, aiMeta } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";

const TEXT = "The team tracked 38 h 12 min. See https://evil.example for more.\nMost time went to:\n- Apollo: 12 h 5 min\n- <img src=x onerror=alert(1)>";

function Answer(props: Partial<React.ComponentProps<typeof AiAnswer>>) {
  return (
    <main>
      <h1>Ask AI</h1>
      <AiAnswer
        text={TEXT}
        sources={[
          { label: "Weekly summary", routeId: "reports.weekly", routeArg: null },
          { label: "Apollo", routeId: "projects.detail", routeArg: "p-1" },
          { label: "Somewhere else", routeId: "https://evil.example", routeArg: null },
          { label: "Not a report", routeId: "reports.nope", routeArg: null },
          { label: "Help: installing on macOS", routeId: null, routeArg: null },
          { label: "Billing", routeId: "billing", routeArg: null },
        ]}
        facts={[{ id: "F1", label: "Tracked this week", display: "38 h 12 min" }]}
        meta={aiMeta("ask")}
        {...props}
      />
    </main>
  );
}

beforeEach(() => {
  resetPrototypeState();
  ctx.value = aiContext({ role: "admin", status: demoAiStatus("admin") });
  vi.mocked(sendAiFeedback).mockClear();
});

describe("AiAnswer", () => {
  it("renders paragraphs and the '- ' lines as a list, as text only", () => {
    const { container } = render(<Answer />);
    expect(screen.getByText("The team tracked 38 h 12 min. See https://evil.example for more.")).toBeInTheDocument();
    const list = screen.getAllByRole("list")[0];
    expect(within(list).getAllByRole("listitem").map((li) => li.textContent)).toEqual(["Apollo: 12 h 5 min", "<img src=x onerror=alert(1)>"]);
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByText(AI_ANSWER_LABEL)).toBeInTheDocument();
    expect(screen.getByText("AI", { selector: "span" })).toBeInTheDocument();
  });

  it("links only validated routes the viewer may open: unknown ids dropped, forbidden pages as text, none from the answer", () => {
    render(<Answer />);
    const links = screen.getAllByRole("link");
    expect(links.map((a) => [a.textContent, a.getAttribute("href")])).toEqual([
      ["Weekly summary", "/reports/weekly"],
      ["Apollo", "/projects/p-1"],
    ]);
    expect(screen.queryByText("Somewhere else")).toBeNull();
    expect(screen.queryByText("Not a report")).toBeNull();
    expect(screen.getByText("Help: installing on macOS").closest("a")).toBeNull();
    // A manager can't open Billing: the source stays, unlinked.
    expect(screen.getByText("Billing").closest("a")).toBeNull();
  });

  it("shows the figures used, and the feedback and Copy footer for a real answer", async () => {
    const user = userEvent.setup();
    render(<Answer />);
    await user.click(screen.getByText("Figures used"));
    expect(screen.getByText("Tracked this week")).toBeVisible();
    expect(screen.getByText("38 h 12 min")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Helpful" }));
    expect(sendAiFeedback).toHaveBeenCalledWith({ resultId: aiMeta("ask").resultId, rating: "up", reason: null });
    expect(screen.getByRole("button", { name: "Copy" })).toBeInTheDocument();
  });

  it("labels a demo answer as a sample and leaves a clarification without a footer", () => {
    const { unmount } = render(<Answer meta={aiMeta("ask", "demo")} />);
    expect(screen.getByText("Sample", { selector: "span" })).toBeInTheDocument();
    expect(screen.getByText(DEMO_NOTE)).toBeInTheDocument();
    unmount();
    render(<Answer isClarification sources={[]} facts={[]} />);
    expect(screen.queryByRole("button", { name: "Helpful" })).toBeNull();
    expect(screen.queryByText("Figures used")).toBeNull();
    expect(screen.queryByText("Sources")).toBeNull();
  });

  it("a11y: ai answer has no violations", async () => {
    const { container } = render(<Answer />);
    expect(await axe(container)).toHaveNoViolations();
  });
});
