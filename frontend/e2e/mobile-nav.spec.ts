// Responsive navigation on a Pixel 7 viewport (runs in the "mobile" project).
import { test, expect, primeSession } from "./fixtures";

test("sidebar collapses to a menu control and opens as a drawer @mobile", async ({
  page,
  context,
}) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/dashboard");

  // The rail is off-canvas: its links exist but sit outside the viewport.
  const nav = page.getByRole("navigation");
  const menuButton = page.getByRole("button", { name: "Open menu" });
  await expect(menuButton).toBeVisible();
  await expect(nav.getByRole("link", { name: "Projects" })).not.toBeInViewport();

  // Opening the menu exposes the full navigation.
  await menuButton.click();
  await expect(nav.getByRole("link", { name: "Dashboard" })).toBeInViewport();
  await expect(nav.getByRole("link", { name: "Projects" })).toBeInViewport();
  await expect(nav.getByRole("link", { name: "Team" })).toBeInViewport();

  // Navigating from the drawer works and closes it again.
  await nav.getByRole("link", { name: "Projects" }).click();
  await expect(page).toHaveURL(/\/projects$/);
  await expect(nav.getByRole("link", { name: "Projects" })).not.toBeInViewport();
});

test("core pages keep their primary actions usable on mobile @mobile", async ({
  page,
  context,
}) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });

  await page.goto("/dashboard");
  await expect(page.getByText("Tracked today")).toBeVisible();
  await expect(page.getByText("Tracked today")).toBeInViewport();

  await page.goto("/projects");
  const newProject = page.getByRole("button", { name: "New project" });
  await expect(newProject).toBeInViewport();

  await page.goto("/team");
  await expect(page.getByRole("button", { name: "Invite member" })).toBeInViewport();
});

test("core pages do not overflow horizontally on mobile @mobile", async ({ page, context }) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });

  for (const path of ["/dashboard", "/projects", "/team", "/settings"]) {
    await page.goto(path);
    // Let the page settle on real content before measuring.
    await expect(page.getByRole("heading", { level: 1 }).first()).toBeVisible();
    const overflow = await page.evaluate(() => {
      const el = document.documentElement;
      return el.scrollWidth - el.clientWidth;
    });
    expect(overflow, `${path} must not scroll horizontally`).toBeLessThanOrEqual(0);
  }
});
