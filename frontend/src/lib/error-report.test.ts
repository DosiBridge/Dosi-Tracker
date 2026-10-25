import { describe, expect, it, vi } from "vitest";
import { describeClientError, errorReference, reportClientError, routePattern } from "./error-report";

describe("routePattern", () => {
  it("replaces ids and tokens so no identifier leaves the page", () => {
    expect(routePattern("/projects/3f2b8c1e-1d2a-4b5c-9e8f-0a1b2c3d4e5f")).toBe("/projects/:id");
    expect(routePattern("/reports/payroll")).toBe("/reports/payroll");
    expect(routePattern("/activities/12345")).toBe("/activities/:id");
    expect(routePattern("/invite/AbCdEfGhIjKlMnOpQrStUvWxYz012345")).toBe("/invite/:id");
    expect(routePattern("/x/deadbeefdeadbeef00")).toBe("/x/:id");
  });

  it("drops the query string and fragment", () => {
    expect(routePattern("/login?next=/invite/secret&email=a@b.c#x")).toBe("/login");
    expect(routePattern("")).toBe("/");
  });
});

describe("describeClientError", () => {
  const now = new Date("2026-10-26T10:00:00.000Z");

  it("keeps the kind, the server's digest, a capped message and the route pattern", () => {
    const err = Object.assign(new TypeError("x".repeat(500)), { digest: "1234567" });
    const report = describeClientError(err, { pathname: "/projects/3f2b8c1e-1d2a-4b5c-9e8f-0a1b2c3d4e5f", boundary: "dashboard", now });
    expect(report).toEqual({
      kind: "TypeError",
      digest: "1234567",
      message: "x".repeat(200),
      route: "/projects/:id",
      boundary: "dashboard",
      at: "2026-10-26T10:00:00.000Z",
    });
  });

  it("copes with thrown non-errors and errors without a message or digest", () => {
    expect(describeClientError("boom", { pathname: "/", boundary: "global", now })).toMatchObject({ kind: "non-error", digest: null, message: null });
    const bare = new Error("");
    bare.name = "";
    expect(describeClientError(bare, { pathname: "/", boundary: "global", now })).toMatchObject({ kind: "Error", message: null });
    expect(describeClientError(Object.assign(new Error("e"), { digest: 42 }), { pathname: "/", boundary: "g" }).digest).toBeNull();
  });
});

describe("errorReference / reportClientError", () => {
  it("offers the digest as the reference, or nothing", () => {
    expect(errorReference(Object.assign(new Error("e"), { digest: "abc" }))).toBe("abc");
    expect(errorReference(new Error("e"))).toBeNull();
    expect(errorReference(undefined)).toBeNull();
  });

  it("logs the report once, tagged", () => {
    const log = vi.fn();
    const report = describeClientError(new Error("e"), { pathname: "/dashboard", boundary: "dashboard" });
    reportClientError(report, log);
    expect(log).toHaveBeenCalledWith("[dosi-tracker] render error", report);
  });

  it("logs to the console by default", () => {
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});
    reportClientError(describeClientError(new Error("e"), { pathname: "/", boundary: "g" }));
    expect(spy).toHaveBeenCalledTimes(1);
    spy.mockRestore();
  });
});
