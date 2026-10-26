import type { NextConfig } from "next";
import { HSTS_HEADER, STATIC_SECURITY_HEADERS } from "./src/lib/security-headers";

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
    turbopackChunking: { minChunkSize: 15000 },
  },
  async headers() {
    // The Content-Security-Policy is per request (nonce, runtime API origin) and set by src/proxy.ts.
    return [
      {
        source: "/:path*",
        headers: [...STATIC_SECURITY_HEADERS, ...(dev ? [] : [HSTS_HEADER])],
      },
    ];
  },
};

export default nextConfig;
