// Project management flows for an owner in the demo workspace.
import { test, expect, primeSession } from "./fixtures";

test("owner creates a project through the wizard and sees it in the list", async ({
  page,
  context,
}) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/projects");

  await page.getByRole("button", { name: "New project" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading", { name: "Create project" })).toBeVisible();

  // Step 1 — details. Continue is gated on a non-empty name.
  await expect(dialog.getByRole("button", { name: "Continue" })).toBeDisabled();
  await dialog.getByPlaceholder("e.g. Mobile App Revamp").fill("E2E Orbit Project");
  await dialog.getByPlaceholder("Short summary").fill("Created by the Playwright suite");
  await dialog.getByRole("button", { name: "Continue" }).click();

  // Step 2 — team.
  await expect(dialog.getByText("Select team members to add to this project.")).toBeVisible();
  await dialog.getByRole("button", { name: /Tanvir Hasan/ }).click();
  await dialog.getByRole("button", { name: "Continue" }).click();

  // Step 3 — tracking, then create.
  await expect(dialog.getByText("Snapshot interval")).toBeVisible();
  await dialog.getByRole("button", { name: "Create project" }).click();

  // The wizard closes and the new project card is in the active list.
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("main").getByText("E2E Orbit Project")).toBeVisible();
  await expect(page.getByRole("main").getByText("Created by the Playwright suite")).toBeVisible();
});

test("a demo-created project survives a page reload", async ({ page, context }) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/projects");

  await page.getByRole("button", { name: "New project" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByPlaceholder("e.g. Mobile App Revamp").fill("Reload Survivor");
  await dialog.getByRole("button", { name: "Continue" }).click(); // -> Team
  await dialog.getByRole("button", { name: "Continue" }).click(); // -> Tracking
  await dialog.getByRole("button", { name: "Create project" }).click();
  await expect(page.getByRole("main").getByText("Reload Survivor")).toBeVisible();

  // Persisted through the tenant-data layer, not just React state.
  const stored = await page.evaluate(() => localStorage.getItem("dosi-projects-created-w1"));
  expect(stored).toContain("Reload Survivor");

  // A full reload drops all module state; the card must be read back from storage.
  await page.reload();
  await expect(page.getByRole("main").getByText("Reload Survivor")).toBeVisible();
});

test("archived filter separates archived projects from active ones", async ({ page, context }) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/projects");

  // Active tab (default): the archived seed project is hidden.
  await expect(page.getByRole("main").getByText("Dosi Web Platform")).toBeVisible();
  await expect(page.getByRole("main").getByText("Data Pipeline")).toHaveCount(0);

  // Archived tab: only the archived project, labeled as such on its card.
  await page.getByRole("tab", { name: "Archived" }).click();
  const archivedCard = page.getByRole("link", { name: /Data Pipeline/ });
  await expect(archivedCard).toBeVisible();
  await expect(archivedCard.getByText("Archived", { exact: true })).toBeVisible();
  await expect(page.getByRole("main").getByText("Dosi Web Platform")).toHaveCount(0);

  // Back to active restores the working set.
  await page.getByRole("tab", { name: "Active" }).click();
  await expect(page.getByRole("main").getByText("Dosi Web Platform")).toBeVisible();
});

test("searching projects narrows the list and can be cleared from the empty state", async ({
  page,
  context,
}) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/projects");

  await page.getByPlaceholder("Search projects…").fill("Design");
  await expect(page.getByRole("main").getByText("Design System")).toBeVisible();
  await expect(page.getByRole("main").getByText("Dosi Web Platform")).toHaveCount(0);

  await page.getByPlaceholder("Search projects…").fill("zzz-no-such-project");
  await expect(page.getByText("No matching projects")).toBeVisible();
  await page.getByRole("button", { name: "Clear search" }).click();
  await expect(page.getByRole("main").getByText("Dosi Web Platform")).toBeVisible();
});
