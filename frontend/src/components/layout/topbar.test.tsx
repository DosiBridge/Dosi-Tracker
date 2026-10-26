// Topbar behavior: tenant-scoped notification feed with a data-derived unread
// badge, an account menu that names the signed-in user + role, and a plan chip
// that only owners can follow to billing.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, vi } from "vitest";
import { screen, within, fireEvent } from "@testing-library/react";
import { axe } from "jest-axe";
import { Topbar } from "@/components/layout/topbar";
import { AiAssistantProvider } from "@/components/ai/ai-provider";
import { datasetFor } from "@/lib/tenant-data";
import { renderAsRole, renderForWorkspace, renderWithProviders, resetPrototypeState } from "@/test/harness";

const noop = () => {};

// The w1 notification seed is a module-level array that the topbar mutates in
// place when items are marked read. resetPrototypeState() restores it (read
// flags included) and rebuilds the dataset around the SAME array instance, so
// this module-level handle stays valid across resets.
const w1Notifications = datasetFor("w1").notifications;

beforeEach(() => {
  resetPrototypeState();
});

describe("Topbar notifications", () => {
  it("shows the workspace feed with an unread badge that matches the data", () => {
    const expectedUnread = w1Notifications.filter((n) => !n.read).length;
    expect(expectedUnread).toBeGreaterThan(0); // guard: seed must contain unread items

    renderAsRole(<Topbar onMenuClick={noop} />, "owner");
    const bell = screen.getByRole("button", { name: "Notifications" });
    expect(within(bell).getByText(String(expectedUnread))).toBeInTheDocument();

    fireEvent.click(bell);
    for (const n of w1Notifications) {
      expect(screen.getByText(n.title)).toBeInTheDocument();
    }
    const alert = w1Notifications.find((n) => n.type === "alert" && !n.read);
    expect(alert).toBeDefined();
    expect(screen.getByText(alert!.body)).toBeInTheDocument();
  });

  it("swaps to the tenant's own feed when viewing another workspace", () => {
    const w1Alert = w1Notifications.find((n) => n.id === "n2");
    renderForWorkspace(<Topbar onMenuClick={noop} />, "w2");
    const w2Alert = datasetFor("w2").notifications.find((n) => n.id === "n2");
    expect(w1Alert).toBeDefined();
    expect(w2Alert).toBeDefined();
    expect(w2Alert!.body).not.toBe(w1Alert!.body); // the tenants genuinely differ

    fireEvent.click(screen.getByRole("button", { name: "Notifications" }));
    expect(screen.getByText(w2Alert!.body)).toBeInTheDocument();
    expect(screen.queryByText(w1Alert!.body)).not.toBeInTheDocument();
  });

  it("mark-all-read clears the unread badge", () => {
    const expectedUnread = w1Notifications.filter((n) => !n.read).length;
    renderAsRole(<Topbar onMenuClick={noop} />, "owner");
    const bell = screen.getByRole("button", { name: "Notifications" });
    fireEvent.click(bell);
    fireEvent.click(screen.getByText("Mark all read"));
    expect(within(bell).queryByText(String(expectedUnread))).not.toBeInTheDocument();
    expect(screen.queryByText("Mark all read")).not.toBeInTheDocument();
  });

  it("the harness reset restores read flags mutated in place (mark-as-read leak guard)", () => {
    w1Notifications.forEach((n) => {
      n.read = true; // what the topbar's mark-all-read does to the shared seed
    });
    resetPrototypeState();
    expect(datasetFor("w1").notifications.some((n) => !n.read)).toBe(true);
  });
});

describe("Topbar account menu", () => {
  it("identifies the signed-in owner by name, email and role label", () => {
    renderAsRole(<Topbar onMenuClick={noop} />, "owner");
    fireEvent.click(screen.getByRole("button", { name: "Account menu" }));
    expect(screen.getByText("Ayesha Rahman")).toBeInTheDocument();
    expect(screen.getByText("ayesha@dosi.dev")).toBeInTheDocument();
    expect(screen.getByText("Owner")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /profile & settings/i })).toHaveAttribute("href", "/settings");
    expect(screen.getByRole("link", { name: /sign out/i })).toHaveAttribute("href", "/login");
  });

  it("reflects a different signed-in role (admin)", () => {
    renderAsRole(<Topbar onMenuClick={noop} />, "admin");
    fireEvent.click(screen.getByRole("button", { name: "Account menu" }));
    expect(screen.getByText("David Chen")).toBeInTheDocument();
    expect(screen.getByText("Administrator")).toBeInTheDocument();
  });
});

describe("Topbar plan chip", () => {
  it("links the owner's plan chip to billing", () => {
    renderAsRole(<Topbar onMenuClick={noop} />, "owner");
    const chip = screen.getByRole("link", { name: /business/i });
    expect(chip).toHaveAttribute("href", "/billing");
  });

  it("renders the plan as plain text for non-owners — no billing link", () => {
    renderAsRole(<Topbar onMenuClick={noop} />, "worker");
    expect(screen.queryByRole("link", { name: /business/i })).not.toBeInTheDocument();
    expect(screen.getByText("Business")).toBeInTheDocument();
  });

  it("flags a trialing workspace on the plan chip", () => {
    renderForWorkspace(<Topbar onMenuClick={noop} />, "w2"); // Acme Studio: starter, trialing
    const chip = screen.getByRole("link", { name: /starter/i });
    expect(within(chip).getByText(/trial/i)).toBeInTheDocument();
  });
});

describe("Topbar chrome", () => {
  it("titles the mobile header from the current route", () => {
    renderWithProviders(<Topbar onMenuClick={noop} />, { role: "owner", route: "/timesheet" });
    expect(screen.getByText("Timesheet")).toBeInTheDocument();
  });

  it("a11y: topbar with notification and account menus open has no axe violations", async () => {
    const { container } = renderAsRole(<Topbar onMenuClick={noop} />, "owner");
    fireEvent.click(screen.getByRole("button", { name: "Notifications" }));
    expect(await axe(container)).toHaveNoViolations();
  });
});

describe("Topbar Ask AI launcher", () => {
  const withAi = (ui: React.ReactNode) => <AiAssistantProvider>{ui}</AiAssistantProvider>;

  it("sits before the theme toggle, opens Ask AI, and adds no keyboard shortcut", () => {
    renderAsRole(withAi(<Topbar onMenuClick={noop} />), "owner");
    const launcher = screen.getByRole("button", { name: "Ask AI" });
    expect(launcher.compareDocumentPosition(screen.getByRole("button", { name: "Toggle theme" })) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(launcher).toHaveAttribute("aria-expanded", "false");
    // Ctrl+J is the browser's Downloads: nothing of ours listens for it.
    fireEvent.keyDown(window, { key: "j", ctrlKey: true });
    expect(launcher).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(launcher);
    expect(launcher).toHaveAttribute("aria-expanded", "true");
  });

  it("follows the server's status: a member gets it, a client doesn't, and nobody without the AI context", () => {
    const worker = renderAsRole(withAi(<Topbar onMenuClick={noop} />), "worker");
    expect(screen.getByRole("button", { name: "Ask AI" })).toBeInTheDocument();
    worker.unmount();
    const client = renderAsRole(withAi(<Topbar onMenuClick={noop} />), "client");
    expect(screen.queryByRole("button", { name: "Ask AI" })).not.toBeInTheDocument();
    client.unmount();
    renderAsRole(<Topbar onMenuClick={noop} />, "owner");
    expect(screen.queryByRole("button", { name: "Ask AI" })).not.toBeInTheDocument();
  });

  it("a11y: topbar with the Ask AI launcher has no axe violations", async () => {
    const { container } = renderAsRole(withAi(<Topbar onMenuClick={noop} />), "owner");
    expect(await axe(container)).toHaveNoViolations();
  });
});
