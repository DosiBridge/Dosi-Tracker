// Acceptance spec — Feature: Role-gated module access.
// Gherkin documentation: src/test/acceptance/features/module-access.feature
//
// Business rule under test: a role only ever sees the modules it may use.
// There are TWO enforcement points — the sidebar (hides forbidden modules) and
// the RoleGuard route boundary (blocks direct navigation). Both must agree:
// a hidden module that is still reachable, or an offered module that is
// blocked, is a security/UX defect.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, vi } from "vitest";
import { screen } from "@testing-library/react";
import { axe } from "jest-axe";
import { Sidebar } from "@/components/layout/sidebar";
import { RoleGuard } from "@/components/role-guard";
import type { Role } from "@/lib/types";
import { renderAsRole, resetPrototypeState } from "@/test/harness";

const noop = () => {};

function TenantSidebar() {
  return <Sidebar collapsed={false} onToggle={noop} mobileOpen={false} onMobileClose={noop} />;
}

/** Every module the tenant sidebar can offer, by user-visible label. */
const modules = [
  { href: "/dashboard", label: "Dashboard" },
  { href: "/projects", label: "Projects" },
  { href: "/team", label: "Team" },
  { href: "/monitor", label: "Member Monitor" },
  { href: "/activities", label: "Activity" },
  { href: "/timesheet", label: "Timesheet" },
  { href: "/reports", label: "Reports" },
  { href: "/billing", label: "Billing & Plan" },
  { href: "/settings", label: "Settings" },
] as const;

beforeEach(() => {
  resetPrototypeState();
});

describe("Feature: Role-gated module access", () => {
  describe("Scenario: Client's restricted workspace view", () => {
    it("Given a signed-in client When the sidebar renders Then only Dashboard and Settings are offered", () => {
      renderAsRole(<TenantSidebar />, "client", { route: "/dashboard" });

      expect(screen.getByRole("link", { name: "Dashboard" })).toBeInTheDocument();
      expect(screen.getByRole("link", { name: "Settings" })).toBeInTheDocument();
      for (const hidden of ["Projects", "Team", "Member Monitor", "Activity", "Timesheet", "Reports", "Billing & Plan"]) {
        expect(
          screen.queryByRole("link", { name: hidden }),
          `client sidebar must not offer "${hidden}"`,
        ).not.toBeInTheDocument();
      }
      expect(screen.getAllByRole("link")).toHaveLength(2);
    });

    it("Given a signed-in client When they navigate directly to /reports Then the restricted notice replaces report content", () => {
      renderAsRole(
        <RoleGuard>
          <div>quarterly report body</div>
        </RoleGuard>,
        "client",
        { route: "/reports" },
      );

      expect(screen.queryByText("quarterly report body")).not.toBeInTheDocument();
      expect(screen.getByRole("heading", { name: /restricted area/i })).toBeInTheDocument();
    });

    it("a11y: Given a client's reduced sidebar When scanned with axe Then it has no violations", async () => {
      const { container } = renderAsRole(<TenantSidebar />, "client", { route: "/dashboard" });
      expect(await axe(container)).toHaveNoViolations();
    });
  });

  describe("Scenario: Worker cannot reach team management", () => {
    it("Given a signed-in worker When the sidebar renders Then Team is missing while their own modules remain", () => {
      renderAsRole(<TenantSidebar />, "worker", { route: "/dashboard" });

      expect(screen.queryByRole("link", { name: "Team" })).not.toBeInTheDocument();
      expect(screen.queryByRole("link", { name: "Reports" })).not.toBeInTheDocument();
      for (const label of ["Dashboard", "Projects", "Member Monitor", "Activity", "Timesheet", "Settings"]) {
        expect(screen.getByRole("link", { name: label })).toBeInTheDocument();
      }
    });

    it("Given a signed-in worker When they navigate directly to /team Then the route guard blocks the page", () => {
      renderAsRole(
        <RoleGuard>
          <div>team management body</div>
        </RoleGuard>,
        "worker",
        { route: "/team" },
      );

      expect(screen.queryByText("team management body")).not.toBeInTheDocument();
      expect(screen.getByRole("heading", { name: /restricted area/i })).toBeInTheDocument();
    });
  });

  describe("Scenario: Sidebar and route guard agree", () => {
    it.each(["owner", "admin", "worker", "client"] as const)(
      "Given a signed-in %s When comparing sidebar visibility with direct navigation Then both enforcement points agree on every module",
      (role: Role) => {
        // What the sidebar OFFERS this role.
        const sidebar = renderAsRole(<TenantSidebar />, role, { route: "/dashboard" });
        const offered = new Set(
          modules.filter((m) => screen.queryByRole("link", { name: m.label }) !== null).map((m) => m.label),
        );
        sidebar.unmount();

        // What direct navigation actually REACHES for this role.
        for (const m of modules) {
          const guarded = renderAsRole(
            <RoleGuard>
              <div>{`module body ${m.href}`}</div>
            </RoleGuard>,
            role,
            { route: m.href },
          );
          const reachable = screen.queryByText(`module body ${m.href}`) !== null;
          guarded.unmount();

          expect(
            reachable,
            `${role} at ${m.href}: sidebar ${offered.has(m.label) ? "offers" : "hides"} "${m.label}" but direct navigation is ${reachable ? "allowed" : "blocked"}`,
          ).toBe(offered.has(m.label));
        }
      },
    );
  });
});
