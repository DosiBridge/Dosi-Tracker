// RoleGuard is the client-side route-access boundary: it must render children
// for permitted roles and a "Restricted area" fallback (never the children)
// for everyone else. These tests exercise the REAL SessionProvider + roles
// matrix — nothing but next/navigation is stubbed.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, vi } from "vitest";
import { screen } from "@testing-library/react";
import { axe } from "jest-axe";
import { RoleGuard } from "@/components/role-guard";
import { renderAsRole, resetPrototypeState } from "@/test/harness";

beforeEach(() => {
  resetPrototypeState();
});

describe("RoleGuard", () => {
  it("renders children when the signed-in role may access the route", () => {
    renderAsRole(
      <RoleGuard>
        <div>reports content</div>
      </RoleGuard>,
      "owner",
      { route: "/reports" },
    );
    expect(screen.getByText("reports content")).toBeInTheDocument();
  });

  it("blocks a worker from /reports and never mounts the children", () => {
    renderAsRole(
      <RoleGuard>
        <div>reports content</div>
      </RoleGuard>,
      "worker",
      { route: "/reports" },
    );
    expect(screen.queryByText("reports content")).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /restricted area/i })).toBeInTheDocument();
  });

  it("names the blocked role in the fallback and offers a way back to the dashboard", () => {
    renderAsRole(
      <RoleGuard>
        <div>billing content</div>
      </RoleGuard>,
      "client",
      { route: "/billing" },
    );
    expect(screen.getByText(/client/i)).toBeInTheDocument();
    const back = screen.getByRole("link", { name: /back to dashboard/i });
    expect(back).toHaveAttribute("href", "/dashboard");
  });

  it("a11y: the restricted fallback has no axe violations", async () => {
    const { container } = renderAsRole(
      <RoleGuard>
        <div>hidden</div>
      </RoleGuard>,
      "client",
      { route: "/billing" },
    );
    expect(await axe(container)).toHaveNoViolations();
  });
});
