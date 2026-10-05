// Acceptance spec — Feature: Plan entitlements.
// Gherkin documentation: src/test/acceptance/features/plan-entitlements.feature
//
// Business rule under test: a workspace's plan caps what the tenant may use
// (seats, projects, storage) and prices the subscription. The billing page is
// where an owner sees those caps — it must tell the truth for the ACTIVE
// tenant, react to plan changes, and never render broken pricing ($NaN).
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, within, fireEvent } from "@testing-library/react";
import BillingPage from "@/app/(dashboard)/billing/page";
import { useSession } from "@/components/session-provider";
import type { PlanId, SubscriptionStatus } from "@/lib/saas-data";
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

/** The same kind of proxy for the host's subscription actions (suspend, a lapsed payment…). */
function StatusChangeButton({ id, status }: { id: string; status: SubscriptionStatus }) {
  const { updateWorkspace } = useSession();
  return (
    <button type="button" onClick={() => updateWorkspace(id, { status })}>
      set {status}
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
  describe("Scenario: A workspace that is cut off says so", () => {
    it.each([
      ["suspended", "This workspace is suspended"],
      ["past_due", "Payment is overdue"],
      ["expired", "This workspace's subscription has expired"],
      ["canceled", "This workspace's subscription is canceled"],
    ] as const)("Given the workspace turns %s When the owner looks at billing Then a banner says what it means for the team", (status, title) => {
      renderWithProviders(
        <>
          <StatusChangeButton id="w1" status={status} />
          <BillingPage />
        </>,
        { role: "owner", workspaceId: "w1", route: "/billing" },
      );
      expect(screen.queryByText(title)).not.toBeInTheDocument();

      fireEvent.click(screen.getByRole("button", { name: `set ${status}` }));
      const banner = screen.getByText(title).closest("[role='status']");
      expect(banner).not.toBeNull();
      expect(within(banner as HTMLElement).getByRole("link", { name: /contact support/i })).toHaveAttribute("href", expect.stringMatching(/^mailto:/));
    });
  });

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

describe("Feature: Plan entitlements — live invoices", () => {
  const API = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");
  const json = (body: unknown, status = 200) => {
    const text = JSON.stringify(body);
    return { ok: status < 300, status, statusText: "OK", headers: new Headers(), json: async () => JSON.parse(text), text: async () => text } as Response;
  };

  beforeEach(() => {
    localStorage.setItem("dosi-token", "jwt-live");
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>().mockImplementation(async (input) => {
        const url = String(input).replace(API, "");
        if (url.startsWith("/api/abp/application-configuration")) {
          return json({
            currentUser: { isAuthenticated: true, id: "u-1", tenantId: "t-1", userName: "mira", roles: ["admin"] },
            currentTenant: { id: "t-1", name: "RealCo", isAvailable: true },
            auth: { grantedPolicies: {} },
          });
        }
        if (url.startsWith("/api/app/billing/invoices")) {
          return json({ items: [{ id: "inv-1", amount: 18, status: "paid", dueDate: "2026-10-01T00:00:00Z", creationTime: "2026-09-18T00:00:00Z", billingPeriodStart: "2026-09-01T00:00:00Z" }] });
        }
        if (url.startsWith("/api/app/workspace/current-subscription")) return json({ planId: "plan-s", status: "active" });
        if (url.startsWith("/api/app/workspace/available-plans")) return json({ items: [{ id: "plan-s", name: "Starter", pricePerUser: 6, maxSeats: 25 }] });
        if (url.startsWith("/api/app/workspace/")) return json({}, 404);
        if (url.startsWith("/api/app/")) return json({ items: [], totalCount: 0 });
        throw new Error(`unexpected fetch: ${url}`);
      }),
    );
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("Scenario: Invoices name the month they bill — Given a live invoice for September Then it shows its billing period", async () => {
    renderWithProviders(<BillingPage />, { route: "/billing" });

    expect((await screen.findAllByText("Sep 2026")).length).toBeGreaterThan(0);
  });
});
