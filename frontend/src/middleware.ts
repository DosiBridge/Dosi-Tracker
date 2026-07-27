import { NextResponse } from "next/server";
import type { NextRequest } from "next/server";

/**
 * Server-side route gate. `loginApi` mirrors a session flag into the
 * `dosi-token` cookie (the JWT itself stays in localStorage only); without it,
 * every app route redirects to /login.
 */
export function middleware(request: NextRequest) {
  const { pathname } = request.nextUrl;
  const hasSession = request.cookies.has("dosi-token");

  if (pathname === "/login") {
    if (hasSession) {
      return NextResponse.redirect(new URL("/dashboard", request.url));
    }
    return NextResponse.next();
  }

  if (!hasSession) {
    const login = new URL("/login", request.url);
    return NextResponse.redirect(login);
  }

  return NextResponse.next();
}

export const config = {
  // Everything except static assets and Next internals is gated.
  matcher: ["/((?!_next/static|_next/image|favicon\\.ico|.*\\.(?:svg|png|jpg|jpeg|gif|webp|ico)$).*)"],
};
