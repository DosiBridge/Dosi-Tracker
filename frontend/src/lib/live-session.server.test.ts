// @vitest-environment node
// The session layer is imported by pages Next.js also renders on the server,
// where there is no window: storage reads nothing, writes are refused, it is
// never a live session (so the demo's fixed clock renders, identically on
// server and client), and the navigation helpers do nothing — none of it may
// throw during a server render.
import { describe, expect, it } from "vitest";
import {
  canRenewSession,
  currentWorkspaceId,
  isLiveSession,
  isStorageAvailable,
  readStorage,
  redirectToLogin,
  redirectToWorkspaceChooser,
  referenceNow,
  reloadPage,
  removeStorage,
  setCurrentWorkspace,
  switchToWorkspace,
  tokenExpiresAt,
  writeStorage,
} from "./live-session";
import { NOW } from "./mock-data";

const WS = "11111111-1111-1111-1111-111111111111";

describe("live-session on the server (no window)", () => {
  it("runs where there is no window at all", () => {
    expect(typeof window).toBe("undefined");
  });

  it("reads nothing, refuses to store, and is never a live session", () => {
    expect(readStorage("dosi-token")).toBeNull();
    expect(writeStorage("dosi-token", "jwt")).toBe(false);
    expect(() => removeStorage("dosi-token")).not.toThrow();
    expect(isStorageAvailable()).toBe(false);
    expect(isLiveSession()).toBe(false);
    expect(canRenewSession()).toBe(false);
    expect(tokenExpiresAt()).toBeNull();
    expect(setCurrentWorkspace(WS)).toBe(false);
    expect(currentWorkspaceId()).toBeNull();
  });

  it("pivots on the demo's fixed clock, so the server and the first client render agree", () => {
    expect(referenceNow().getTime()).toBe(new Date(NOW).getTime());
  });

  it("the navigation helpers do nothing instead of throwing", () => {
    expect(() => redirectToLogin()).not.toThrow();
    expect(() => redirectToWorkspaceChooser()).not.toThrow();
    expect(() => switchToWorkspace(WS)).not.toThrow();
    expect(() => reloadPage()).not.toThrow();
  });
});
