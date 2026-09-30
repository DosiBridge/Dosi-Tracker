// Acceptance spec — Feature: Host platform operations.
// Gherkin documentation: src/test/acceptance/features/host-operations.feature
//
// Business rule under test: the /host console belongs to the platform operator
// alone. The host can oversee every tenant, step into one as its owner
// (impersonation, clearly announced and reversible), while tenant roles are
// locked out of the console entirely.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, vi } from "vitest";
import { screen, within, fireEvent } from "@testing-library/react";
import HostLayout from "@/app/(host)/layout";
import HostOverviewPage from "@/app/(host)/host/page";
import HostTenantsPage from "@/app/(host)/host/tenants/page";
import DashboardLayout from "@/app/(dashboard)/layout";
import { renderAsRole, renderWithProviders, resetPrototypeState } from "@/test/harness";
import { __router } from "@/test/next-navigation-stub";

beforeEach(() => {
  resetPrototypeState();
});

describe("Feature: Host platform operations", () => {
  describe("Scenario: Platform admin oversees tenants", () => {
    it("Given the host user When they open the host console Then the platform chrome renders instead of the lock screen", () => {
      renderAsRole(
        <HostLayout>
          <div>host console body</div>
        </HostLayout>,
        "host",
        { route: "/host" },
      );

      expect(screen.getByText("host console body")).toBeInTheDocument();
      expect(screen.queryByRole("heading", { name: /host area/i })).not.toBeInTheDocument();
    });

    it("Given the host user When they view the platform overview Then it renders with a path into tenant management", () => {
      renderAsRole(<HostOverviewPage />, "host", { route: "/host" });

      expect(screen.getByText("Platform Overview")).toBeInTheDocument();
      expect(screen.getByRole("link", { name: /manage tenants/i })).toHaveAttribute("href", "/host/tenants");
    });

    it("Given the host user When they review the tenants page in demo mode Then every seed workspace is listed", () => {
      renderAsRole(<HostTenantsPage />, "host", { route: "/host/tenants" });

      expect(screen.getByText("3 of 3 workspaces on the platform.")).toBeInTheDocument();
      for (const name of ["Dosi Labs", "Acme Studio", "Nimbus Co"]) {
        expect(screen.getAllByText(name).length, `tenant "${name}" should be listed`).toBeGreaterThan(0);
      }
    });
  });

  describe("Scenario: Host impersonates a tenant", () => {
    it("Given the host on the tenants page When they log in as Acme Studio Then the tenant dashboard announces the impersonation And Return to Host restores the platform session", () => {
      // Step 1 — the host clicks "Login as tenant" on the Acme Studio row.
      const tenantsPage = renderAsRole(<HostTenantsPage />, "host", { route: "/host/tenants" });
      const acmeRow = screen
        .getAllByText("Acme Studio")
        .map((el) => el.closest("tr"))
        .find((tr): tr is HTMLTableRowElement => tr !== null);
      if (!acmeRow) throw new Error("Acme Studio row not rendered in the tenants table");
      fireEvent.click(within(acmeRow).getByTitle("Login as tenant"));
      tenantsPage.unmount();

      // Step 2 — the app navigates to the tenant dashboard; mount it fresh so
      // the impersonated session is read back from storage, exactly as a real
      // navigation would.
      renderWithProviders(
        <DashboardLayout>
          <div>tenant dashboard body</div>
        </DashboardLayout>,
        { route: "/dashboard" },
      );

      const banner = screen.getByText(/host impersonation/i);
      expect(banner).toHaveTextContent("Acme Studio"); // viewing the impersonated tenant…
      expect(banner).toHaveTextContent("Diego Alvarez"); // …as that tenant's owner
      expect(screen.getByText("tenant dashboard body")).toBeInTheDocument();

      // Step 3 — returning ends the impersonation and sends the host back.
      fireEvent.click(screen.getByRole("button", { name: /return to host/i }));
      expect(screen.queryByText(/host impersonation/i)).not.toBeInTheDocument();
      expect(__router.push).toHaveBeenCalledWith("/host");
    });
  });

  describe("Scenario: Tenant users cannot reach the host console", () => {
    it.each(["owner", "admin", "worker", "client"] as const)(
      "Given a signed-in %s When they browse to /host Then the console lock screen blocks them",
      (role) => {
        renderAsRole(
          <HostLayout>
            <div>platform secrets</div>
          </HostLayout>,
          role,
          { route: "/host" },
        );

        expect(screen.queryByText("platform secrets")).not.toBeInTheDocument();
        expect(screen.getByRole("heading", { name: /host area/i })).toBeInTheDocument();
        expect(screen.getByRole("link", { name: /go to login/i })).toHaveAttribute("href", "/login");
      },
    );
  });
});
