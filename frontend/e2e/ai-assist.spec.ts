// AI assist in demo mode (AI assist spec §18, §19.4): the sample cards, a
// keyboard-only Generate, the Monitor and Timesheet cards, the task-draft
// dialog's focus handling, Ask AI from the command palette, and an axe scan
// of the open drawer and a ready card.
//
// The demo answers from the frozen demo data in the browser. The fixtures
// already abort every request to the API origin; each test also records
// every request the page makes and asserts none of them was an API call
// (any origin's /api/ or /connect/ path), so a demo AI flow that reached for
// a server would fail here even though the abort hides it.
import AxeBuilder from "@axe-core/playwright";
import type { Locator, Page, TestInfo } from "@playwright/test";
import { test, expect, primeSession, API_ORIGINS } from "./fixtures";

/** Every API request the page attempts, from now on. */
function apiRequests(page: Page): string[] {
  const seen: string[] = [];
  page.on("request", (request) => {
    const url = request.url();
    const { pathname } = new URL(url);
    if (API_ORIGINS.some((origin) => url.startsWith(origin)) || pathname.startsWith("/api/") || pathname.startsWith("/connect/")) {
      seen.push(`${request.method()} ${url}`);
    }
  });
  return seen;
}

/** Presses Tab until `target` has focus (a keyboard user's path to it), failing after `max` presses. */
async function tabTo(page: Page, target: Locator, max = 150): Promise<void> {
  for (let i = 0; i < max; i++) {
    await page.keyboard.press("Tab");
    if (await target.evaluate((el) => el === document.activeElement)) return;
  }
  throw new Error(`Tab never reached ${target}`);
}

/** The same gate as e2e/a11y.spec.ts: zero serious or critical violations, the rest recorded. */
async function scan(page: Page, testInfo: TestInfo, include: string): Promise<void> {
  const results = await new AxeBuilder({ page }).include(include).analyze();
  for (const v of results.violations.filter((v) => v.impact !== "serious" && v.impact !== "critical")) {
    testInfo.annotations.push({ type: `a11y-${v.impact ?? "unknown"}`, description: `${v.id} (${v.nodes.length} nodes): ${v.help}` });
  }
  const gate = results.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map((v) => `${v.id} [${v.impact}] (${v.nodes.length} nodes): ${v.help}`);
  expect(gate).toEqual([]);
}

test.beforeEach(async ({ context }) => {
  // Seeded identities (src/lib/demo-tenants.ts): u1 owns w1, u2 is one of its members.
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
});

test("the dashboard's AI card is marked as a sample @smoke", async ({ page }) => {
  const api = apiRequests(page);
  await page.goto("/dashboard");
  const card = page.getByRole("region", { name: "This week at a glance" });
  await expect(card.getByRole("heading", { name: "This week at a glance" })).toBeVisible();
  await expect(card.getByText("Sample", { exact: true })).toBeVisible();
  await expect(card.getByRole("button", { name: "Generate" })).toBeVisible();
  expect(api).toEqual([]);
});

test("a summary can be generated and read with the keyboard alone", async ({ page }) => {
  const api = apiRequests(page);
  await page.goto("/dashboard");
  const card = page.getByRole("region", { name: "This week at a glance" });
  const generate = card.getByRole("button", { name: "Generate" });
  await expect(generate).toBeVisible();

  await tabTo(page, generate);
  await page.keyboard.press("Enter");
  await expect(card.getByRole("status")).toHaveText("Summary ready");
  await expect(card.getByText(/^The team tracked /).first()).toBeVisible();
  // The pressed button's place is taken by Regenerate, and focus goes with it rather than back to the page.
  await expect(card.getByRole("button", { name: "Regenerate" })).toBeFocused();

  const figures = card.getByText("Show the figures used");
  await tabTo(page, figures, 40);
  await page.keyboard.press("Enter");
  await expect(card.locator("details")).toHaveAttribute("open", "");
  await expect(card.locator("details dl")).toBeVisible();
  expect(api).toEqual([]);
});

test("a member's Monitor explains their own day", async ({ page, context }) => {
  await primeSession(context, { userId: "u2", workspaceId: "w1" });
  const api = apiRequests(page);
  await page.goto("/monitor");
  const card = page.getByRole("region", { name: "Explain this day" });
  await expect(card.getByText("Sample", { exact: true })).toBeVisible();
  await card.getByRole("button", { name: "Generate" }).click();
  await expect(card.getByRole("status")).toHaveText("Summary ready");
  await expect(card.getByRole("button", { name: "Regenerate" })).toBeVisible();
  expect(api).toEqual([]);
});

test("the timesheet's week review reviews the week on screen", async ({ page }) => {
  const api = apiRequests(page);
  await page.goto("/timesheet");
  const card = page.getByRole("region", { name: "Week review" });
  await expect(card.getByText("Sample", { exact: true })).toBeVisible();
  await card.getByRole("button", { name: "Generate" }).click();
  // The demo week is tidy: no rule finds a candidate, which a review answers as "no-data". That must
  // never read as "nothing was tracked" on a week full of tracked time.
  await expect(card.getByRole("status")).toHaveText("Nothing in this period's tracking looks unusual.");
  await expect(card.getByText("Nothing was tracked for this period.")).toHaveCount(0);
  expect(api).toEqual([]);
});

test("the task-draft dialog keeps focus inside and gives it back on Escape", async ({ page }) => {
  const api = apiRequests(page);
  await page.goto("/projects/p1");
  const opener = page.getByRole("button", { name: "Draft tasks with AI" });
  await opener.focus();
  await page.keyboard.press("Enter");

  const dialog = page.getByRole("dialog", { name: "Draft tasks with AI" });
  await expect(dialog.getByLabel("Goal")).toBeVisible();
  const inside = () => dialog.evaluate((el) => el.contains(document.activeElement));
  await expect.poll(inside).toBe(true);
  // Round the whole dialog both ways: focus never escapes to the page behind it.
  for (let i = 0; i < 12; i++) {
    await page.keyboard.press("Tab");
    expect(await inside()).toBe(true);
  }
  for (let i = 0; i < 12; i++) {
    await page.keyboard.press("Shift+Tab");
    expect(await inside()).toBe(true);
  }

  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(opener).toBeFocused();
  expect(api).toEqual([]);
});

test("Ask AI opens from the command palette and answers a suggested question with its sources", async ({ page }) => {
  const api = apiRequests(page);
  await page.goto("/dashboard");
  await expect(page.getByText("Organization overview for today.")).toBeVisible();

  await page.keyboard.press("ControlOrMeta+k");
  const palette = page.getByRole("dialog", { name: "Search and jump" });
  await expect(palette).toBeVisible();
  await palette.getByRole("option", { name: /Ask AI\.\.\./ }).click();

  const drawer = page.getByRole("dialog", { name: "Ask AI" });
  await expect(drawer).toBeVisible();
  await expect(drawer.getByText("Sample answers from the demo data")).toBeVisible();
  // A demo starts on the first suggested question (its composer is off).
  const chips = drawer.getByRole("group", { name: "Suggested questions" });
  await expect(chips.getByRole("button").first()).toBeFocused();
  await chips.getByRole("button", { name: "How did this week go?" }).click();

  const log = drawer.getByRole("log", { name: "Conversation" });
  await expect(log.getByText("How did this week go?")).toBeVisible();
  await expect(log.getByText(/^The team tracked .* in the last 7 days/)).toBeVisible();
  await expect(log.getByText("Sample", { exact: true })).toBeVisible();
  await expect(log.getByRole("link", { name: "Weekly summary" })).toHaveAttribute("href", "/reports/weekly");
  await expect(drawer.getByRole("status")).toHaveText("Answer ready");

  // Escape closes it and hands focus back to the page.
  await page.keyboard.press("Escape");
  await expect(drawer).toBeHidden();
  expect(api).toEqual([]);
});

test("a page's suggested question in the palette asks it straight away", async ({ page }) => {
  const api = apiRequests(page);
  await page.goto("/dashboard");
  await expect(page.getByText("Organization overview for today.")).toBeVisible();
  await page.keyboard.press("ControlOrMeta+k");
  await page.getByRole("dialog", { name: "Search and jump" }).getByRole("option", { name: /Which projects took the most time this week\?/ }).click();
  const log = page.getByRole("dialog", { name: "Ask AI" }).getByRole("log", { name: "Conversation" });
  await expect(log.getByText("These projects took the most time in the last 7 days:")).toBeVisible();
  await expect(log.getByRole("link", { name: "Projects" })).toHaveAttribute("href", "/projects");
  expect(api).toEqual([]);
});

test("the open Ask AI drawer has no serious or critical axe violations @a11y", async ({ page }, testInfo) => {
  await page.goto("/dashboard");
  await page.getByRole("button", { name: "Ask AI", exact: true }).click();
  const drawer = page.getByRole("dialog", { name: "Ask AI" });
  await drawer.getByRole("group", { name: "Suggested questions" }).getByRole("button", { name: "How did this week go?" }).click();
  await expect(drawer.getByRole("status")).toHaveText("Answer ready");
  await drawer.getByText("Figures used").click();
  await scan(page, testInfo, '[role="dialog"]');
});

test("a ready AI card has no serious or critical axe violations @a11y", async ({ page }, testInfo) => {
  await page.goto("/dashboard");
  const card = page.getByRole("region", { name: "This week at a glance" });
  await card.getByRole("button", { name: "Generate" }).click();
  await expect(card.getByRole("status")).toHaveText("Summary ready");
  await card.getByText("Show the figures used").click();
  const id = await card.getAttribute("aria-labelledby");
  await scan(page, testInfo, `section[aria-labelledby="${id}"]`);
});
