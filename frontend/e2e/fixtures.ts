// Shared E2E fixtures.
//
// `test` puts every page into DEMO MODE:
//  - the `dosi-token` session cookie is set (the middleware gate passes),
//  - NO localStorage JWT exists (the live-backend hydration path is skipped),
//  - every request to the real API origin is aborted, so a locally running
//    backend can never leak nondeterminism into the suite.
// The app then renders its deterministic seeded demo data (frozen NOW clock).
//
// Tests must be order-independent: each test gets a fresh context, and state
// the app persists (localStorage) dies with the context. Never chain tests.
import { test as base, expect } from "@playwright/test";

export const API_ORIGINS = ["https://localhost:44342", "http://localhost:8080"];

export const test = base.extend({
  context: async ({ context, baseURL }, use) => {
    await context.addCookies([
      {
        name: "dosi-token",
        value: "1",
        url: baseURL!,
      },
    ]);
    for (const origin of API_ORIGINS) {
      await context.route(`${origin}/**`, (route) => route.abort());
    }
    await use(context);
  },
});

export { expect };

/** Switch the demo session to a given seeded user id (must run before page.goto). */
export async function primeSession(
  context: import("@playwright/test").BrowserContext,
  opts: { userId?: string; workspaceId?: string },
): Promise<void> {
  await context.addInitScript(({ userId, workspaceId }) => {
    if (userId) localStorage.setItem("dosi-user", userId);
    if (workspaceId) localStorage.setItem("dosi-workspace", workspaceId);
  }, opts);
}
