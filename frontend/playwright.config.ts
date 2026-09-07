import { defineConfig, devices } from "@playwright/test";

// E2E runs against a production build in DEMO MODE (session cookie set, no
// backend): deterministic mock data with a frozen clock (NOW = 2026-07-14).
// Build first: `npm run build` — then `npm run test:e2e`.
//
// Projects:
//   chromium — full suite (functional + a11y + @visual screenshots)
//   firefox / webkit — cross-browser @smoke flows only
//   mobile — @mobile responsive flows on a Pixel 7 viewport
const PORT = Number(process.env.E2E_PORT ?? 3123);

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? [["github"], ["html", { open: "never" }]] : [["list"], ["html", { open: "never" }]],
  timeout: 30_000,
  expect: {
    // Small anti-aliasing tolerance for @visual screenshots; anything visible fails.
    toHaveScreenshot: { maxDiffPixelRatio: 0.02, animations: "disabled" },
  },
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: "on-first-retry",
    screenshot: "only-on-failure",
    reducedMotion: "reduce",
    timezoneId: "UTC",
    locale: "en-US",
    colorScheme: "light",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] }, grepInvert: /@mobile/ },
    { name: "firefox", use: { ...devices["Desktop Firefox"] }, grep: /@smoke/ },
    { name: "webkit", use: { ...devices["Desktop Safari"] }, grep: /@smoke/ },
    { name: "mobile", use: { ...devices["Pixel 7"] }, grep: /@mobile/ },
  ],
  webServer: {
    command: `npx next start -p ${PORT}`,
    port: PORT,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
