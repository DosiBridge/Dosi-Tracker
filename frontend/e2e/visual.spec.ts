// @visual — screenshot regression of stable layouts (viewport shots).
// Baselines are platform-suffixed (win32); CI must regenerate or skip @visual
// until linux baselines exist. The dashboard greeting is masked because it
// depends on the real wall-clock hour (morning/afternoon/evening). AI assist's
// cards are masked too: their copy is tuned with the prompts and covered by
// e2e/ai-assist.spec.ts, so it shouldn't churn the layout baselines. Their box
// still counts: a card that moves, grows or disappears changes the shot.
import { test, expect, primeSession } from "./fixtures";

test("login page layout is stable @visual", async ({ browser, baseURL }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${baseURL}/login`);
  await expect(page.getByRole("heading", { name: "Welcome back" })).toBeVisible();
  await expect(page).toHaveScreenshot("login.png");
  await context.close();
});

test("dashboard layout is stable @visual", async ({ page, context }) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/dashboard");
  await expect(page.getByText("Organization overview for today.")).toBeVisible();
  // AI assist's sample card arrives in its own chunk: wait for it, so the shot never races the download.
  await expect(page.getByRole("button", { name: "Generate" })).toBeVisible();
  await expect(page).toHaveScreenshot("dashboard.png", {
    mask: [page.getByRole("heading", { level: 1 }), page.getByRole("region", { name: "This week at a glance" })],
  });
});

test("host overview layout is stable @visual", async ({ page, context }) => {
  await primeSession(context, { userId: "host" });
  await page.goto("/host");
  await expect(page.getByRole("heading", { level: 1, name: /Platform Overview/ })).toBeVisible();
  // The platform's AI card loads on demand: wait for it, then mask it like the dashboard's.
  const aiCard = page.getByRole("region", { name: "AI assist" });
  await expect(aiCard).toBeVisible();
  await expect(page).toHaveScreenshot("host-overview.png", { mask: [aiCard] });
});

test("reports hub layout is stable @visual", async ({ page, context }) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/reports");
  await expect(page.getByRole("heading", { level: 1, name: "Reports" })).toBeVisible();
  await expect(page).toHaveScreenshot("reports.png");
});
