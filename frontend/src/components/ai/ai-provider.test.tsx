// The AI assist context: safe no-op defaults without a provider (AI hidden), and
// with one, the session's status, a result cache shared by the workspace's
// cards, the drawer's state and the page context Ask AI will send.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, renderHook, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AiAssistantProvider, useAiAssistant, useAiPageContext, useAiPageContextValue } from "./ai-provider";
import type { AiPageContextDto } from "@/lib/ai-contract";
import { demoAiStatus } from "@/lib/ai-demo-status";
import { SessionProvider, useSession } from "@/components/session-provider";
import { ThemeProvider } from "@/components/theme-provider";
import type { Role } from "@/lib/types";
import { resetPrototypeState, seededUserId } from "@/test/harness";

vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

beforeEach(() => {
  resetPrototypeState();
});

function Providers({ children }: { children: React.ReactNode }) {
  return (
    <ThemeProvider>
      <SessionProvider>{children}</SessionProvider>
    </ThemeProvider>
  );
}

/** Render inside the real session (a demo one), signed in as `role`; rerender keeps the providers. */
function renderAsRole(ui: React.ReactElement, role: Role) {
  localStorage.setItem("dosi-user", seededUserId(role));
  return render(ui, { wrapper: Providers });
}

describe("without a provider", () => {
  it("answers with AI hidden and inert controls", () => {
    const { result } = renderHook(() => {
      useAiPageContext({ page: "dashboard" });
      return { ai: useAiAssistant(), page: useAiPageContextValue() };
    });
    expect(result.current.ai).toMatchObject({ status: null, live: false, drawerOpen: false, pageContext: null });
    expect(result.current.ai.cache.get("x")).toBeUndefined();
    act(() => {
      result.current.ai.cache.set("x", 1);
      result.current.ai.setDrawerOpen(true);
      result.current.ai.refreshStatus();
    });
    expect(result.current.ai.cache.get("x")).toBeUndefined();
    expect(result.current.page).toBeNull();
  });
});

function Probe() {
  const ai = useAiAssistant();
  const page = useAiPageContextValue();
  return (
    <div>
      <span data-testid="role">{ai.role}</span>
      <span data-testid="workspace">{ai.workspaceId}</span>
      <span data-testid="live">{String(ai.live)}</span>
      <span data-testid="status">{ai.status ? JSON.stringify(ai.status.features.map((f) => f.allowed)) : "none"}</span>
      <span data-testid="page">{page ? JSON.stringify(page) : "none"}</span>
      <span data-testid="drawer">{String(ai.drawerOpen)}</span>
      <button onClick={() => ai.setDrawerOpen(!ai.drawerOpen)}>Toggle</button>
      <button onClick={() => ai.cache.set("k", "kept")}>Keep</button>
      <span data-testid="cached">{String(ai.cache.get("k") ?? "")}</span>
    </div>
  );
}

function Registers({ context }: { context: AiPageContextDto | null }) {
  useAiPageContext(context);
  return null;
}

describe("AiAssistantProvider (demo session)", () => {
  it("carries the session's role, workspace and the demo's status", async () => {
    renderAsRole(
      <AiAssistantProvider>
        <Probe />
      </AiAssistantProvider>,
      "worker",
    );
    await waitFor(() => expect(screen.getByTestId("role")).toHaveTextContent("worker"));
    expect(screen.getByTestId("workspace")).toHaveTextContent("w1");
    expect(screen.getByTestId("live")).toHaveTextContent("false");
    expect(screen.getByTestId("status")).toHaveTextContent(JSON.stringify(demoAiStatus("worker").features.map((f) => f.allowed)));
  });

  it("keeps results and the drawer state for the workspace", async () => {
    const user = userEvent.setup();
    renderAsRole(
      <AiAssistantProvider>
        <Probe />
      </AiAssistantProvider>,
      "owner",
    );
    await user.click(screen.getByRole("button", { name: "Keep" }));
    await user.click(screen.getByRole("button", { name: "Toggle" }));
    expect(screen.getByTestId("drawer")).toHaveTextContent("true");
    expect(screen.getByTestId("cached")).toHaveTextContent("kept");
  });

  it("holds the page context while its component is mounted; a newer registration wins", async () => {
    const dashboard: AiPageContextDto = { page: "dashboard", from: "2026-07-07T00:00:00Z", to: undefined };
    const monitor: AiPageContextDto = { page: "monitor", memberId: "u2", day: "2026-07-14" };
    const tree = (contexts: (AiPageContextDto | null)[]) => (
      <AiAssistantProvider>
        <Probe />
        {contexts.map((c, i) => (
          <Registers key={i} context={c} />
        ))}
      </AiAssistantProvider>
    );
    const { rerender } = renderAsRole(tree([dashboard]), "owner");
    await waitFor(() => expect(screen.getByTestId("page")).toHaveTextContent('{"from":"2026-07-07T00:00:00Z","page":"dashboard"}'));

    // The same content in a new object doesn't re-register; new content does.
    rerender(tree([{ ...dashboard }]));
    rerender(tree([dashboard, monitor]));
    await waitFor(() => expect(screen.getByTestId("page")).toHaveTextContent('"page":"monitor"'));
    // The older one unmounting leaves the newer in place.
    rerender(tree([null, monitor]));
    expect(screen.getByTestId("page")).toHaveTextContent('"page":"monitor"');
    rerender(tree([]));
    await waitFor(() => expect(screen.getByTestId("page")).toHaveTextContent("none"));
  });
});

describe("a workspace switch", () => {
  function Switcher() {
    const { setWorkspaceById } = useSession();
    return <button onClick={() => setWorkspaceById("w2")}>Switch</button>;
  }

  it("drops the previous workspace's results and closes the drawer, keeping the same provider", async () => {
    const user = userEvent.setup();
    renderAsRole(
      <AiAssistantProvider>
        <Probe />
        <Switcher />
      </AiAssistantProvider>,
      "owner",
    );
    await user.click(screen.getByRole("button", { name: "Keep" }));
    await user.click(screen.getByRole("button", { name: "Toggle" }));
    expect(screen.getByTestId("cached")).toHaveTextContent("kept");
    expect(screen.getByTestId("drawer")).toHaveTextContent("true");
    // The probe stays mounted: only the provider's state is reset.
    const probe = screen.getByTestId("cached");
    await user.click(screen.getByRole("button", { name: "Switch" }));
    await waitFor(() => expect(screen.getByTestId("workspace")).toHaveTextContent("w2"));
    expect(screen.getByTestId("cached")).toBe(probe);
    expect(probe).toHaveTextContent("");
    expect(screen.getByTestId("drawer")).toHaveTextContent("false");
  });
});
