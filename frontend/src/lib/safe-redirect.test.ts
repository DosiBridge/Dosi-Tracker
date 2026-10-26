import { describe, expect, it } from "vitest";
import fc from "fast-check";
import { safeRedirectPath } from "./safe-redirect";

describe("safeRedirectPath", () => {
  it("keeps same-site paths with their query and fragment", () => {
    expect(safeRedirectPath("/invite/abc")).toBe("/invite/abc");
    expect(safeRedirectPath("/dashboard")).toBe("/dashboard");
    expect(safeRedirectPath("/reports/payroll?from=2026-07-01&to=2026-07-14#totals")).toBe(
      "/reports/payroll?from=2026-07-01&to=2026-07-14#totals",
    );
  });

  it("refuses nothing, empty and overly long values", () => {
    expect(safeRedirectPath(null)).toBeNull();
    expect(safeRedirectPath(undefined)).toBeNull();
    expect(safeRedirectPath("")).toBeNull();
    expect(safeRedirectPath(`/${"a".repeat(2048)}`)).toBeNull();
  });

  it("refuses absolute and protocol-relative URLs", () => {
    expect(safeRedirectPath("https://evil.example/")).toBeNull();
    expect(safeRedirectPath("//evil.example")).toBeNull();
    expect(safeRedirectPath("javascript:alert(1)")).toBeNull();
    expect(safeRedirectPath("dashboard")).toBeNull();
  });

  it("refuses the backslash tricks WHATWG parsing turns into another host", () => {
    // new URL("/\\evil.example", base) resolves to https://evil.example/
    expect(safeRedirectPath("/\\evil.example")).toBeNull();
    expect(safeRedirectPath("\\\\evil.example")).toBeNull();
    expect(safeRedirectPath("/\\/evil.example")).toBeNull();
  });

  it("refuses tabs, newlines and other control characters (stripped by URL parsing)", () => {
    expect(safeRedirectPath("/\t/evil.example")).toBeNull();
    expect(safeRedirectPath("/\n/evil.example")).toBeNull();
    expect(safeRedirectPath("/\r\n/evil.example")).toBeNull();
    expect(safeRedirectPath("/\u0000/evil.example")).toBeNull();
    expect(safeRedirectPath("/dashboard\u007f")).toBeNull();
  });

  it("refuses encoded slashes and backslashes that decode to a second leading slash", () => {
    // What a router or proxy that decodes once more would see.
    expect(safeRedirectPath("/%2Fevil.example")).toBeNull();
    expect(safeRedirectPath("/%2F%2Fevil.example")).toBeNull();
    expect(safeRedirectPath("/%5Cevil.example")).toBeNull();
    expect(safeRedirectPath("/%5c%5cevil.example")).toBeNull();
    expect(safeRedirectPath("/%09/evil.example")).toBeNull();
    expect(safeRedirectPath("/%0A/evil.example")).toBeNull();
  });

  it("refuses malformed percent-encoding instead of guessing", () => {
    expect(safeRedirectPath("/%E0%A4%A")).toBeNull();
  });

  it("never returns to the sign-in page itself (a loop)", () => {
    expect(safeRedirectPath("/login")).toBeNull();
    expect(safeRedirectPath("/login?next=/dashboard")).toBeNull();
    expect(safeRedirectPath("/login/")).toBeNull();
    // A different page that merely starts with the same letters is fine.
    expect(safeRedirectPath("/loginhelp")).toBe("/loginhelp");
  });

  it("normalises dot segments without leaving the site", () => {
    expect(safeRedirectPath("/projects/../dashboard")).toBe("/dashboard");
    expect(safeRedirectPath("/../../evil.example")).toBe("/evil.example");
  });

  it("whatever it accepts resolves to this origin in a real browser-style parse", () => {
    const origin = "https://tracker.example.com";
    fc.assert(
      fc.property(fc.string({ maxLength: 40 }), (tail) => {
        for (const raw of [`/${tail}`, tail, `//${tail}`, `/\\${tail}`]) {
          const accepted = safeRedirectPath(raw);
          if (accepted === null) continue;
          expect(new URL(accepted, origin).origin).toBe(origin);
          expect(accepted.startsWith("/")).toBe(true);
          expect(accepted.startsWith("//")).toBe(false);
        }
      }),
    );
  });
});
