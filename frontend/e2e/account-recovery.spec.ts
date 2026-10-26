import AxeBuilder from "@axe-core/playwright";
import { test, expect } from "@playwright/test";
import { API_ORIGINS } from "./fixtures";

// Account recovery (FE-09) in a real browser, signed OUT (no session cookie):
// the pages emailed links open are public, ask the server anonymously, and
// answer the same whether or not the address has an account. The API is
// faked per test; nothing reaches a real backend.

async function fakeApi(page: import("@playwright/test").Page, handle: (path: string, body: unknown) => { status: number; body?: unknown }) {
  for (const origin of API_ORIGINS) {
    await page.route(`${origin}/**`, async (route) => {
      const url = new URL(route.request().url());
      const raw = route.request().postData();
      const answer = handle(url.pathname, raw ? JSON.parse(raw) : null);
      await route.fulfill({ status: answer.status, contentType: "application/json", body: answer.body === undefined ? "" : JSON.stringify(answer.body) });
    });
  }
}

test("'Forgot password?' leads to a public page that never says whether the account exists @smoke", async ({ page }) => {
  const asked: unknown[] = [];
  await fakeApi(page, (path, body) => {
    if (path === "/api/account/send-password-reset-code") {
      asked.push(body);
      return { status: 204 };
    }
    return { status: 404, body: {} };
  });
  await page.goto("/login");
  await page.getByLabel("Email").fill("ana@acme.example");
  await page.getByRole("link", { name: "Forgot password?" }).click();
  await expect(page).toHaveURL(/\/forgot-password\?email=ana%40acme\.example$/);
  await expect(page.getByLabel("Email")).toHaveValue("ana@acme.example");
  await page.getByRole("button", { name: "Email me a link" }).click();
  await expect(page.getByRole("status")).toContainText("If ana@acme.example belongs to an account here");
  expect(asked).toEqual([{ email: "ana@acme.example", appName: "Dosi-Tracker web" }]);
});

test("a reset link opens signed out, checks the link and sets the new password", async ({ page }) => {
  await fakeApi(page, (path) => {
    if (path === "/api/account/verify-password-reset-token") return { status: 200, body: true };
    if (path === "/api/account/reset-password") return { status: 204 };
    return { status: 404, body: {} };
  });
  await page.goto("/reset-password?userId=0b8a6f7e-1c2d-4e5f-8a9b-0c1d2e3f4a5b&resetToken=abc");
  await expect(page).toHaveURL(/\/reset-password\?/);
  await page.getByLabel("New password", { exact: true }).fill("brand-new-1");
  await page.getByLabel("Confirm new password").fill("brand-new-1");
  await page.getByRole("button", { name: "Save and continue" }).click();
  await expect(page.getByRole("link", { name: "Continue to sign in" })).toBeVisible();
});

test("an incomplete reset link explains itself and offers a new one", async ({ page }) => {
  await page.goto("/reset-password");
  // (Next.js keeps a route announcer with role "alert" in every page: look for the page's own.)
  await expect(page.getByRole("alert").filter({ hasText: /incomplete/i })).toBeVisible();
  await page.getByRole("link", { name: "Ask for a new link" }).click();
  await expect(page).toHaveURL(/\/forgot-password$/);
});

test("the recovery pages have no a11y violations", async ({ page }) => {
  for (const path of ["/forgot-password", "/reset-password", "/confirm-email"]) {
    await page.goto(path);
    await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
    const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
    expect(results.violations, path).toEqual([]);
  }
});
