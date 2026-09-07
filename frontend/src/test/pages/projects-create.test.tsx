// ProjectsPage behavior in demo mode (no backend token): the create-project
// wizard, title enforcement, cancel, active/archived segregation, and
// persistence of demo-created projects, all through the real page + modal +
// session wiring. Only global fetch is stubbed (the page's useApi always fires
// one request on mount); a never-settling promise keeps the test deterministic
// and forces the demo-data path.
//
// Persistence contract: demo-mode creation goes through createTenantProject()
// in src/lib/tenant-data.ts, which writes localStorage
// "dosi-projects-created-w1" — the key datasetFor() reads back on the next
// (re)build — so a created project survives a full reload.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import ProjectsPage from "@/app/(dashboard)/projects/page";
import { renderAsRole, resetPrototypeState, simulateReload } from "@/test/harness";

let fetchMock: ReturnType<typeof vi.fn<typeof fetch>>;

beforeEach(() => {
  resetPrototypeState();
  fetchMock = vi.fn<typeof fetch>().mockImplementation(() => new Promise<Response>(() => {}));
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const newProjectButton = () => screen.getByRole("button", { name: /new project/i });

async function openWizard(user: ReturnType<typeof userEvent.setup>) {
  await user.click(newProjectButton());
  const dialog = screen.getByRole("dialog");
  expect(within(dialog).getByRole("heading", { name: "Create project" })).toBeInTheDocument();
  return dialog;
}

describe("projects page — create project (owner, demo mode)", () => {
  it("the New project button opens the create wizard on its Details step", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });

    const dialog = await openWizard(user);
    expect(within(dialog).getByText("Project name")).toBeInTheDocument();
    expect(within(dialog).getByPlaceholderText("e.g. Mobile App Revamp")).toBeInTheDocument();
  });

  it("cannot advance past Details until a title is entered", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });

    const dialog = await openWizard(user);
    const next = within(dialog).getByRole("button", { name: /continue/i });
    expect(next).toBeDisabled();

    await user.click(next); // click on a disabled button is a no-op
    expect(within(dialog).getByText("Project name")).toBeInTheDocument(); // still on Details

    await user.type(within(dialog).getByPlaceholderText("e.g. Mobile App Revamp"), "Orbital Launch");
    expect(next).toBeEnabled();
  });

  it("completing the wizard closes it and shows the new project card in the active list", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });
    expect(screen.getByText(/5 active/)).toBeInTheDocument(); // w1 seeds 5 active + 1 archived

    const dialog = await openWizard(user);
    await user.type(within(dialog).getByPlaceholderText("e.g. Mobile App Revamp"), "Orbital Launch");
    await user.click(within(dialog).getByRole("button", { name: /continue/i })); // -> Team
    await user.click(within(dialog).getByRole("button", { name: /continue/i })); // -> Tracking
    await user.click(within(dialog).getByRole("button", { name: /create project/i }));

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByText("Orbital Launch")).toBeInTheDocument();
    expect(screen.getByText(/6 active/)).toBeInTheDocument();
    // No network write in demo mode — creation is purely local.
    expect(fetchMock).toHaveBeenCalledTimes(1); // only the page's initial GET
  });

  it("a demo-created project is persisted and survives a remount (simulated reload)", async () => {
    const user = userEvent.setup();
    const { unmount } = renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });

    const dialog = await openWizard(user);
    await user.type(within(dialog).getByPlaceholderText("e.g. Mobile App Revamp"), "Orbital Launch");
    await user.click(within(dialog).getByRole("button", { name: /continue/i })); // -> Team
    await user.click(within(dialog).getByRole("button", { name: /continue/i })); // -> Tracking
    await user.click(within(dialog).getByRole("button", { name: /create project/i }));
    expect(screen.getByText("Orbital Launch")).toBeInTheDocument();

    // Creation must persist through the tenant-data layer, not just React state.
    const saved = localStorage.getItem("dosi-projects-created-w1");
    expect(saved).toContain("Orbital Launch");

    // Simulate a full reload: module-level dataset caches are dropped, web
    // storage (the persisted key) survives — the fresh render must read the
    // project back through datasetFor()'s localStorage path.
    unmount();
    simulateReload();

    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });
    expect(screen.getByText("Orbital Launch")).toBeInTheDocument();
    expect(screen.getByText(/6 active/)).toBeInTheDocument(); // 5 seeded + the survivor
  });

  it("cancel closes the wizard without adding a card", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });

    const dialog = await openWizard(user);
    await user.type(within(dialog).getByPlaceholderText("e.g. Mobile App Revamp"), "Half Finished");
    await user.click(within(dialog).getByRole("button", { name: /cancel/i }));

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByText("Half Finished")).not.toBeInTheDocument();
    expect(screen.getByText(/5 active/)).toBeInTheDocument();
  });

  it("cancel discards typed input — reopening starts on a blank Details step", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });

    const dialog = await openWizard(user);
    await user.type(within(dialog).getByPlaceholderText("e.g. Mobile App Revamp"), "Half Finished");
    await user.click(within(dialog).getByRole("button", { name: /cancel/i }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    const reopened = await openWizard(user);
    expect(within(reopened).getByPlaceholderText("e.g. Mobile App Revamp")).toHaveValue("");
    expect(within(reopened).getByPlaceholderText("Short summary")).toHaveValue("");
  });

  it("the Archived tab shows archived projects and hides active ones", async () => {
    const user = userEvent.setup();
    renderAsRole(<ProjectsPage />, "owner", { route: "/projects" });

    expect(screen.getByText("Dosi Web Platform")).toBeInTheDocument();
    expect(screen.queryByText("Data Pipeline")).not.toBeInTheDocument(); // p5 is archived

    await user.click(screen.getByRole("tab", { name: "Archived" }));

    expect(screen.getByText("Data Pipeline")).toBeInTheDocument();
    expect(screen.queryByText("Dosi Web Platform")).not.toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Active" }));
    expect(screen.getByText("Dosi Web Platform")).toBeInTheDocument();
  });

  it("a worker gets no create affordance and only their own projects", () => {
    renderAsRole(<ProjectsPage />, "worker", { route: "/projects" });

    expect(screen.queryByRole("button", { name: /new project/i })).not.toBeInTheDocument();
    // u2 (seeded worker) is a member of Mobile App Revamp but not Acme Corp CRM.
    expect(screen.getByText("Mobile App Revamp")).toBeInTheDocument();
    expect(screen.queryByText("Acme Corp CRM")).not.toBeInTheDocument();
  });
});
