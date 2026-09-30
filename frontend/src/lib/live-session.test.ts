import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { agoLabel, isLiveSession, referenceNow, TOKEN_STORAGE_KEY } from "./live-session";
import { NOW } from "./mock-data";

beforeEach(() => localStorage.clear());
afterEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
});

describe("isLiveSession — the single live-mode probe", () => {
  it("is false without a token and true with one", () => {
    expect(isLiveSession()).toBe(false);
    localStorage.setItem(TOKEN_STORAGE_KEY, "jwt");
    expect(isLiveSession()).toBe(true);
  });

  it("treats an empty token as no session", () => {
    localStorage.setItem(TOKEN_STORAGE_KEY, "");
    expect(isLiveSession()).toBe(false);
  });

  it("falls back to demo when storage access throws (blocked storage)", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("SecurityError");
    });
    expect(isLiveSession()).toBe(false);
  });
});

describe("referenceNow — which clock a page pivots on", () => {
  it("is the frozen demo instant in demo mode (a copy, not the shared object)", () => {
    const now = referenceNow();
    expect(now.getTime()).toBe(NOW.getTime());
    expect(now).not.toBe(NOW);
  });

  it("is the real clock in a live session", () => {
    localStorage.setItem(TOKEN_STORAGE_KEY, "jwt");
    const before = Date.now();
    const now = referenceNow().getTime();
    expect(now).toBeGreaterThanOrEqual(before);
    expect(now).toBeLessThanOrEqual(Date.now());
  });
});

describe("agoLabel", () => {
  const now = new Date("2026-09-23T12:00:00.000Z");
  const ago = (ms: number) => new Date(now.getTime() - ms).toISOString();

  it.each([
    [0, "just now"],
    [30_000, "just now"],
    [5 * 60_000, "5m ago"],
    [59 * 60_000, "59m ago"],
    [3 * 3_600_000, "3h ago"],
    [2 * 86_400_000, "2d ago"],
  ])("%i ms before → %s", (ms, expected) => {
    expect(agoLabel(ago(ms), now)).toBe(expected);
  });

  it("reads a future instant as 'just now' instead of a negative age", () => {
    expect(agoLabel(new Date(now.getTime() + 10 * 60_000).toISOString(), now)).toBe("just now");
  });

  it("renders an unparseable timestamp as a dash, never NaN", () => {
    expect(agoLabel("not-a-date", now)).toBe("—");
  });

  it("defaults to the reference clock", () => {
    expect(agoLabel(new Date(NOW.getTime() - 2 * 60_000).toISOString())).toBe("2m ago");
  });
});
