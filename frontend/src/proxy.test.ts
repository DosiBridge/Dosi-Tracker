// @vitest-environment node
// The route gate runs on the server (Next's proxy, on the Node.js runtime), so we exercise it in a node environment.
import { afterEach, describe, it, expect, vi } from "vitest";
import { config, proxy } from "./proxy";
import type { NextRequest } from "next/server";

function req(pathname: string, hasToken: boolean, search = "", init: { protocol?: string; headers?: Record<string, string> } = {}): NextRequest {
  const protocol = init.protocol ?? "http:";
  return {
    nextUrl: { pathname, protocol, searchParams: new URLSearchParams(search) },
    url: `${protocol}//localhost${pathname}`,
    headers: new Headers(init.headers ?? {}),
    cookies: { has: (name: string) => name === "dosi-token" && hasToken },
  } as unknown as NextRequest;
}

const locationOf = (res: Response) => res.headers.get("location");
const cspOf = (res: Response) => res.headers.get("content-security-policy") ?? "";
/** The headers Next.js hands the page render (what the root layout reads the nonce from). */
const forwarded = (res: Response, name: string) => res.headers.get(`x-middleware-request-${name}`);

afterEach(() => {
  vi.unstubAllEnvs();
});

describe("proxy route gate", () => {
  it("redirects an unauthenticated request for an app route to /login", () => {
    const res = proxy(req("/dashboard", false));
    expect(res.status).toBe(307);
    expect(locationOf(res)).toMatch(/\/login$/);
  });

  it("lets an authenticated request through to an app route", () => {
    const res = proxy(req("/dashboard", true));
    expect(locationOf(res)).toBeNull(); // NextResponse.next() carries no redirect
  });

  it("bounces an already-authenticated user away from /login to /dashboard", () => {
    const res = proxy(req("/login", true));
    expect(res.status).toBe(307);
    expect(locationOf(res)).toMatch(/\/dashboard$/);
  });

  it("allows an unauthenticated user to reach /login", () => {
    const res = proxy(req("/login", false));
    expect(locationOf(res)).toBeNull();
  });

  it("lets anyone open an invitation link", () => {
    expect(locationOf(proxy(req("/invite/abc", false)))).toBeNull();
  });

  it("lets anyone open the installation and onboarding guides without signing in", () => {
    expect(locationOf(proxy(req("/install", false)))).toBeNull();
    expect(locationOf(proxy(req("/install/linux", false)))).toBeNull();
    expect(locationOf(proxy(req("/install/macos", false)))).toBeNull();
    expect(locationOf(proxy(req("/install/windows", false)))).toBeNull();
  });

  it("lets the health probe through without a session", () => {
    expect(locationOf(proxy(req("/healthz", false)))).toBeNull();
  });

  it("lets anyone open the account recovery and email confirmation pages, signed in or not (FE-09)", () => {
    for (const path of ["/forgot-password", "/reset-password", "/confirm-email"]) {
      expect(locationOf(proxy(req(path, false, "userId=x&token=y")))).toBeNull();
      expect(locationOf(proxy(req(path, true)))).toBeNull();
    }
  });

  it("does not treat look-alike paths as public", () => {
    expect(locationOf(proxy(req("/reset-password/extra", false)))).toMatch(/\/login$/);
    expect(locationOf(proxy(req("/installer", false)))).toMatch(/\/login$/);
    expect(locationOf(proxy(req("/invite", false)))).toMatch(/\/login$/);
    expect(locationOf(proxy(req("/healthz/extra", false)))).toMatch(/\/login$/);
  });

  it("sends a signed-in user from /login to a same-site ?next=", () => {
    expect(locationOf(proxy(req("/login", true, "next=/invite/abc")))).toMatch(/\/invite\/abc$/);
    expect(locationOf(proxy(req("/login", true, "next=//evil.example")))).toMatch(/\/dashboard$/);
  });

  it("never follows ?next= off-site, however it is disguised", () => {
    const attacks = [
      "next=%2F%5Cevil.example", // /\evil.example
      "next=%2F%09%2Fevil.example", // /<TAB>/evil.example
      "next=%2F%0A%2Fevil.example", // /<LF>/evil.example
      "next=%2F%252F%252Fevil.example", // decodes once more to ///evil.example
      "next=%2F%255Cevil.example", // decodes once more to /\evil.example
      "next=https%3A%2F%2Fevil.example",
      "next=%5C%5Cevil.example",
    ];
    for (const search of attacks) {
      const location = locationOf(proxy(req("/login", true, search)));
      expect(new URL(location!).host, search).toBe("localhost");
      expect(location, search).toMatch(/\/dashboard$/);
    }
  });

  it("keeps a safe ?next='s query string", () => {
    expect(locationOf(proxy(req("/login", true, "next=%2Freports%2Fpayroll%3Ffrom%3D2026-07-01")))).toMatch(
      /\/reports\/payroll\?from=2026-07-01$/,
    );
  });
});

describe("proxy security headers", () => {
  it("gives every page it lets through a nonce-based CSP, also forwarded to the render", () => {
    const res = proxy(req("/dashboard", true));
    const csp = cspOf(res);
    const nonce = /'nonce-([^']+)'/.exec(csp)?.[1];
    expect(nonce).toBeTruthy();
    expect(csp).toContain("'strict-dynamic'");
    expect(csp).toContain("frame-ancestors 'none'");
    expect(forwarded(res, "x-nonce")).toBe(nonce);
    expect(forwarded(res, "content-security-policy")).toBe(csp);
  });

  it("uses a fresh nonce for every request", () => {
    const a = forwarded(proxy(req("/login", false)), "x-nonce");
    const b = forwarded(proxy(req("/login", false)), "x-nonce");
    expect(a).not.toBe(b);
  });

  it("allows connections only to itself and the configured API origin", () => {
    vi.stubEnv("DOSI_API_URL", "https://api.example.com/tracker/");
    const csp = cspOf(proxy(req("/install", false)));
    expect(csp).toMatch(/connect-src 'self' https:\/\/api\.example\.com(;|$)/);
  });

  it("allows only itself when production has no API configured", () => {
    vi.stubEnv("NODE_ENV", "production");
    vi.stubEnv("DOSI_API_URL", "");
    vi.stubEnv("NEXT_PUBLIC_API_URL", "");
    vi.stubEnv("DOSI_DEMO_MODE", "");
    expect(cspOf(proxy(req("/login", false)))).toMatch(/connect-src 'self'(;|$)/);
  });

  it("allows eval only under next dev", () => {
    vi.stubEnv("NODE_ENV", "development");
    expect(cspOf(proxy(req("/login", false)))).toContain("'unsafe-eval'");
    vi.stubEnv("NODE_ENV", "production");
    expect(cspOf(proxy(req("/login", false)))).not.toContain("'unsafe-eval'");
  });

  it("asks for upgraded sub-requests only when the page itself came over https", () => {
    expect(cspOf(proxy(req("/login", false)))).not.toContain("upgrade-insecure-requests");
    expect(cspOf(proxy(req("/login", false, "", { protocol: "https:" })))).toContain("upgrade-insecure-requests");
    expect(cspOf(proxy(req("/login", false, "", { headers: { "x-forwarded-proto": "https, http" } })))).toContain(
      "upgrade-insecure-requests",
    );
  });

  it("adds no CSP to redirects (nothing renders)", () => {
    expect(cspOf(proxy(req("/dashboard", false)))).toBe("");
  });
});

describe("proxy matcher", () => {
  const matcher = new RegExp(`^${config.matcher[0]}$`);

  it("runs on pages, not on static assets, Next internals or the health probe", () => {
    for (const path of ["/", "/dashboard", "/login", "/install/linux", "/invite/abc", "/healthz/extra"]) {
      expect(matcher.test(path), path).toBe(true);
    }
    for (const path of ["/_next/static/chunks/a.js", "/_next/image", "/favicon.ico", "/logo.svg", "/healthz"]) {
      expect(matcher.test(path), path).toBe(false);
    }
  });
});
