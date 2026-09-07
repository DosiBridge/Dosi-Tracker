// Acceptance spec — Feature: Tenant lifecycle.
// Gherkin documentation: src/test/acceptance/features/tenant-lifecycle.feature
//
// Business rule under test: workspaces (tenants) can be created, rebranded and
// deleted from the client, the switcher chrome always reflects the change, and
// the session survives a reload (localStorage is the persistence mechanism the
// real app uses — the reload test remounts and reads it back, nothing mocked).
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, vi } from "vitest";
import { screen, within, fireEvent } from "@testing-library/react";
import { WorkspaceSwitcher } from "@/components/layout/workspace-switcher";
import { useSession } from "@/components/session-provider";
import type { Workspace } from "@/lib/saas-data";
import { renderAsRole, renderWithProviders, resetPrototypeState } from "@/test/harness";

/** Thin user-action proxies for session operations that other screens trigger. */
function PatchWorkspaceButton({ id, patch, label }: { id: string; patch: Partial<Workspace>; label: string }) {
  const { updateWorkspace } = useSession();
  return (
    <button type="button" onClick={() => updateWorkspace(id, patch)}>
      {label}
    </button>
  );
}

function DeleteWorkspaceButton({ id, label }: { id: string; label: string }) {
  const { deleteWorkspace } = useSession();
  return (
    <button type="button" onClick={() => deleteWorkspace(id)}>
      {label}
    </button>
  );
}

/** Drive the real create-workspace flow through the switcher UI. */
function createRocketCorpThroughUi() {
  fireEvent.click(screen.getByRole("button", { name: /dosi labs/i })); // open switcher
  fireEvent.click(screen.getByRole("button", { name: /create workspace/i })); // open modal
  fireEvent.change(screen.getByPlaceholderText("Acme Corp"), { target: { value: "Rocket Corp" } });
  fireEvent.click(screen.getByRole("button", { name: "Create workspace" })); // submit (Starter is the default plan)
}

beforeEach(() => {
  resetPrototypeState();
});

describe("Feature: Tenant lifecycle", () => {
  describe("Scenario: Owner creates a workspace", () => {
    it("Given the owner When they create 'Rocket Corp' on Starter Then it becomes the active trialing workspace and joins the switcher list", () => {
      renderAsRole(<WorkspaceSwitcher collapsed={false} />, "owner", { route: "/dashboard", withToasts: true });

      createRocketCorpThroughUi();

      // Feedback confirms the plan it was created on.
      expect(screen.getByText("Workspace created")).toBeInTheDocument();
      expect(screen.getByText("Rocket Corp is ready on the Starter plan.")).toBeInTheDocument();

      // The new tenant is now the ACTIVE workspace, on a Starter trial.
      const trigger = screen.getByRole("button", { name: /rocket corp/i });
      expect(trigger).toHaveTextContent("Starter · Trial");

      // And it appears in the switcher list alongside the seed tenants.
      fireEvent.click(trigger);
      expect(screen.getAllByRole("button", { name: /rocket corp/i })).toHaveLength(2); // trigger + list entry
      expect(screen.getAllByRole("button", { name: /dosi labs/i })).toHaveLength(1);
      expect(screen.getAllByRole("button", { name: /acme studio/i })).toHaveLength(1);
    });

    it("Given a freshly created workspace When the app reloads Then Rocket Corp is still the active workspace on its Starter trial", () => {
      const firstVisit = renderAsRole(<WorkspaceSwitcher collapsed={false} />, "owner", { route: "/dashboard" });
      createRocketCorpThroughUi();
      firstVisit.unmount();

      // "Reload": a brand-new provider tree that reads only what was persisted.
      renderWithProviders(<WorkspaceSwitcher collapsed={false} />);

      const trigger = screen.getByRole("button", { name: /rocket corp/i });
      expect(trigger).toHaveTextContent("Starter · Trial");
    });
  });

  describe("Scenario: Workspace rebrand", () => {
    it("Given Acme Studio is renamed and recoloured When the owner switches to it Then the chrome shows the new identity", () => {
      renderAsRole(
        <>
          <PatchWorkspaceButton id="w2" patch={{ name: "Acme Collective", color: "#16a34a" }} label="rebrand acme" />
          <WorkspaceSwitcher collapsed={false} />
        </>,
        "owner",
        { route: "/dashboard" },
      );

      fireEvent.click(screen.getByRole("button", { name: "rebrand acme" }));

      // The switcher list already shows the rebrand — the old name is gone.
      fireEvent.click(screen.getByRole("button", { name: /dosi labs/i }));
      expect(screen.queryByText("Acme Studio")).not.toBeInTheDocument();

      // Switching makes the rebrand the active chrome.
      fireEvent.click(screen.getByRole("button", { name: /acme collective/i }));
      const trigger = screen.getByRole("button", { name: /acme collective/i });
      expect(trigger).toHaveTextContent("Starter · Trial");
      expect(within(trigger).getByText("A")).toHaveStyle({ background: "#16a34a" });
      expect(screen.queryByText("Acme Studio")).not.toBeInTheDocument();
    });
  });

  describe("Scenario: Deleting the active workspace", () => {
    it("Given Acme Studio is the active workspace When it is deleted Then the session falls back to the primary tenant and Acme leaves the switcher", () => {
      renderWithProviders(
        <>
          <DeleteWorkspaceButton id="w2" label="delete acme" />
          <WorkspaceSwitcher collapsed={false} />
        </>,
        { role: "owner", workspaceId: "w2", route: "/dashboard" },
      );

      // Given: Acme Studio is active.
      expect(screen.getByRole("button", { name: /acme studio/i })).toBeInTheDocument();

      // When: it is deleted.
      fireEvent.click(screen.getByRole("button", { name: "delete acme" }));

      // Then: the chrome falls back to the primary tenant (Dosi Labs, Business plan)…
      const trigger = screen.getByRole("button", { name: /dosi labs/i });
      expect(trigger).toHaveTextContent("Business · Active");

      // …and Acme Studio no longer exists anywhere in the switcher.
      fireEvent.click(trigger);
      expect(screen.queryByText("Acme Studio")).not.toBeInTheDocument();
      expect(screen.getAllByRole("button", { name: /nimbus co/i })).toHaveLength(1); // other tenants survive
    });
  });
});
