// Cross-browser smoke flows (@smoke runs on chromium, firefox and webkit).
// Keep these lean and unambiguous: one page, one clear behavioral assertion set.
import { test, expect, primeSession } from "./fixtures";

test("unauthenticated visit to /dashboard redirects to /login @smoke", async ({ browser, baseURL }) => {
  // Deliberately NO demo-session cookie: use a pristine context.
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${baseURL}/dashboard`);
  await expect(page).toHaveURL(/\/login$/);
  await expect(page.getByRole("button", { name: "Sign in" })).toBeVisible();
  await context.close();
});

test("login page renders its sign-in form @smoke", async ({ browser, baseURL }) => {
  // A session cookie would bounce /login to /dashboard, so use a pristine context.
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${baseURL}/login`);
  await expect(page.getByRole("heading", { name: "Welcome back" })).toBeVisible();
  await expect(page.getByPlaceholder("acme (leave empty for host sign-in)")).toBeVisible();
  await expect(page.getByText("Email", { exact: true })).toBeVisible();
  await expect(page.getByText("Password", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Sign in" })).toBeEnabled();
  await context.close();
});

test("demo owner sees the KPI dashboard on /dashboard @smoke", async ({ page, context }) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/dashboard");
  await expect(page.getByText("Organization overview for today.")).toBeVisible();
  await expect(page.getByText("Tracked today")).toBeVisible();
  await expect(page.getByText("Active members")).toBeVisible();
  await expect(page.getByText("Avg productivity")).toBeVisible();
  await expect(page.getByText("Captures today")).toBeVisible();
});

test("/projects lists the seeded demo projects @smoke", async ({ page, context }) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/projects");
  await expect(page.getByRole("heading", { level: 1, name: "Projects" })).toBeVisible();
  await expect(page.getByRole("main").getByText("Dosi Web Platform")).toBeVisible();
  await expect(page.getByRole("main").getByText("Mobile App Revamp")).toBeVisible();
  await expect(page.getByRole("main").getByText("Acme Corp CRM")).toBeVisible();
});

test("/team renders the workspace roster @smoke", async ({ page, context }) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/team");
  await expect(page.getByRole("heading", { level: 1, name: "Team" })).toBeVisible();
  await expect(page.getByRole("main").getByText("Ayesha Rahman")).toBeVisible();
  await expect(page.getByRole("main").getByText("Tanvir Hasan")).toBeVisible();
  await expect(page.getByRole("main").getByText("tanvir@dosi.dev")).toBeVisible();
});

test("/settings renders the settings screen @smoke", async ({ page, context }) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/settings");
  await expect(page.getByRole("heading", { level: 1, name: "Settings" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Profile" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Profile" })).toBeVisible();
});

test("host user reaches the /host platform overview @smoke", async ({ page, context }) => {
  await primeSession(context, { userId: "host" });
  await page.goto("/host");
  await expect(page.getByRole("heading", { level: 1, name: /Platform Overview/ })).toBeVisible();
  await expect(page.getByText("Host Console")).toBeVisible();
  await expect(page.getByText("Monthly recurring revenue")).toBeVisible();
});
