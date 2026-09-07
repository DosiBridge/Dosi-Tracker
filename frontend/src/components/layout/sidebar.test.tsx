// The sidebar's role-filtered nav IS the product's module-visibility system.
// These tests pin the rendered link set to the navAccess matrix (src/lib/roles.ts)
// for every tenant role, verify active-route highlighting, and axe-scan both the
// expanded and collapsed rails.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, vi } from "vitest";
import { screen, within } from "@testing-library/react";
import { axe } from "jest-axe";
import { Sidebar } from "@/components/layout/sidebar";
import { canAccess } from "@/lib/roles";
import { renderAsRole, resetPrototypeState } from "@/test/harness";

// Mirror of the component's own nav list (href + visible label), in render order.
// If the component adds/renames a module this mirror must be updated — the
// matrix test below will fail loudly if the two drift apart.
const NAV = [
  { href: "/dashboard", label: "Dashboard" },
  { href: "/projects", label: "Projects" },
  { href: "/team", label: "Team" },
  { href: "/monitor", label: "Member Monitor" },
  { href: "/activities", label: "Activity" },
  { href: "/timesheet", label: "Timesheet" },
  { href: "/reports", label: "Reports" },
  { href: "/billing", label: "Billing & Plan" },
  { href: "/settings", label: "Settings" },
];

const baseProps = {
  collapsed: false,
  onToggle: () => {},
  mobileOpen: false,
  onMobileClose: () => {},
};

function renderedNavHrefs(): string[] {
  const nav = screen.getByRole("navigation");
  return within(nav)
    .getAllByRole("link")
    .map((l) => l.getAttribute("href") ?? "");
}

beforeEach(() => {
  resetPrototypeState();
});

describe("Sidebar module visibility (role × navAccess matrix)", () => {
  it.each(["owner", "admin", "worker", "client"] as const)(
    "shows a %s exactly the modules the access matrix allows, in nav order",
    (role) => {
      renderAsRole(<Sidebar {...baseProps} />, role);
      const expected = NAV.filter((i) => canAccess(role, i.href)).map((i) => i.href);
      expect(expected.length).toBeGreaterThan(0); // guard against a vacuous pass
      expect(renderedNavHrefs()).toEqual(expected);
    },
  );

  it.each(["owner", "admin", "worker", "client"] as const)(
    "never renders a link a %s is not allowed to open",
    (role) => {
      renderAsRole(<Sidebar {...baseProps} />, role);
      for (const href of renderedNavHrefs()) {
        expect(canAccess(role, href), `${role} must be allowed to open ${href}`).toBe(true);
      }
    },
  );

  it("gives the owner a Billing link that the admin never sees", () => {
    const owner = renderAsRole(<Sidebar {...baseProps} />, "owner");
    expect(screen.getByRole("link", { name: "Billing & Plan" })).toHaveAttribute("href", "/billing");
    owner.unmount();

    renderAsRole(<Sidebar {...baseProps} />, "admin");
    expect(screen.queryByRole("link", { name: "Billing & Plan" })).not.toBeInTheDocument();
    // …while the admin keeps the analytics/people modules
    expect(screen.getByRole("link", { name: "Reports" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Team" })).toBeInTheDocument();
  });

  it("hides Reports and Team from a worker but keeps their own work modules", () => {
    renderAsRole(<Sidebar {...baseProps} />, "worker");
    expect(screen.queryByRole("link", { name: "Reports" })).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Team" })).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Timesheet" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Projects" })).toBeInTheDocument();
  });

  it("limits a client to Dashboard and Settings only", () => {
    renderAsRole(<Sidebar {...baseProps} />, "client");
    expect(renderedNavHrefs()).toEqual(["/dashboard", "/settings"]);
  });
});

describe("Sidebar active-route highlighting", () => {
  it("highlights the current route and only that route", () => {
    renderAsRole(<Sidebar {...baseProps} />, "owner", { route: "/reports" });
    const nav = screen.getByRole("navigation");
    const reports = within(nav).getByRole("link", { name: "Reports" });
    const dashboard = within(nav).getByRole("link", { name: "Dashboard" });
    expect(reports.className).toContain("bg-accent");
    expect(dashboard.className).not.toContain("bg-accent");
  });

  it("keeps the parent module highlighted on nested routes", () => {
    renderAsRole(<Sidebar {...baseProps} />, "owner", { route: "/projects/p1" });
    const nav = screen.getByRole("navigation");
    expect(within(nav).getByRole("link", { name: "Projects" }).className).toContain("bg-accent");
    expect(within(nav).getByRole("link", { name: "Dashboard" }).className).not.toContain("bg-accent");
  });
});

describe("Sidebar chrome", () => {
  it("identifies the signed-in user with name and role label", () => {
    renderAsRole(<Sidebar {...baseProps} />, "admin");
    expect(screen.getByText("David Chen")).toBeInTheDocument();
    expect(screen.getByText("Administrator")).toBeInTheDocument();
  });

  it("collapsed rail hides labels but keeps accessible link names", () => {
    renderAsRole(<Sidebar {...baseProps} collapsed={true} />, "owner");
    const nav = screen.getByRole("navigation");
    const dash = within(nav).getByRole("link", { name: "Dashboard" });
    expect(dash.textContent).toBe(""); // icon-only — name comes from the title attribute
  });

  it("a11y: expanded sidebar has no axe violations", async () => {
    const { container } = renderAsRole(<Sidebar {...baseProps} />, "owner");
    expect(await axe(container)).toHaveNoViolations();
  });

  it("a11y: collapsed rail has no axe violations (icon-only controls stay named)", async () => {
    const { container } = renderAsRole(<Sidebar {...baseProps} collapsed={true} />, "owner");
    expect(await axe(container)).toHaveNoViolations();
  });
});
