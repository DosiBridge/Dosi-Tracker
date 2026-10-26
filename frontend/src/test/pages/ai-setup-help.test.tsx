// "Ask the setup assistant" (AI assist spec §3.1, §18.5): the setup checklist
// and the Download page open Ask AI with the setup questions (installing the
// agent on macOS, what AI assist sends) wherever the Ask AI launcher would
// show, and nowhere else.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const session = vi.hoisted(() => ({ current: null as unknown }));
vi.mock("@/components/session-provider", () => ({ useSession: () => session.current }));
const ai = vi.hoisted(() => ({ value: null as unknown }));
vi.mock("@/components/ai/ai-provider", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/components/ai/ai-provider")>()),
  useAiAssistant: () => ai.value,
}));

import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "jest-axe";
import DownloadPage from "@/app/(dashboard)/download/page";
import type { AiAssistantOpenOptions } from "@/components/ai/ai-provider";
import { SetupChecklist } from "@/components/dashboard/setup-checklist";
import type { AiStatusDto } from "@/lib/ai-contract";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { installLiveDataset } from "@/lib/tenant-data";
import type { Role, User } from "@/lib/types";
import { aiContext, aiStatusWith } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";

const me: User = {
  id: "owner-1",
  name: "Olivia Owner",
  email: "olivia@real.example",
  role: "owner",
  designation: "Owner",
  status: "offline",
  timezone: "UTC",
  trackedToday: 0,
  productivity: 0,
  joinedAt: "",
};

let openAssistant: ReturnType<typeof vi.fn<(options?: AiAssistantOpenOptions) => void>>;

function as(role: Role, status: AiStatusDto | null = demoAiStatus(role)) {
  openAssistant = vi.fn<(options?: AiAssistantOpenOptions) => void>();
  session.current = { user: { ...me, role }, isLive: true, status: "ready" };
  ai.value = aiContext({ role, status, openAssistant });
}

beforeEach(() => {
  resetPrototypeState();
  // A brand-new live workspace: every setup step is still to do.
  localStorage.setItem("dosi-token", "live-token");
  installLiveDataset({ workspaceId: "ws-new", users: [me], projects: [], activities: [], now: new Date() });
  as("owner");
});

describe("Ask the setup assistant", () => {
  it("opens Ask AI with the setup questions from the setup checklist", async () => {
    const user = userEvent.setup();
    const { container } = render(<SetupChecklist isOwner />);
    expect(screen.getByText("Get your workspace live")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Ask the setup assistant" }));
    expect(openAssistant).toHaveBeenCalledWith({ chips: "setup" });
    expect(await axe(container)).toHaveNoViolations(); // a11y: setup checklist with the setup assistant has no violations
  });

  it("opens Ask AI with the setup questions from the Download page", async () => {
    const user = userEvent.setup();
    as("worker");
    render(<DownloadPage />);
    await user.click(screen.getByRole("button", { name: "Ask the setup assistant" }));
    expect(openAssistant).toHaveBeenCalledWith({ chips: "setup" });
  });

  it("is offered to the owner of a workspace that hasn't opted in (Ask AI then explains it)", () => {
    as("owner", aiStatusWith("workspace-disabled"));
    render(<SetupChecklist isOwner />);
    expect(screen.getByRole("button", { name: "Ask the setup assistant" })).toBeInTheDocument();
  });

  it.each<[string, Role, AiStatusDto | null]>([
    ["a server without AI", "owner", null],
    ["a client", "client", demoAiStatus("client")],
    ["a manager of a workspace with AI off", "admin", aiStatusWith("workspace-disabled", { canManage: false })],
  ])("is left out for %s", (_label, role, status) => {
    as(role, status);
    render(
      <>
        <SetupChecklist isOwner={role === "owner"} />
        <DownloadPage />
      </>,
    );
    expect(screen.queryByRole("button", { name: "Ask the setup assistant" })).toBeNull();
  });
});
