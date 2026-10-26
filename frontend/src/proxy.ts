import { NextResponse } from "next/server";
import type { NextRequest } from "next/server";
import { apiOrigin, resolveRuntimeConfig } from "@/lib/runtime-config";
import { safeRedirectPath } from "@/lib/safe-redirect";
import { NONCE_HEADER, contentSecurityPolicy, createNonce } from "@/lib/security-headers";

/**
 * Pages emailed links open (account recovery and email confirmation): public,
 * and reachable with or without a session.
 */
const ACCOUNT_LINK_PAGES = new Set(["/forgot-password", "/reset-password", "/confirm-email"]);

/** Pages anyone may open without a session: invitation links, account links, the install guides, the health probe. */
function isPublic(pathname: string): boolean {
  return (
    pathname.startsWith("/invite/") ||
    ACCOUNT_LINK_PAGES.has(pathname) ||
    pathname === "/install" ||
    pathname.startsWith("/install/") ||
    pathname === "/healthz"
  );
}

/** Whether the browser reached this app over https (directly, or through a TLS-terminating proxy). */
function servedOverHttps(request: NextRequest): boolean {
  if (request.nextUrl.protocol === "https:") return true;
  const forwarded = request.headers?.get?.("x-forwarded-proto");
  return forwarded?.split(",")[0]?.trim().toLowerCase() === "https";
}

/**
 * Let the request through with this request's Content-Security-Policy: on the
 * request (Next.js reads the nonce from it and stamps it on every script it
 * renders), as `x-nonce` for the root layout, and on the response.
 */
function nextWithCsp(request: NextRequest): NextResponse {
  const nonce = createNonce();
  const csp = contentSecurityPolicy({
    nonce,
    apiOrigin: apiOrigin(resolveRuntimeConfig(process.env)),
    dev: process.env.NODE_ENV === "development",
    secure: servedOverHttps(request),
  });
  const requestHeaders = new Headers(request.headers);
  requestHeaders.set(NONCE_HEADER, nonce);
  requestHeaders.set("Content-Security-Policy", csp);
  const response = NextResponse.next({ request: { headers: requestHeaders } });
  response.headers.set("Content-Security-Policy", csp);
  return response;
}

/**
 * Server-side route gate (Next.js 16 renamed the `middleware` file convention
 * to `proxy`). `loginApi` mirrors a session flag into the `dosi-token` cookie
 * (the JWT itself stays in localStorage only); without it, every app route
 * redirects to /login. The flag only says "a session may exist": the browser
 * decides with the stored token (SessionGate), and a flag without one is a
 * signed-out visitor, never the demo (unless the deployment enables it).
 *
 * Every page response also gets its Content-Security-Policy here, since it
 * carries a per-request nonce and the runtime API origin.
 */
export function proxy(request: NextRequest) {
  const { pathname } = request.nextUrl;
  const hasSession = request.cookies.has("dosi-token");

  if (isPublic(pathname)) return nextWithCsp(request);

  if (pathname === "/login") {
    if (hasSession) {
      const next = safeRedirectPath(request.nextUrl.searchParams?.get("next"));
      return NextResponse.redirect(new URL(next ?? "/dashboard", request.url));
    }
    return nextWithCsp(request);
  }

  if (!hasSession) {
    const login = new URL("/login", request.url);
    return NextResponse.redirect(login);
  }

  return nextWithCsp(request);
}

export const config = {
  // Everything except Next's build output, the app icon, the files in public/ (by exact name) and
  // the health probe is gated. Never skip by file extension: every dynamic route accepts a segment
  // like `abc.png`, and that page would render with no Content-Security-Policy and no sign-in
  // check. A file added to public/ must be listed here (proxy.test.ts fails until it is).
  matcher: ["/((?!_next/static/|_next/image$|healthz$|favicon\\.ico$|(?:file|globe|next|vercel|window)\\.svg$).*)"],
};
