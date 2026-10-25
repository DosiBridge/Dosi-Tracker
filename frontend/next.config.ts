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
