// Regression gates for UX blockers found by the production-readiness audit.
// Each of these reproduced a real defect that made part of the app unusable;
// they are written to fail loudly if the defect returns.
import AxeBuilder from "@axe-core/playwright";
import { test, expect, primeSession } from "./fixtures";

test("dashboard KPI cards are visible when the OS asks for reduced motion @smoke", async ({
  browser,
  baseURL,
}) => {
  // The staggered entrance animation supplied the final opacity, so disabling
  // animations left the four headline stats permanently transparent — a blank
  // band where the most important numbers on the page should be.
  const context = await browser.newContext({ reducedMotion: "reduce" });
  await context.addCookies([{ name: "dosi-token", value: "1", url: baseURL! }]);
  await context.addInitScript(() => localStorage.setItem("dosi-user", "u1"));
  const page = await context.newPage();

  await page.goto(`${baseURL}/dashboard`);
  await expect(page.getByText("Organization overview for today.")).toBeVisible();

  const opacities = await page.evaluate(() =>
    Array.from(document.querySelectorAll(".stagger > *")).map((el) => getComputedStyle(el).opacity),
  );
  expect(opacities.length).toBeGreaterThan(0);
  expect(opacities.every((o) => Number(o) > 0.99)).toBe(true);

  await context.close();
});

test("every activity session card can be reached and opened with the keyboard", async ({
  page,
  context,
}) => {
  // The session cards were clickable <div>s: the page's primary action was
  // invisible to the keyboard, so the whole content area was a dead zone.
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/activities");

  const cards = page.getByRole("button", { name: /^Open session:/ });
  await expect(cards.first()).toBeVisible();

  await cards.first().focus();
  await expect(cards.first()).toBeFocused();

  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog")).toBeVisible();
});

test("an open dialog keeps keyboard focus inside it and restores focus on close", async ({
  page,
  context,
}) => {
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/projects");

  const trigger = page.getByRole("button", { name: /new project/i });
  await trigger.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();

  // Focus starts inside, and Tab never escapes to the page behind.
  for (let i = 0; i < 12; i++) {
    const inside = await dialog.evaluate((d) => d.contains(document.activeElement));
    expect(inside).toBe(true);
    await page.keyboard.press("Tab");
  }

  await page.keyboard.press("Escape");
  await expect(dialog).not.toBeVisible();
  await expect(trigger).toBeFocused();
});

test("the platform admin is never stranded on a route their role cannot open", async ({
  page,
  context,
}) => {
  // The blocked-route fallback always pointed at /dashboard, which the host
  // also cannot open — clicking it re-blocked them with no way to the console.
  await primeSession(context, { userId: "host" });
  await page.goto("/projects");

  await expect(page.getByRole("heading", { name: /restricted area/i })).toBeVisible();
  const back = page.getByRole("link", { name: /back to platform console/i });
  await expect(back).toBeVisible();

  await back.click();
  await expect(page).toHaveURL(/\/host$/);
  await expect(page.getByRole("heading", { level: 1, name: /Platform Overview/ })).toBeVisible();
});

test("the attendance presence matrix renders one row per member with a mark per day", async ({
  page,
  context,
}) => {
  // `display: contents` was not applying, so every member row collapsed into a
  // single grid cell and the names flowed sideways through the day columns.
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await page.goto("/reports/attendance");

  const table = page.getByRole("table", { name: /daily presence/i });
  await expect(table).toBeVisible();

  const bodyRows = table.locator("tbody tr");
  const rowCount = await bodyRows.count();
  expect(rowCount).toBeGreaterThan(1);

  // Each member row must carry a presence mark for every day column.
  const dayColumns = await table.locator("thead th").count();
  const firstRowCells = await bodyRows.first().locator("td").count();
  expect(firstRowCells).toBe(dayColumns - 1); // minus the "Member" header column
  await expect(bodyRows.first().getByText(/Present|Absent/).first()).toBeAttached();
});

test("dark theme has no serious or critical contrast failures on the core screens", async ({
  page,
  context,
}, testInfo) => {
  // Dark mode was never scanned: primary buttons were white-on-teal at 2.47:1
  // and success/info/primary had collapsed into one indistinguishable color.
  await primeSession(context, { userId: "u1", workspaceId: "w1" });
  await context.addInitScript(() => localStorage.setItem("dosi-theme", "dark"));

  for (const route of ["/dashboard", "/projects", "/team", "/settings"]) {
    await page.goto(route);
    await expect(page.locator("html.dark")).toBeAttached();

    const results = await new AxeBuilder({ page }).withRules(["color-contrast"]).analyze();
    const serious = results.violations
      .filter((v) => v.impact === "serious" || v.impact === "critical")
      .flatMap((v) => v.nodes.map((n) => `${route} ${v.id}: ${n.html.slice(0, 90)}`));
    testInfo.annotations.push({ type: "dark-scan", description: `${route}: ${serious.length} nodes` });
    expect(serious).toEqual([]);
  }
});
