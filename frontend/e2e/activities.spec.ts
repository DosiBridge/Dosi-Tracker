// Activity review page in demo mode.
//
// With the backend absent the page falls back to the seeded tenant activities
// from "@/lib/tenant-data" — the same demo-data idiom as /projects, /team and
// /dashboard — scoped by role (owners see the team, workers only themselves).
import { test, expect, primeSession } from "./fixtures";

test("demo mode lists seeded sessions for the owner instead of the empty state", async ({
  page,
  context,
}) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/activities");

  // The seeded tenant data backs the page — no empty state, non-zero count.
  await expect(page.getByText("No activities match your filters")).toHaveCount(0);
  await expect(page.getByText(/[1-9]\d* tracked sessions/)).toBeVisible();
});

test("demo mode renders the worker's own seeded activity cards", async ({ page, context }) => {
  await primeSession(context, { userId: "u2", workspaceId: "w1" }); // Tanvir Hasan (worker)
  await page.goto("/activities");

  await expect(page.getByText("No activities match your filters")).toHaveCount(0);
  // Workers get no member filter select, so this name can only come from a
  // seeded activity card — and it is scoped to the worker's own rows.
  await expect(page.getByRole("main").getByText("Tanvir Hasan").first()).toBeVisible();
  await expect(page.getByRole("main").getByText("Ayesha Rahman")).toHaveCount(0);
});

test("the demo-mode honesty notice about mock previews is visible", async ({ page, context }) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/activities");
  await expect(page.getByText("previews are mock placeholders")).toBeVisible();
});

test("view toggle switches between Sessions and Screens and updates the URL", async ({
  page,
  context,
}) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/activities");

  await expect(page.getByRole("tab", { name: "Sessions" })).toBeVisible();
  await page.getByRole("tab", { name: "Screens" }).click();
  await expect(page).toHaveURL(/\/activities\?view=screens$/);

  await page.getByRole("tab", { name: "Sessions" }).click();
  await expect(page).toHaveURL(/\/activities$/);
});

test("owner can open the filter panel and apply a date-range preset", async ({ page, context }) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/activities");

  // The member filter is an owner/admin capability.
  await expect(page.getByRole("combobox").filter({ hasText: "All members" })).toBeVisible();

  await page.getByRole("button", { name: /Filters/ }).click();
  await expect(page.getByText("Date range", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Today", exact: true }).click();

  // The applied filter surfaces as a removable chip and in the filter count.
  await expect(page.getByRole("button", { name: /Date: Today/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /Filters/ })).toContainText("1");

  // Removing the chip returns to the default range.
  await page.getByRole("button", { name: /Date: Today/ }).click();
  await expect(page.getByRole("button", { name: /Date: Today/ })).toHaveCount(0);
});

test("filtering by member updates the active filter chips", async ({ page, context }) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/activities");

  await page
    .getByRole("combobox")
    .filter({ hasText: "All members" })
    .selectOption({ label: "Tanvir Hasan" });
  await expect(page.getByRole("button", { name: "Member: Tanvir Hasan" })).toBeVisible();
});
