// Role-based access: what each role can see in the nav and reach by URL.
// Seeded identities (src/lib/mock-data.ts): u1 owner, u2 worker, u7 client.
import { test, expect, primeSession } from "./fixtures";

test("a worker's sidebar hides Reports, Team and Billing", async ({ page, context }) => {
  await primeSession(context, { userId: "u2", workspaceId: "w1" });
  await page.goto("/dashboard");

  const nav = page.getByRole("navigation");
  await expect(nav.getByRole("link", { name: "Dashboard" })).toBeVisible();
  await expect(nav.getByRole("link", { name: "Projects" })).toBeVisible();
  await expect(nav.getByRole("link", { name: "Timesheet" })).toBeVisible();
  await expect(nav.getByRole("link", { name: "Reports" })).toHaveCount(0);
  await expect(nav.getByRole("link", { name: "Team" })).toHaveCount(0);
  await expect(nav.getByRole("link", { name: "Billing & Plan" })).toHaveCount(0);
});

test("a worker navigating straight to /billing hits the Restricted area, not billing content", async ({
  page,
  context,
}) => {
  await primeSession(context, { userId: "u2", workspaceId: "w1" });
  await page.goto("/billing");

  await expect(page.getByRole("heading", { name: "Restricted area" })).toBeVisible();
  await expect(page.getByText(/Your role \(Member\)/)).toBeVisible();
  // None of the actual billing content leaks through the guard.
  await expect(page.getByRole("heading", { name: "Billing & Plan" })).toHaveCount(0);
  await expect(page.getByRole("link", { name: "Back to dashboard" })).toBeVisible();
});

test("an owner sees the real billing page on /billing", async ({ page, context }) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/billing");

  await expect(page.getByRole("heading", { level: 1, name: "Billing & Plan" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Restricted area" })).toHaveCount(0);
});

test("a client only gets the permitted nav entries (Dashboard and Settings)", async ({
  page,
  context,
}) => {
  await primeSession(context, { userId: "u7", workspaceId: "w1" });
  await page.goto("/dashboard");

  const nav = page.getByRole("navigation");
  await expect(nav.getByRole("link", { name: "Dashboard" })).toBeVisible();
  await expect(nav.getByRole("link", { name: "Settings" })).toBeVisible();
  // Everything else is hidden for the read-only client role.
  await expect(nav.getByRole("link", { name: "Projects" })).toHaveCount(0);
  await expect(nav.getByRole("link", { name: "Team" })).toHaveCount(0);
  await expect(nav.getByRole("link", { name: "Member Monitor" })).toHaveCount(0);
  await expect(nav.getByRole("link", { name: "Activity" })).toHaveCount(0);
  await expect(nav.getByRole("link", { name: "Timesheet" })).toHaveCount(0);
  await expect(nav.getByRole("link", { name: "Reports" })).toHaveCount(0);
  await expect(nav.getByRole("link", { name: "Billing & Plan" })).toHaveCount(0);
});

test("a client navigating straight to /reports is blocked by the guard", async ({ page, context }) => {
  await primeSession(context, { userId: "u7", workspaceId: "w1" });
  await page.goto("/reports");

  await expect(page.getByRole("heading", { name: "Restricted area" })).toBeVisible();
  await expect(page.getByRole("heading", { level: 1, name: "Reports" })).toHaveCount(0);
});
