// Multi-tenant workspace switching through the real sidebar switcher UI.
// Each tenant is an isolated dataset: switching must swap branding AND data.
import { test, expect, primeSession } from "./fixtures";

test("switching to Acme Studio swaps branding and roster, and switching back restores Dosi Labs", async ({
  page,
  context,
}) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/team");

  // Baseline: Dosi Labs chrome + Dosi Labs roster.
  const switcher = page.getByRole("button", { name: /Dosi Labs/ });
  await expect(switcher).toBeVisible();
  await expect(page.getByRole("main").getByText("Ayesha Rahman")).toBeVisible();

  // Switch to Acme Studio.
  await switcher.click();
  await page.getByRole("button", { name: /Acme Studio/ }).click();

  // Chrome shows Acme branding and the plan badge for its trial.
  await expect(page.getByRole("button", { name: /Acme Studio/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /Dosi Labs/ })).toHaveCount(0);

  // Roster is DIFFERENT data — Acme members appear, Dosi members are gone.
  await expect(page.getByRole("main").getByText("Diego Alvarez")).toBeVisible();
  await expect(page.getByRole("main").getByText("Lucia Romero")).toBeVisible();
  await expect(page.getByRole("main").getByText("Ayesha Rahman")).toHaveCount(0);

  // Switch back to Dosi Labs.
  await page.getByRole("button", { name: /Acme Studio/ }).click();
  await page.getByRole("button", { name: /Dosi Labs/ }).click();
  await expect(page.getByRole("button", { name: /Dosi Labs/ })).toBeVisible();
  await expect(page.getByRole("main").getByText("Ayesha Rahman")).toBeVisible();
  await expect(page.getByRole("main").getByText("Diego Alvarez")).toHaveCount(0);
});

test("creating a workspace through the switcher makes it active and it survives a reload", async ({
  page,
}) => {
  // No primeSession here: its init script would re-stamp the seed identity on
  // reload and clobber the app-persisted workspace selection. The session
  // defaults to the owner of Dosi Labs anyway.
  await page.goto("/dashboard");

  // Open the switcher and start the create flow.
  await page.getByRole("button", { name: /Dosi Labs/ }).click();
  await page.getByRole("button", { name: "Create workspace" }).click();

  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading", { name: "Create a workspace" })).toBeVisible();
  await dialog.getByPlaceholder("Acme Corp").fill("Falcon Works");
  // The derived tenant URL is previewed from the name.
  await expect(dialog.getByText("falcon-works.dositracker.app")).toBeVisible();
  await dialog.getByRole("button", { name: "Create workspace" }).click();

  // Feedback + the new tenant becomes the active workspace.
  await expect(page.getByText("Workspace created")).toBeVisible();
  await expect(page.getByRole("button", { name: /Falcon Works/ })).toBeVisible();

  // Persistence: the workspace and its selection live in localStorage.
  await page.reload();
  await expect(page.getByRole("button", { name: /Falcon Works/ })).toBeVisible();

  // It is listed alongside the seed tenants in the switcher.
  await page.getByRole("button", { name: /Falcon Works/ }).click();
  await expect(page.getByRole("button", { name: /Dosi Labs/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /Acme Studio/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /Nimbus Co/ })).toBeVisible();
});
