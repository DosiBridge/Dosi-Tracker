import { test, expect } from "./fixtures";

// The production server's security headers and Content-Security-Policy, in a
// real browser: every page renders with the per-request nonce and nothing the
// app itself ships is blocked.

test("pages carry a nonce CSP and the static security headers, and no framework banner @smoke", async ({ page }) => {
  const res = await page.goto("/login");
  expect(res).not.toBeNull();
  const headers = res!.headers();
  const csp = headers["content-security-policy"] ?? "";
  expect(csp).toMatch(/script-src 'self' 'nonce-[A-Za-z0-9+/=]+' 'strict-dynamic'/);
  expect(csp).toContain("frame-ancestors 'none'");
  expect(csp).toMatch(/connect-src 'self'[^;]*/);
  expect(headers["x-frame-options"]).toBe("DENY");
  expect(headers["x-content-type-options"]).toBe("nosniff");
  expect(headers["referrer-policy"]).toBe("strict-origin-when-cross-origin");
  expect(headers["permissions-policy"]).toContain("camera=()");
  expect(headers["x-powered-by"]).toBeUndefined();
});

test("a page whose URL ends in an image extension still gets the nonce CSP, and the sign-in check", async ({ page, browser, baseURL }) => {
  for (const path of ["/projects/abc.png", "/invite/x.png", "/reports/a.svg"]) {
    const res = await page.goto(path);
    expect(res!.headers()["content-security-policy"] ?? "", path).toMatch(/script-src 'self' 'nonce-[A-Za-z0-9+/=]+' 'strict-dynamic'/);
  }
  // Without a session the gate still runs: the app page redirects to /login.
  const signedOut = await browser.newContext({ baseURL });
  const visitor = await signedOut.newPage();
  await visitor.goto("/projects/abc.png");
  await expect(visitor).toHaveURL(/\/login$/);
  await signedOut.close();
});

test("whatever the proxy skips carries a CSP under which no script runs", async ({ request }) => {
  for (const path of ["/favicon.ico", "/next.svg", "/healthz"]) {
    const csp = (await request.get(path)).headers()["content-security-policy"] ?? "";
    expect(csp, path).toContain("script-src 'none'");
    expect(csp, path).toContain("frame-ancestors 'none'");
  }
});

for (const path of ["/login", "/dashboard", "/install/linux", "/download"]) {
  test(`${path} renders without a single CSP violation`, async ({ page }) => {
    const violations: string[] = [];
    await page.exposeFunction("__cspViolation", (v: string) => violations.push(v));
    await page.addInitScript(() => {
      document.addEventListener("securitypolicyviolation", (e) => {
        (window as unknown as { __cspViolation: (v: string) => void }).__cspViolation(`${e.violatedDirective} ${e.blockedURI}`);
      });
    });
    await page.goto(path);
    await expect(page.locator("h1, h2").first()).toBeVisible();
    expect(violations).toEqual([]);
  });
}

test("the runtime configuration is served with the page, not baked into the bundle", async ({ page }) => {
  await page.goto("/install");
  const config = await page.locator("#dosi-runtime-config").textContent();
  expect(JSON.parse(config ?? "{}")).toMatchObject({ demoMode: true, downloadBaseUrl: expect.stringMatching(/^https:\/\//) });
});

test("/healthz answers without a session", async ({ request }) => {
  const res = await request.get("/healthz");
  expect(res.status()).toBe(200);
  expect(await res.json()).toMatchObject({ status: "ok", demoMode: true });
});

test("a signed-in visitor's ?next= never leaves the site, however it is disguised", async ({ page, baseURL }) => {
  for (const next of ["%2F%5Cevil.example", "%2F%09%2Fevil.example", "%2F%252F%252Fevil.example", "https%3A%2F%2Fevil.example"]) {
    await page.goto(`/login?next=${next}`);
    expect(new URL(page.url()).origin).toBe(new URL(baseURL!).origin);
    await expect(page).toHaveURL(/\/dashboard$/);
  }
});
