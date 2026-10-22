import { NextResponse } from "next/server";
import type { NextRequest } from "next/server";

/** Same-site paths only (`?next=` must never send anyone off-site). */
function safeNext(raw: string | null | undefined): string | null {
  if (!raw || !raw.startsWith("/") || raw.startsWith("//")) return null;
  return raw;
}

/**
 * Server-side route gate. `loginApi` mirrors a session flag into the
 * `dosi-token` cookie (the JWT itself stays in localStorage only); without it,
 * every app route redirects to /login. Invitation links open for everyone.
 */
export function middleware(request: NextRequest) {
  const { pathname } = request.nextUrl;
  const hasSession = request.cookies.has("dosi-token");

  if (pathname.startsWith("/invite/") || pathname.startsWith("/install")) return NextResponse.next();

  if (pathname === "/login") {
    if (hasSession) {
      const next = safeNext(request.nextUrl.searchParams?.get("next"));
      return NextResponse.redirect(new URL(next ?? "/dashboard", request.url));
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
