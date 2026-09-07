// WorkspaceSwitcher is the multi-tenant entry point: it must list every seed
// workspace with its plan/status, switch the active tenant (rebrand + persist
// dosi-workspace), and offer the create-workspace flow.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, vi } from "vitest";
import { screen, within, fireEvent } from "@testing-library/react";
import { WorkspaceSwitcher } from "@/components/layout/workspace-switcher";
import { workspaces, planById, statusLabel } from "@/lib/saas-data";
import { renderWithProviders, resetPrototypeState } from "@/test/harness";

beforeEach(() => {
  resetPrototypeState();
});

function openMenu(): HTMLElement {
  fireEvent.click(screen.getByRole("button", { name: /dosi labs/i }));
  // The dropdown is the container that carries the "Workspaces" heading.
  return screen.getByText("Workspaces").parentElement as HTMLElement;
}

describe("WorkspaceSwitcher", () => {
  it("brands the trigger with the active workspace, plan and subscription status", () => {
    renderWithProviders(<WorkspaceSwitcher collapsed={false} />);
    expect(screen.getByText("Dosi Labs")).toBeInTheDocument();
    expect(screen.getByText("Business · Active")).toBeInTheDocument();
  });

  it("lists every seed workspace with its plan label when opened", () => {
    renderWithProviders(<WorkspaceSwitcher collapsed={false} />);
    const menu = openMenu();
    expect(workspaces.length).toBeGreaterThanOrEqual(3); // w1, w2, w3 seed tenants
    for (const w of workspaces) {
      expect(within(menu).getByText(w.name)).toBeInTheDocument();
      expect(
        within(menu).getByText(`${planById(w.planId).name} · ${statusLabel[w.status]}`),
      ).toBeInTheDocument();
    }
  });

  it("switching to Acme Studio rebrands the switcher and persists dosi-workspace=w2", () => {
    renderWithProviders(<WorkspaceSwitcher collapsed={false} />);
    openMenu();
    fireEvent.click(screen.getByRole("button", { name: /acme studio/i }));

    expect(localStorage.getItem("dosi-workspace")).toBe("w2");
    // Menu closed; the trigger now shows the new tenant's branding.
    expect(screen.getByText("Acme Studio")).toBeInTheDocument();
    expect(screen.getByText("Starter · Trial")).toBeInTheDocument();
    expect(screen.queryByText("Dosi Labs")).not.toBeInTheDocument();
  });

  it("create-workspace flow provisions a new tenant and makes it active", () => {
    renderWithProviders(<WorkspaceSwitcher collapsed={false} />, { withToasts: true });
    openMenu();
    fireEvent.click(screen.getByRole("button", { name: /create workspace/i }));

    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("Create a workspace")).toBeInTheDocument();

    const submit = within(dialog).getByRole("button", { name: "Create workspace" });
    expect(submit).toBeDisabled(); // no name yet

    fireEvent.change(within(dialog).getByPlaceholderText("Acme Corp"), {
      target: { value: "Test Rocket" },
    });
    expect(within(dialog).getByText(/test-rocket\.dositracker\.app/)).toBeInTheDocument();
    fireEvent.click(submit);

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    // The new tenant is persisted and active…
    expect(localStorage.getItem("dosi-workspace")).toMatch(/^w-/);
    const created = JSON.parse(localStorage.getItem("dosi-workspaces-created") ?? "[]") as {
      name: string;
    }[];
    expect(created.map((w) => w.name)).toContain("Test Rocket");
    // …the switcher rebrands (default plan = Starter, which starts as a trial)…
    expect(screen.getByText("Test Rocket")).toBeInTheDocument();
    expect(screen.getByText("Starter · Trial")).toBeInTheDocument();
    // …and the user gets confirmation.
    expect(screen.getByText("Workspace created")).toBeInTheDocument();
  });
});
