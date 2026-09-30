// Dark mode: the toggle flips the documentElement class and the choice
// persists across a reload (localStorage "dosi-theme").
import { test, expect, primeSession } from "./fixtures";

function hasDarkClass(page: import("@playwright/test").Page): Promise<boolean> {
  return page.evaluate(() => document.documentElement.classList.contains("dark"));
}

test("toggling dark mode flips the document theme and persists across reload", async ({
  page,
  context,
}) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/dashboard");
  await expect(page.getByText("Organization overview for today.")).toBeVisible();

  // The context runs with a light color scheme and no stored preference.
  await expect.poll(() => hasDarkClass(page)).toBe(false);

  await page.getByRole("button", { name: "Toggle theme" }).click();
  await expect.poll(() => hasDarkClass(page)).toBe(true);

  // Reload: the preference is re-applied from storage.
  await page.reload();
  await expect(page.getByText("Organization overview for today.")).toBeVisible();
  await expect.poll(() => hasDarkClass(page)).toBe(true);

  // Toggling back returns to light and that also persists.
  await page.getByRole("button", { name: "Toggle theme" }).click();
  await expect.poll(() => hasDarkClass(page)).toBe(false);
  await page.reload();
  await expect(page.getByText("Organization overview for today.")).toBeVisible();
  await expect.poll(() => hasDarkClass(page)).toBe(false);
});
