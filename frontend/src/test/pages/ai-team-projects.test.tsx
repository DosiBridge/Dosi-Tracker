// AI assist on the team and project pages, in the DEMO (AI assist spec §18.5,
// §18.7), through the real session and AI assist providers: "Agent health"
// sits above the roster for the owner and managers only; a project's page
// offers "Project pulse" near "Recent activity" for whoever sees the team's
// figures, and "Draft tasks with AI" where tasks can be added. Every answer
// is the demo's sample, and nothing touches the network.
const params = vi.hoisted(() => ({ id: "p1" }));
vi.mock("next/navigation", async () => ({
  ...(await import("@/test/next-navigation-stub")),
  useParams: () => params,
}));

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import TeamPage from "@/app/(dashboard)/team/page";
import ProjectDetailPage from "@/app/(dashboard)/projects/[id]/page";
import { AiAssistantProvider } from "@/components/ai/ai-provider";
import { AI_COPY } from "@/lib/ai-assist";
import type { Role } from "@/lib/types";
import { renderAsRole, resetPrototypeState } from "@/test/harness";

let fetchSpy: ReturnType<typeof vi.fn>;

function renderPage(page: React.ReactNode, role: Role) {
  return renderAsRole(<AiAssistantProvider>{page}</AiAssistantProvider>, role);
}

beforeEach(() => {
  resetPrototypeState();
  params.id = "p1";
  fetchSpy = vi.fn();
  vi.stubGlobal("fetch", fetchSpy);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("Agent health (team page)", () => {
  it("sits above the roster for the owner and reviews the demo's agents with no network", async () => {
    const user = userEvent.setup();
    renderPage(<TeamPage />, "owner");
    const card = await screen.findByRole("region", { name: "Agent health" });
    expect(card.compareDocumentPosition(screen.getByPlaceholderText("Search members…")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    await user.click(within(card).getByRole("button", { name: "Generate" }));
    expect(await within(card).findByText("Review ready")).toBeInTheDocument();
    expect(within(card).getByText(AI_COPY.sampleBadge, { selector: "span" })).toBeInTheDocument();
    expect(within(card).getByText(/^Demo - sample AI output/)).toBeInTheDocument();
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("is offered to a manager, never to a member", async () => {
    const first = renderPage(<TeamPage />, "admin");
    expect(await screen.findByRole("region", { name: "Agent health" })).toBeInTheDocument();
    first.unmount();
    renderPage(<TeamPage />, "worker");
    await screen.findByRole("heading", { name: "Team" });
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByRole("region", { name: "Agent health" })).toBeNull();
  });
});

describe("Project pulse and task drafts (project page)", () => {
  it("offers the owner the project's pulse before Recent activity, from the demo data", async () => {
    const user = userEvent.setup();
    renderPage(<ProjectDetailPage />, "owner");
    const pulse = await screen.findByRole("region", { name: "Project pulse" });
    expect(pulse.compareDocumentPosition(screen.getByText("Recent activity")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    await user.click(within(pulse).getByRole("button", { name: "Generate" }));
    expect(await within(pulse).findByText(AI_COPY.ready)).toBeInTheDocument();
    expect(within(pulse).getByText(/^Demo - sample AI output/)).toBeInTheDocument();
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("offers the owner Draft tasks with AI, which adds nothing in the demo", async () => {
    const user = userEvent.setup();
    renderPage(<ProjectDetailPage />, "owner");
    await user.click(await screen.findByRole("button", { name: "Draft tasks with AI" }));
    const dialog = await screen.findByRole("dialog", { name: "Draft tasks with AI" });
    await user.type(within(dialog).getByRole("textbox", { name: "Goal" }), "Ship the beta");
    await user.click(within(dialog).getByRole("button", { name: "Draft tasks" }));
    expect(await within(dialog).findByRole("group", { name: "Suggested tasks" })).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: /^Add \d+ tasks$/ }));
    expect(screen.queryByRole("dialog", { name: "Draft tasks with AI" })).toBeNull();
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("shows a member no pulse", async () => {
    renderPage(<ProjectDetailPage />, "worker");
    await screen.findByText("Recent activity");
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByRole("region", { name: "Project pulse" })).toBeNull();
  });
});
