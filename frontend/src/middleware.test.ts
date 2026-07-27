// @vitest-environment node
// The route gate runs on the server/edge, so we exercise it in a node environment.
import { describe, it, expect } from "vitest";
import { middleware } from "./middleware";
import type { NextRequest } from "next/server";

function req(pathname: string, hasToken: boolean): NextRequest {
  return {
    nextUrl: { pathname },
    url: `http://localhost${pathname}`,
    cookies: { has: (name: string) => name === "dosi-token" && hasToken },
  } as unknown as NextRequest;
}

const locationOf = (res: Response) => res.headers.get("location");

describe("middleware route gate", () => {
  it("redirects an unauthenticated request for an app route to /login", () => {
    const res = middleware(req("/dashboard", false));
    expect(res.status).toBe(307);
    expect(locationOf(res)).toMatch(/\/login$/);
  });

  it("lets an authenticated request through to an app route", () => {
    const res = middleware(req("/dashboard", true));
    expect(locationOf(res)).toBeNull(); // NextResponse.next() carries no redirect
  });

  it("bounces an already-authenticated user away from /login to /dashboard", () => {
    const res = middleware(req("/login", true));
    expect(res.status).toBe(307);
    expect(locationOf(res)).toMatch(/\/dashboard$/);
  });

  it("allows an unauthenticated user to reach /login", () => {
    const res = middleware(req("/login", false));
    expect(locationOf(res)).toBeNull();
  });
});
