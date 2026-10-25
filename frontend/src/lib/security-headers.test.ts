import { describe, expect, it } from "vitest";
import { HSTS_HEADER, STATIC_SECURITY_HEADERS, contentSecurityPolicy, createNonce } from "./security-headers";

const directives = (csp: string) => new Map(csp.split("; ").map((d) => [d.split(" ")[0], d.split(" ").slice(1)]));

describe("contentSecurityPolicy", () => {
  it("only lets this request's scripts run, and never inline handlers or eval", () => {
    const csp = directives(contentSecurityPolicy({ nonce: "abc123==", apiOrigin: "https://api.example.com" }));
    expect(csp.get("script-src")).toEqual(["'self'", "'nonce-abc123=='", "'strict-dynamic'"]);
    expect(csp.get("script-src")).not.toContain("'unsafe-inline'");
    expect(csp.get("object-src")).toEqual(["'none'"]);
    expect(csp.get("base-uri")).toEqual(["'self'"]);
    expect(csp.get("form-action")).toEqual(["'self'"]);
  });

  it("can't be framed, and frames nothing itself", () => {
    const csp = directives(contentSecurityPolicy({ nonce: "n", apiOrigin: null }));
    expect(csp.get("frame-ancestors")).toEqual(["'none'"]);
    expect(csp.get("frame-src")).toEqual(["'none'"]);
  });

  it("connects only to itself and the API origin", () => {
    expect(directives(contentSecurityPolicy({ nonce: "n", apiOrigin: "https://api.example.com" })).get("connect-src")).toEqual([
      "'self'",
      "https://api.example.com",
    ]);
    expect(directives(contentSecurityPolicy({ nonce: "n", apiOrigin: null })).get("connect-src")).toEqual(["'self'"]);
  });

  it("allows the blob: screenshots and data: images the app renders", () => {
    expect(directives(contentSecurityPolicy({ nonce: "n", apiOrigin: null })).get("img-src")).toEqual(["'self'", "data:", "blob:"]);
  });

  it("adds eval for next dev and upgrade-insecure-requests for https pages only", () => {
    const plain = contentSecurityPolicy({ nonce: "n", apiOrigin: null });
    expect(plain).not.toContain("'unsafe-eval'");
    expect(plain).not.toContain("upgrade-insecure-requests");
    expect(contentSecurityPolicy({ nonce: "n", apiOrigin: null, dev: true })).toContain("'unsafe-eval'");
    expect(contentSecurityPolicy({ nonce: "n", apiOrigin: null, secure: true }).endsWith("; upgrade-insecure-requests")).toBe(true);
  });
});

describe("createNonce", () => {
  it("is 128 random bits of base64, different every time", () => {
    const seen = new Set<string>();
    for (let i = 0; i < 50; i++) {
      const nonce = createNonce();
      expect(nonce).toMatch(/^[A-Za-z0-9+/]{22}==$/);
      expect(atob(nonce)).toHaveLength(16);
      seen.add(nonce);
    }
    expect(seen.size).toBe(50);
  });
});

describe("static security headers", () => {
  it("cover sniffing, referrers, framing, opener and powerful features", () => {
    const byKey = new Map(STATIC_SECURITY_HEADERS.map((h) => [h.key, h.value]));
    expect(byKey.get("X-Content-Type-Options")).toBe("nosniff");
    expect(byKey.get("Referrer-Policy")).toBe("strict-origin-when-cross-origin");
    expect(byKey.get("X-Frame-Options")).toBe("DENY");
    expect(byKey.get("Cross-Origin-Opener-Policy")).toBe("same-origin");
    expect(byKey.get("Permissions-Policy")).toMatch(/camera=\(\).*microphone=\(\).*geolocation=\(\)/);
    expect(HSTS_HEADER.value).toMatch(/^max-age=\d{8,}; includeSubDomains$/);
  });
});
