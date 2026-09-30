// Automated accessibility gate: ZERO serious/critical axe violations on the
// core screens. Moderate/minor findings are recorded as annotations for the
// report but do not fail the gate.
import AxeBuilder from "@axe-core/playwright";
import type { Page, TestInfo } from "@playwright/test";
import { test, expect, primeSession } from "./fixtures";

async function scan(page: Page, testInfo: TestInfo): Promise<void> {
  const results = await new AxeBuilder({ page }).analyze();

  const lower = results.violations.filter(
    (v) => v.impact !== "serious" && v.impact !== "critical",
  );
  for (const v of lower) {
    testInfo.annotations.push({
      type: `a11y-${v.impact ?? "unknown"}`,
      description: `${v.id} (${v.nodes.length} nodes): ${v.help}`,
    });
  }

  const gate = results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id} [${v.impact}] (${v.nodes.length} nodes): ${v.help}`);
  expect(gate).toEqual([]);
}

test("login page has no serious or critical axe violations", async ({ browser, baseURL }, testInfo) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${baseURL}/login`);
  await expect(page.getByRole("heading", { name: "Welcome back" })).toBeVisible();
  await scan(page, testInfo);
  await context.close();
});

test("dashboard has no serious or critical axe violations", async ({ page, context }, testInfo) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/dashboard");
  await expect(page.getByText("Organization overview for today.")).toBeVisible();
  await scan(page, testInfo);
});

test("projects page has no serious or critical axe violations", async ({ page, context }, testInfo) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/projects");
  await expect(page.getByRole("heading", { level: 1, name: "Projects" })).toBeVisible();
  await scan(page, testInfo);
});

test("team page has no serious or critical axe violations", async ({ page, context }, testInfo) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/team");
  await expect(page.getByRole("heading", { level: 1, name: "Team" })).toBeVisible();
  await scan(page, testInfo);
});

test("host overview has no serious or critical axe violations", async ({ page, context }, testInfo) => {
  await primeSession(context, { userId: "host" });
  await page.goto("/host");
  await expect(page.getByRole("heading", { level: 1, name: /Platform Overview/ })).toBeVisible();
  await scan(page, testInfo);
});
