// Acceptance spec — Feature: Plan entitlements.
// Gherkin documentation: src/test/acceptance/features/plan-entitlements.feature
//
// Business rule under test: a workspace's plan caps what the tenant may use
// (seats, projects, storage) and prices the subscription. The billing page is
// where an owner sees those caps — it must tell the truth for the ACTIVE
// tenant, react to plan changes, and never render broken pricing ($NaN).
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, vi } from "vitest";
import { screen, within, fireEvent } from "@testing-library/react";
import BillingPage from "@/app/(dashboard)/billing/page";
import { useSession } from "@/components/session-provider";
import type { PlanId } from "@/lib/saas-data";
import { renderWithProviders, resetPrototypeState } from "@/test/harness";

/**
 * Thin user-action proxy: fires the exact `updateWorkspace` call the host
 * console's plan <Select> is wired to. The unit under test stays BillingPage.
 */
function PlanChangeButton({ id, planId, label }: { id: string; planId: PlanId; label: string }) {
  const { updateWorkspace } = useSession();
  return (
    <button type="button" onClick={() => updateWorkspace(id, { planId })}>
      {label}
    </button>
  );
}

/** Locate a usage meter (block + progress fill) by its user-visible limit text, e.g. "of 3 members". */
function usageMeter(limitLabel: string): { block: HTMLElement; fill: HTMLElement } {
  const block = screen.getByText(limitLabel).parentElement?.parentElement ?? null;
  const fill = block?.querySelector<HTMLElement>("[style*='width']") ?? null;
  if (!block || !fill) throw new Error(`No usage meter found next to "${limitLabel}"`);
  return { block, fill };
}

beforeEach(() => {
  resetPrototypeState();
});

describe("Feature: Plan entitlements", () => {
  describe("Scenario: Free plan seat ceiling", () => {
    it("Given Nimbus Co on the Free plan When the owner views usage Then seats read 3 of 3 members with a full meter", () => {
      renderWithProviders(<BillingPage />, { role: "owner", workspaceId: "w3", route: "/billing" });

      expect(screen.getByText("Usage this cycle")).toBeInTheDocument();
      expect(screen.getByText("Limits from your Free plan")).toBeInTheDocument();

      const { block, fill } = usageMeter("of 3 members");
      expect(within(block).getByText("Seats")).toBeInTheDocument();
      expect(within(block).getByText("3")).toBeInTheDocument();
      expect(fill).toHaveStyle({ width: "100%" });
    });

    it("Given the seat meter sits at its ceiling When the owner looks at it Then the bar is drawn in the warning colour", () => {
      renderWithProviders(<BillingPage />, { role: "owner", workspaceId: "w3", route: "/billing" });

      const { fill } = usageMeter("of 3 members");
      expect(fill).toHaveStyle({ background: "#ef4444" });
    });

    it("Given the Free plan is $0 per user When the owner checks the cost Then 3 seats are priced at $0 without broken math", () => {
      renderWithProviders(<BillingPage />, { role: "owner", workspaceId: "w3", route: "/billing" });

      expect(screen.getByText("$0/user · 3 seats")).toBeInTheDocument();
      expect(document.body.textContent).not.toContain("NaN");
    });
  });

  describe("Scenario: Plan upgrade lifts limits", () => {
    it("Given Nimbus Co at its Free seat ceiling When the workspace is upgraded to Business Then the seat limit reads 50 and headroom opens up", () => {
      renderWithProviders(
        <>
          <PlanChangeButton id="w3" planId="business" label="upgrade nimbus to business" />
          <BillingPage />
        </>,
        { role: "owner", workspaceId: "w3", route: "/billing" },
      );

      // Given: the ceiling is real before the upgrade.
      expect(usageMeter("of 3 members").fill).toHaveStyle({ width: "100%" });

      // When: the plan is lifted to Business.
      fireEvent.click(screen.getByRole("button", { name: "upgrade nimbus to business" }));

      // Then: same 3 seats, but against the Business limit of 50 (3/50 = 6%).
      expect(screen.getByText("Limits from your Business plan")).toBeInTheDocument();
      const { block, fill } = usageMeter("of 50 members");
      expect(within(block).getByText("3")).toBeInTheDocument();
      expect(fill).toHaveStyle({ width: "6%" });
      expect(screen.queryByText("of 3 members")).not.toBeInTheDocument();
    });
  });

  describe("Scenario: Enterprise custom pricing", () => {
    it("Given Nimbus Co When the workspace moves to Enterprise custom pricing Then cost shows Custom / Contact sales and never $NaN", () => {
      renderWithProviders(
        <>
          <PlanChangeButton id="w3" planId="enterprise" label="switch nimbus to enterprise" />
          <BillingPage />
        </>,
        { role: "owner", workspaceId: "w3", route: "/billing" },
      );

      fireEvent.click(screen.getByRole("button", { name: "switch nimbus to enterprise" }));

      // "Custom" replaces the monthly figure (current-plan card + the Enterprise plan card).
      expect(screen.getAllByText("Custom").length).toBeGreaterThanOrEqual(2);
      expect(screen.getByText("Contact sales")).toBeInTheDocument();
      // No "/mo" suffix dangles next to a price that does not exist.
      expect(screen.queryByText("/mo")).not.toBeInTheDocument();
      // Limits are lifted entirely.
      expect(screen.getByText("of Unlimited members")).toBeInTheDocument();
      // null * seats must never leak through as NaN anywhere on the page.
      expect(document.body.textContent).not.toContain("NaN");
    });
  });
});
