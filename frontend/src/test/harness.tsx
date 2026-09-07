// Reusable render harness for component/integration tests.
//
// The app's client state lives in three places that MUST be reset between
// tests or they leak: (1) localStorage (`dosi-*` keys), (2) the module-level
// tenant-data singleton (dataset cache + live bindings + in-place seed-array
// mutations), (3) the next/navigation stub. `resetPrototypeState()` handles
// all three and is safe to call in a global `beforeEach`.
//
// NOTE ON MOCKING next/navigation: components under test that import
// usePathname/useRouter need the stub active. Add this line at the top of the
// TEST FILE (vi.mock is hoisted, so it cannot live inside this module):
//
//   vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));
//
import { render, type RenderResult } from "@testing-library/react";
import { SessionProvider, hostUser } from "@/components/session-provider";
import { ThemeProvider } from "@/components/theme-provider";
import { ToastViewport } from "@/components/toast";
import { datasetFor, resetTenantDataForTests } from "@/lib/tenant-data";
import type { Role } from "@/lib/types";
import { __resetNavigation, __setPathname } from "@/test/next-navigation-stub";
import { resetFactories } from "@/test/factories";

/** Wipe every piece of cross-test state. Call in beforeEach (and afterAll). */
export function resetPrototypeState(): void {
  localStorage.clear();
  sessionStorage.clear();
  resetTenantDataForTests();
  __resetNavigation();
  resetFactories();
}

/**
 * Simulate a browser reload MID-TEST: drop every module-level cache (dataset
 * cache, seed-array mutations, live bindings) while KEEPING web storage —
 * exactly what survives a real reload. Unmount the tree first; the next render
 * rebuilds its datasets from the seeds plus whatever localStorage persisted.
 */
export function simulateReload(): void {
  resetTenantDataForTests();
  __resetNavigation();
}

export interface HarnessOptions {
  /** Role to sign in as; resolves to a seeded user of that role. Default: the seeded owner. */
  role?: Role;
  /** Explicit user id — wins over `role`. */
  userId?: string;
  /** Active workspace id (w1 = Dosi Labs, w2 = Acme Studio, w3 = Nimbus Co). Default w1. */
  workspaceId?: string;
  /** Pathname reported by the next/navigation stub (for RoleGuard etc.). */
  route?: string;
  /** Include the toast viewport so `toast()` calls are visible to queries. */
  withToasts?: boolean;
}

/** Find a seeded user of a given role in a workspace ("host" resolves to the platform admin). */
export function seededUserId(role: Role, workspaceId = "w1"): string {
  if (role === "host") return hostUser.id;
  const user = datasetFor(workspaceId).users.find((u) => u.role === role);
  if (!user) throw new Error(`No seeded ${role} user in workspace ${workspaceId} — build one with buildUser() and push it into datasetFor("${workspaceId}").users before rendering.`);
  return user.id;
}

/**
 * Render UI inside the real providers (ThemeProvider + SessionProvider).
 * Session identity/workspace are seeded through localStorage BEFORE render —
 * exactly the mechanism the app itself uses — so no provider internals are
 * mocked and tests exercise the true wiring.
 */
export function renderWithProviders(ui: React.ReactNode, options: HarnessOptions = {}): RenderResult {
  const { role, userId, workspaceId, route, withToasts } = options;

  if (route) __setPathname(route);
  if (workspaceId) localStorage.setItem("dosi-workspace", workspaceId);
  const resolvedUser = userId ?? (role ? seededUserId(role, workspaceId ?? "w1") : undefined);
  if (resolvedUser) localStorage.setItem("dosi-user", resolvedUser);

  return render(
    <ThemeProvider>
      <SessionProvider>
        {ui}
        {withToasts ? <ToastViewport /> : null}
      </SessionProvider>
    </ThemeProvider>,
  );
}

/** Sugar: render as a specific role (in the primary workspace unless told otherwise). */
export function renderAsRole(ui: React.ReactNode, role: Role, options: Omit<HarnessOptions, "role"> = {}): RenderResult {
  return renderWithProviders(ui, { ...options, role });
}

/** Sugar: render inside a specific workspace. */
export function renderForWorkspace(ui: React.ReactNode, workspaceId: string, options: Omit<HarnessOptions, "workspaceId"> = {}): RenderResult {
  return renderWithProviders(ui, { ...options, workspaceId });
}
