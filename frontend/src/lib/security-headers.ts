/* ============================================================================
 * SECURITY HEADERS — what every response tells the browser about this app.
 *
 * The session's access and refresh tokens live in localStorage, so a single
 * injected script could read them; the Content-Security-Policy is the main
 * defence. It is built per request by the proxy (src/proxy.ts) because it
 * carries a fresh nonce and the API origin, which is runtime configuration:
 *
 *   script-src   'nonce-…' 'strict-dynamic' — only scripts Next.js rendered
 *                with this request's nonce, and what they load, may run.
 *   connect-src  'self' plus the configured API origin, nothing else.
 *   style-src    'unsafe-inline' — React renders `style` attributes (charts,
 *                progress bars) into the server HTML, and nonces can't cover
 *                attributes. Styles can't read tokens; scripts are the risk.
 *   img-src      'self' data: blob: — screenshots arrive as blob: URLs.
 *   frame-ancestors 'none' — the destructive confirm flows can't be clickjacked.
 *
 * The static headers (nosniff, referrer and permissions policy, frame
 * denial, HSTS) are set for every route, assets included, in next.config.ts.
 * So is BASELINE_CSP, a policy that lets no script run at all: the proxy's
 * header replaces it on every response the proxy handles, so it only stays
 * on responses the proxy skipped (build output, the icon, public/ files, the
 * health probe) and makes any page that ever slips past the proxy inert
 * instead of unprotected.
 * ========================================================================== */

export interface CspOptions {
  /** This request's nonce (base64). */
  nonce: string;
  /** The API origin the app calls, or null when none is configured. */
  apiOrigin: string | null;
  /** `next dev`: React needs eval for its debugging stacks. */
  dev?: boolean;
  /** The page was served over https: ask the browser to upgrade any http sub-request. */
  secure?: boolean;
}

/** The Content-Security-Policy header value for one request. */
export function contentSecurityPolicy({ nonce, apiOrigin, dev = false, secure = false }: CspOptions): string {
  const connect = ["'self'", ...(apiOrigin ? [apiOrigin] : [])];
  const directives: [string, ...string[]][] = [
    ["default-src", "'self'"],
    ["script-src", "'self'", `'nonce-${nonce}'`, "'strict-dynamic'", ...(dev ? ["'unsafe-eval'"] : [])],
    ["style-src", "'self'", "'unsafe-inline'"],
    ["img-src", "'self'", "data:", "blob:"],
    ["font-src", "'self'", "data:"],
    ["connect-src", ...connect],
    ["media-src", "'self'", "blob:"],
    ["worker-src", "'self'", "blob:"],
    ["manifest-src", "'self'"],
    ["object-src", "'none'"],
    ["base-uri", "'self'"],
    ["form-action", "'self'"],
    ["frame-src", "'none'"],
    ["frame-ancestors", "'none'"],
  ];
  if (secure) directives.push(["upgrade-insecure-requests"]);
  return directives.map((d) => d.join(" ")).join("; ");
}

/** A fresh, unguessable nonce (128 random bits, base64). */
export function createNonce(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary);
}

/** The request header the proxy hands the nonce to the root layout in. */
export const NONCE_HEADER = "x-nonce";

/** Headers every response carries (next.config.ts `headers()`), independent of the request. */
export const STATIC_SECURITY_HEADERS: ReadonlyArray<{ key: string; value: string }> = [
  { key: "X-Content-Type-Options", value: "nosniff" },
  { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
  { key: "X-Frame-Options", value: "DENY" },
  { key: "Cross-Origin-Opener-Policy", value: "same-origin" },
  {
    key: "Permissions-Policy",
    value: "camera=(), microphone=(), geolocation=(), payment=(), usb=(), serial=(), hid=(), bluetooth=()",
  },
];

/**
 * The Content-Security-Policy of every response the proxy did not handle
 * (next.config.ts): nothing may run, load or frame it. Subresources ignore a
 * policy on their own response, so the scripts, styles and fonts under
 * /_next/static/ load as before; a document served with it is inert.
 */
export const BASELINE_CSP_HEADER = {
  key: "Content-Security-Policy",
  value: "default-src 'none'; script-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
} as const;

/**
 * HSTS for production servers (next.config.ts adds it outside `next dev`, so a
 * developer's https://localhost isn't pinned). Browsers ignore it on plain-http
 * responses, so it is harmless behind a TLS-terminating proxy too.
 */
export const HSTS_HEADER = { key: "Strict-Transport-Security", value: "max-age=63072000; includeSubDomains" } as const;
