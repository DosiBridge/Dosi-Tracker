// THE CORRUPTED-STORAGE GAUNTLET.
//
// localStorage is user-editable, shared with other tabs and older app builds —
// the app must NEVER crash because a `dosi-*` key contains garbage. Every key
// the app reads is seeded with hostile values BEFORE the providers mount, and
// the contract is always the same: the tree renders without throwing and falls
// back to the defaults (owner Ayesha Rahman in the primary workspace Dosi Labs).
//
// These tests were written against the unguarded implementation first; they
// exposed real crashes (spreading a parsed non-array in session-provider.tsx,
// `.length` on parsed null in tenant-data.ts) which were then fixed with
// minimal Array.isArray-style guards. They stay as regression tests.
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { act, screen } from "@testing-library/react";
import { useSession } from "@/components/session-provider";
import { renderWithProviders, resetPrototypeState } from "@/test/harness";

function Probe() {
  const session = useSession();
  return (
    <div>
      <span data-testid="user-name">{session.user.name}</span>
      <span data-testid="workspace-name">{session.workspace.name}</span>
    </div>
  );
}

const HUGE = "x".repeat(100 * 1024);

/** Hostile payloads: raw garbage, wrong-type JSON, and a 100KB blob. */
const hostilePayloads: ReadonlyArray<readonly [string, string]> = [
  ["raw garbage ('{')", "{"],
  ["an object literal ('{}')", "{}"],
  ["an array literal ('[]')", "[]"],
  ["JSON null ('null')", "null"],
  ["JSON zero ('0')", "0"],
  ["a 100KB string", HUGE],
];

/** Every dosi-* key read by the app (dosi-projects-created-* is tested below). */
const storageKeys = [
  "dosi-user",
  "dosi-workspace",
  "dosi-workspaces-created",
  "dosi-workspaces-patches",
  "dosi-workspaces-deleted",
  "dosi-impersonating",
  "dosi-theme",
  "dosi-token",
  "dosi-tenant",
] as const;

const sessionCases = storageKeys.flatMap((key) =>
  hostilePayloads.map(([label, value]) => ({ key: key as string, label, value })),
);

const projectCases = hostilePayloads.map(([label, value]) => ({ label, value }));

beforeEach(() => {
  resetPrototypeState();
  // A hostile dosi-token makes the provider attempt backend hydration; no real
  // network is ever allowed, and a rejecting fetch is part of the gauntlet.
  vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new Error("network disabled in tests"))));
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("corrupted session storage never crashes the app", () => {
  it.each(sessionCases)(
    "falls back to the default owner and primary workspace when $key holds $label",
    async ({ key, value }) => {
      localStorage.setItem(key, value);

      expect(() => renderWithProviders(<Probe />)).not.toThrow();
      await act(async () => {});

      expect(screen.getByTestId("user-name")).toHaveTextContent("Ayesha Rahman");
      expect(screen.getByTestId("workspace-name")).toHaveTextContent("Dosi Labs");
    },
  );
});

describe("corrupted per-workspace project storage never crashes the app", () => {
  it.each(projectCases)(
    "renders workspace w2 with its seed roster when dosi-projects-created-w2 holds $label",
    async ({ value }) => {
      localStorage.setItem("dosi-projects-created-w2", value);

      expect(() => renderWithProviders(<Probe />, { workspaceId: "w2" })).not.toThrow();
      await act(async () => {});

      // The tenant still resolves; the stored default user is unknown in w2, so
      // the session lands on w2's first roster member (its owner).
      expect(screen.getByTestId("workspace-name")).toHaveTextContent("Acme Studio");
      expect(screen.getByTestId("user-name")).toHaveTextContent("Diego Alvarez");
    },
  );
});

describe("stale-but-well-formed session ids fall back safely", () => {
  it("an unknown dosi-user id falls back to a valid seeded user", () => {
    localStorage.setItem("dosi-user", "u-ghost-9999");

    renderWithProviders(<Probe />);

    expect(screen.getByTestId("user-name")).toHaveTextContent("Ayesha Rahman");
  });

  it("an unknown dosi-workspace id falls back to the primary workspace", () => {
    localStorage.setItem("dosi-workspace", "w-ghost-9999");

    renderWithProviders(<Probe />);

    expect(screen.getByTestId("workspace-name")).toHaveTextContent("Dosi Labs");
  });

  it("a dosi-workspace pointing at a deleted workspace falls back to the primary workspace", () => {
    localStorage.setItem("dosi-workspaces-deleted", JSON.stringify(["w2"]));
    localStorage.setItem("dosi-workspace", "w2");

    renderWithProviders(<Probe />);

    expect(screen.getByTestId("workspace-name")).toHaveTextContent("Dosi Labs");
  });
});
