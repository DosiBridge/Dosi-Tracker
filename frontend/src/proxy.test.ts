// @vitest-environment node
// The route gate runs on the server (Next's proxy, on the Node.js runtime), so we exercise it in a node environment.
import { describe, it, expect } from "vitest";
import { proxy } from "./proxy";
import type { NextRequest } from "next/server";

function req(pathname: string, hasToken: boolean, search = ""): NextRequest {
  return {
    nextUrl: { pathname, searchParams: new URLSearchParams(search) },
    url: `http://localhost${pathname}`,
    cookies: { has: (name: string) => name === "dosi-token" && hasToken },
  } as unknown as NextRequest;
}

const locationOf = (res: Response) => res.headers.get("location");

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

  it("sends a signed-in user from /login to a same-site ?next=", () => {
    expect(locationOf(proxy(req("/login", true, "next=/invite/abc")))).toMatch(/\/invite\/abc$/);
    expect(locationOf(proxy(req("/login", true, "next=//evil.example")))).toMatch(/\/dashboard$/);
  });
});
