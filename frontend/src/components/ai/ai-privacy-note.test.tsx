// AI assist on the Privacy tab (AI assist spec §18.5): everyone who has the tab
// reads whether AI assist is on, who can use it and what they can, what of
// their data may be sent (with this workspace's window-title and note
// choices), and what never is. Read-only: no controls; only the owner gets
// the way to the full disclosure and its settings.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { axe } from "jest-axe";
import { AiPrivacyNote } from "./ai-privacy-note";
import type { AiAssistantValue } from "./ai-provider";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { aiContext, aiStatusWith } from "@/test/ai-fixtures";
import { resetPrototypeState } from "@/test/harness";

vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

const ctx = vi.hoisted(() => ({ value: null as unknown as AiAssistantValue }));
vi.mock("@/components/ai/ai-provider", async (importOriginal) => {
  const mod = await importOriginal<typeof import("./ai-provider")>();
  return { ...mod, useAiAssistant: () => ctx.value };
});

beforeEach(() => {
  resetPrototypeState();
});

describe("AiPrivacyNote", () => {
  it("tells a member what is on, what they can use, and what of their data may and may not be sent — read-only", async () => {
    ctx.value = aiContext({ role: "worker", status: { ...demoAiStatus("worker"), providerDisplayName: "Example AI" } });
    const { container } = render(<AiPrivacyNote />);
    expect(screen.getByRole("region", { name: "AI assist" })).toBeInTheDocument();
    expect(screen.getByText("On", { selector: "span" })).toBeInTheDocument();
    expect(screen.getByText(/Summaries are written with Example AI/)).toBeInTheDocument();
    expect(screen.getByText(/You can use:/)).toHaveTextContent("Week summaries, Day summaries, Tracking checks, Note suggestions, Task drafts, Ask AI");
    expect(screen.getByText(/You can use:/)).not.toHaveTextContent("Report explanations");
    expect(screen.getByText("Window titles (not included in this workspace)")).toBeInTheDocument();
    expect(screen.getByText("Task notes (not included in this workspace)")).toBeInTheDocument();
    expect(screen.getByText("Screenshots and webcam images")).toBeInTheDocument();
    expect(screen.getByText("Keystrokes (only counts are recorded, and counts are not sent either)")).toBeInTheDocument();
    expect(screen.queryByRole("link")).toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.queryByRole("switch")).toBeNull();
    expect(await axe(container)).toHaveNoViolations(); // a11y: ai privacy note member has no violations
  });

  it("shows when the owner includes window titles or task notes", () => {
    ctx.value = aiContext({ role: "admin", status: { ...demoAiStatus("admin"), allowWindowTitles: true, allowNotes: true } });
    render(<AiPrivacyNote />);
    expect(screen.getByText("Window titles (included in this workspace)")).toBeInTheDocument();
    expect(screen.getByText("Task notes (included in this workspace)")).toBeInTheDocument();
    expect(screen.queryByRole("link")).toBeNull();
  });

  it("gives the owner the way to the full disclosure and the settings", () => {
    ctx.value = aiContext();
    render(<AiPrivacyNote />);
    expect(screen.getByRole("link", { name: "Read the full AI disclosure and change the AI settings" })).toHaveAttribute("href", "/settings?tab=workspace");
  });

  it.each<[string, Partial<AiAssistantValue>, string]>([
    ["off in the workspace", { role: "worker", status: aiStatusWith("workspace-disabled", { workspaceEnabled: false }) }, "Off"],
    ["unavailable on the server", { role: "worker", status: aiStatusWith("not-configured") }, "Unavailable"],
    ["paused for a changed disclosure", { role: "admin", status: aiStatusWith("reconsent-required") }, "Paused"],
  ])("says plainly when AI assist is %s, without listing features", (_label, patch, label) => {
    ctx.value = aiContext(patch);
    render(<AiPrivacyNote />);
    expect(screen.getByText(label, { selector: "span" })).toBeInTheDocument();
    expect(screen.queryByText(/You can use:/)).toBeNull();
    expect(screen.getByText("Never sent")).toBeInTheDocument();
  });

  it("says when no feature is left for this member", () => {
    const status = { ...demoAiStatus("worker"), features: demoAiStatus("worker").features.map((f) => ({ ...f, allowed: false })) };
    ctx.value = aiContext({ role: "worker", status });
    render(<AiPrivacyNote />);
    expect(screen.getByText("No AI feature is available to you in this workspace.")).toBeInTheDocument();
  });

  it("checks the status before saying anything about it, and calls a demo's output sample text", () => {
    ctx.value = aiContext({ status: null, statusLoading: true });
    const { unmount } = render(<AiPrivacyNote />);
    expect(screen.getByRole("status")).toHaveTextContent("Checking AI assist…");
    unmount();
    ctx.value = aiContext({ live: false });
    render(<AiPrivacyNote />);
    expect(screen.getByText("Sample", { selector: "span" })).toBeInTheDocument();
    expect(screen.getByText(/nothing is sent anywhere/)).toBeInTheDocument();
  });
});
