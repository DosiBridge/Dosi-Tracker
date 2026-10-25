import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  REFRESH_TOKEN_STORAGE_KEY,
  WORKSPACE_STORAGE_KEY,
  currentWorkspaceId,
  isWorkspaceFreePath,
  setCurrentWorkspace,
  clearCurrentWorkspace,
  TOKEN_EXPIRES_STORAGE_KEY,
  TOKEN_STORAGE_KEY,
  agoLabel,
  canRenewSession,
  isLiveSession,
  isStorageAvailable,
  readStorage,
  referenceNow,
  reloadPage,
  removeStorage,
  tokenExpiresAt,
  writeStorage,
} from "./live-session";
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

/** Make every Storage method throw, as blocked site data / a full quota does. */
function blockStorage() {
  const denied = () => {
    throw new DOMException("The operation is insecure.", "SecurityError");
  };
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(denied);
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(denied);
  vi.spyOn(Storage.prototype, "removeItem").mockImplementation(denied);
}

describe("safe storage — blocked site data degrades, it never throws", () => {
  it("reads, writes and removes through to localStorage when it works", () => {
    expect(writeStorage("dosi-x", "1")).toBe(true);
    expect(readStorage("dosi-x")).toBe("1");
    removeStorage("dosi-x");
    expect(readStorage("dosi-x")).toBeNull();
    expect(isStorageAvailable()).toBe(true);
    expect(localStorage.getItem("dosi-storage-probe")).toBeNull(); // the probe leaves nothing behind
  });

  it("reports blocked storage instead of throwing (SecurityError on every call)", () => {
    blockStorage();
    expect(readStorage("dosi-x")).toBeNull();
    expect(writeStorage("dosi-x", "1")).toBe(false);
    expect(() => removeStorage("dosi-x")).not.toThrow();
    expect(isStorageAvailable()).toBe(false);
    expect(isLiveSession()).toBe(false);
    expect(tokenExpiresAt()).toBeNull();
    expect(canRenewSession()).toBe(false);
  });

  it("survives a browser that throws on merely touching window.localStorage", () => {
    vi.spyOn(window, "localStorage", "get").mockImplementation(() => {
      throw new DOMException("Access is denied for this document.", "SecurityError");
    });
    expect(readStorage(TOKEN_STORAGE_KEY)).toBeNull();
    expect(writeStorage("dosi-x", "1")).toBe(false);
    expect(() => removeStorage("dosi-x")).not.toThrow();
    expect(isStorageAvailable()).toBe(false);
  });

  it("a full quota (setItem throws, reads work) is not available for a session", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("Quota exceeded", "QuotaExceededError");
    });
    expect(writeStorage("dosi-x", "1")).toBe(false);
    expect(isStorageAvailable()).toBe(false);
  });
});

describe("session token facts", () => {
  it("tokenExpiresAt reads the stored expiry and ignores anything unusable", () => {
    expect(tokenExpiresAt()).toBeNull();
    localStorage.setItem(TOKEN_EXPIRES_STORAGE_KEY, "1790000000000");
    expect(tokenExpiresAt()).toBe(1790000000000);
    localStorage.setItem(TOKEN_EXPIRES_STORAGE_KEY, "soon");
    expect(tokenExpiresAt()).toBeNull();
    localStorage.setItem(TOKEN_EXPIRES_STORAGE_KEY, "0");
    expect(tokenExpiresAt()).toBeNull();
  });

  it("canRenewSession follows the refresh token", () => {
    expect(canRenewSession()).toBe(false);
    localStorage.setItem(REFRESH_TOKEN_STORAGE_KEY, "rt");
    expect(canRenewSession()).toBe(true);
  });

  it("currentWorkspaceId is the stored workspace GUID; demo ids and junk never count", () => {
    expect(currentWorkspaceId()).toBeNull();
    setCurrentWorkspace("3f2504e0-4f89-11d3-9a0c-0305e82c3301");
    expect(localStorage.getItem(WORKSPACE_STORAGE_KEY)).toBe("3f2504e0-4f89-11d3-9a0c-0305e82c3301");
    expect(currentWorkspaceId()).toBe("3f2504e0-4f89-11d3-9a0c-0305e82c3301");
    localStorage.setItem(WORKSPACE_STORAGE_KEY, "w1");
    expect(currentWorkspaceId()).toBeNull();
    setCurrentWorkspace("3f2504e0-4f89-11d3-9a0c-0305e82c3301");
    clearCurrentWorkspace();
    expect(currentWorkspaceId()).toBeNull();
  });

  it("knows the pages that need no workspace", () => {
    expect(isWorkspaceFreePath("/workspaces")).toBe(true);
    expect(isWorkspaceFreePath("/invite/abc")).toBe(true);
    expect(isWorkspaceFreePath("/login")).toBe(true);
    expect(isWorkspaceFreePath("/dashboard")).toBe(false);
    expect(isWorkspaceFreePath("/invitesomething")).toBe(false);
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

describe("reloadPage — another tab switched the browser to a different account", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("reloads the page", () => {
    const reload = vi.fn();
    vi.stubGlobal("location", { ...window.location, reload });
    reloadPage();
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it("keeps the current screen when the browser refuses to reload", () => {
    vi.stubGlobal("location", {
      ...window.location,
      reload: () => {
        throw new DOMException("Navigation blocked", "SecurityError");
      },
    });
    expect(() => reloadPage()).not.toThrow();
  });
});
