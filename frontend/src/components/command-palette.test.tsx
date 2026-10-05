// Command palette: opens on Ctrl/⌘-K (and the topbar's custom event), lists
// only the navigation targets the signed-in role may open, and navigates via
// router.push on selection — that push IS the user-visible behavior.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, vi } from "vitest";
import { screen, fireEvent, within } from "@testing-library/react";
import { axe } from "jest-axe";
import { CommandPalette } from "@/components/command-palette";
import { canAccess } from "@/lib/roles";
import { reportCatalog } from "@/lib/reports-data";
import { renderAsRole, resetPrototypeState } from "@/test/harness";
import { __router } from "@/test/next-navigation-stub";

// Mirror of the palette's own page list (label + guarding href).
const PALETTE_PAGES = [
  { label: "Dashboard", href: "/dashboard" },
  { label: "Projects", href: "/projects" },
  { label: "Team", href: "/team" },
  { label: "Member Monitor", href: "/monitor" },
  { label: "Activity", href: "/activities" },
  { label: "Activity · screens", href: "/activities" },
  { label: "Timesheet", href: "/timesheet" },
  { label: "Reports", href: "/reports" },
  { label: "Billing & Plan", href: "/billing" },
  { label: "Desktop agent", href: "/download" },
  { label: "Settings", href: "/settings" },
];

function pressShortcut() {
  fireEvent.keyDown(window, { key: "k", ctrlKey: true });
}

function queryInput() {
  return screen.queryByPlaceholderText(/search pages/i);
}

beforeEach(() => {
  resetPrototypeState();
});

describe("CommandPalette open/close", () => {
  it("opens with Ctrl/⌘-K, toggles shut on a second press, and closes on Escape", () => {
    renderAsRole(<CommandPalette />, "owner");
    expect(queryInput()).not.toBeInTheDocument();

    pressShortcut();
    expect(queryInput()).toBeInTheDocument();

    pressShortcut();
    expect(queryInput()).not.toBeInTheDocument();

    pressShortcut();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(queryInput()).not.toBeInTheDocument();
  });

  it("opens when the topbar's open-command-palette event fires", () => {
    renderAsRole(<CommandPalette />, "owner");
    fireEvent(window, new Event("open-command-palette"));
    expect(queryInput()).toBeInTheDocument();
  });
});

describe("CommandPalette role filtering", () => {
  it.each(["client", "worker"] as const)(
    "a %s only sees palette pages their route permissions allow",
    (role) => {
      renderAsRole(<CommandPalette />, role);
      pressShortcut();
      for (const p of PALETTE_PAGES) {
        const present = screen.queryAllByText(p.label).length > 0;
        expect(present, `"${p.label}" (${p.href}) visibility for ${role}`).toBe(
          canAccess(role, p.href),
        );
      }
      // Each report shortcut follows its own route's access; member jump-tos are gated behind /team.
      for (const r of reportCatalog) {
        expect(screen.queryAllByText(r.title).length > 0, `report "${r.title}" for ${role}`).toBe(canAccess(role, `/reports/${r.slug}`));
      }
      expect(screen.queryByText("Time & Activity")).not.toBeInTheDocument();
      expect(screen.queryByText("Members")).not.toBeInTheDocument();
    },
  );

  it("gives an owner the full catalog: report shortcuts, billing and member jump-tos", () => {
    renderAsRole(<CommandPalette />, "owner");
    pressShortcut();
    expect(screen.getByText("Time & Activity")).toBeInTheDocument(); // report shortcut
    expect(screen.getByText("Billing & Plan")).toBeInTheDocument();
    expect(screen.getByText("Tanvir Hasan")).toBeInTheDocument(); // member jump-to
  });
});

describe("CommandPalette navigation", () => {
  it("navigates to the clicked page and closes", () => {
    renderAsRole(<CommandPalette />, "owner");
    pressShortcut();
    fireEvent.click(screen.getByRole("option", { name: "Team" }));
    expect(__router.push).toHaveBeenCalledWith("/team");
    expect(queryInput()).not.toBeInTheDocument();
  });

  it("offers the desktop agent to tracked roles and opens /download", () => {
    renderAsRole(<CommandPalette />, "worker");
    pressShortcut();
    fireEvent.click(screen.getByRole("option", { name: /desktop agent/i }));
    expect(__router.push).toHaveBeenCalledWith("/download");
  });

  it("keyboard flow: arrow to an item and press Enter to open it", () => {
    renderAsRole(<CommandPalette />, "client");
    pressShortcut();
    const input = screen.getByRole("combobox");
    const options = screen.getAllByRole("option");
    expect(options.length).toBeGreaterThan(1);
    fireEvent.keyDown(input, { key: "ArrowDown" });
    // The highlighted result is announced through aria-activedescendant.
    expect(input).toHaveAttribute("aria-activedescendant", options[1].id);
    expect(options[1]).toHaveAttribute("aria-selected", "true");
    const second = PALETTE_PAGES.find((p) => p.label === options[1].textContent?.trim());
    fireEvent.keyDown(input, { key: "Enter" });
    expect(__router.push).toHaveBeenCalledWith(second?.href);
  });

  it("End and Home jump to the last and first result", () => {
    renderAsRole(<CommandPalette />, "client");
    pressShortcut();
    const input = screen.getByRole("combobox");
    const options = screen.getAllByRole("option");
    fireEvent.keyDown(input, { key: "End" });
    expect(input).toHaveAttribute("aria-activedescendant", options[options.length - 1].id);
    fireEvent.keyDown(input, { key: "Home" });
    expect(input).toHaveAttribute("aria-activedescendant", options[0].id);
  });

  it("search narrows to matching commands and Enter opens the top hit", () => {
    renderAsRole(<CommandPalette />, "owner");
    pressShortcut();
    const input = screen.getByPlaceholderText(/search pages/i);
    fireEvent.change(input, { target: { value: "payroll" } });
    expect(screen.getByText("Payroll & Billing")).toBeInTheDocument();
    expect(screen.queryByText("Dashboard")).not.toBeInTheDocument();
    fireEvent.keyDown(input, { key: "Enter" });
    expect(__router.push).toHaveBeenCalledWith("/reports/payroll");
  });

  it("shows an empty state for a dead-end query and Enter does not navigate", () => {
    renderAsRole(<CommandPalette />, "owner");
    pressShortcut();
    const input = screen.getByPlaceholderText(/search pages/i);
    fireEvent.change(input, { target: { value: "zzz-not-here" } });
    expect(screen.getByText(/no results for/i)).toBeInTheDocument();
    fireEvent.keyDown(input, { key: "Enter" });
    expect(__router.push).not.toHaveBeenCalled();
  });
});

describe("CommandPalette semantics", () => {
  it("is a labelled modal dialog whose search field controls a listbox of options", () => {
    renderAsRole(<CommandPalette />, "owner");
    pressShortcut();
    const dialog = screen.getByRole("dialog", { name: /search and jump/i });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    const input = within(dialog).getByRole("combobox");
    const listbox = within(dialog).getByRole("listbox");
    expect(input).toHaveAttribute("aria-controls", listbox.id);
    expect(input).toHaveFocus();
    expect(within(listbox).getAllByRole("option").length).toBeGreaterThan(5);
  });

  it("a11y: the open palette has no axe violations", async () => {
    const { container } = renderAsRole(<CommandPalette />, "owner");
    pressShortcut();
    expect(await axe(container)).toHaveNoViolations();
  });
});
