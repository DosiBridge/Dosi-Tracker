import type { NextConfig } from "next";
import { BASELINE_CSP_HEADER, HSTS_HEADER, STATIC_SECURITY_HEADERS } from "./src/lib/security-headers";

const dev = process.env.NODE_ENV === "development";

const nextConfig: NextConfig = {
  // A self-contained server (`node .next/standalone/server.js`, honouring PORT
  // and HOSTNAME) for the container image; see frontend/README.md.
  output: "standalone",
  // Don't advertise the framework (X-Powered-By: Next.js).
  poweredByHeader: false,
  // next/image isn't used: keep the optimizer (and its native image library) out of the attack surface.
  images: { unoptimized: true },
  experimental: {
    // Turbopack merges small chunks into each route's own (default threshold 50 000 bytes), so a
    // module many routes share — next/link, the page header, the buttons, the AI cards — is copied
    // into every one of them: ~440 KB of the bundle was such copies. Merging less keeps one shared
    // copy instead. Measured (2026-10, 21 routes): total client JS 2468 → 2330 KB, and every page
    // loads LESS JavaScript than before (dashboard 328 → 314 KB, reports 533 → 519 KB) in ~25
    // requests instead of ~13 (HTTP/2, immutable-cached). See scripts/bundle-budget.mjs.
    // Re-measured with the journey pages (2026-10, 24 routes): at 15 000 the copies grew back to
    // 2480 KB in all; at 6 000 the total is 2344 KB while each page loads the same JavaScript as at
    // 15 000 (dashboard 330 KB, reports 534 KB, settings 292 KB) in ~37 requests instead of ~28.
    turbopackChunking: { minChunkSize: 6000 },
  },
  async headers() {
    // A page's Content-Security-Policy is per request (nonce, runtime API origin) and set by
    // src/proxy.ts, replacing the baseline one here; the baseline (no scripts at all) stays on
    // whatever the proxy skips, so a page that ever slips past it is inert, not unprotected.
    return [
      {
        source: "/:path*",
        headers: [...STATIC_SECURITY_HEADERS, BASELINE_CSP_HEADER, ...(dev ? [] : [HSTS_HEADER])],
      },
    ];
  },
};

export default nextConfig;
