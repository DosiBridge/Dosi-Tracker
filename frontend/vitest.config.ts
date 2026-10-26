import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";

// Test runner config. Coverage is scoped to the business-LOGIC layer: src/lib logic + the route
// gate (src/proxy.ts, formerly middleware.ts). UI components and pages are deliberately NOT in the
// denominator — they are covered behaviorally by component/integration/e2e suites, and chasing a
// global percentage over JSX rewards volume over meaning.
//
// `saas-data.ts`, `tenant-data.ts` and `reports-data.ts` ARE included despite the *-data suffix:
// they hold real derivation logic (plan/usage/invoice math, tenant lifecycle + persistence, report
// aggregation), which the earlier blanket `*-data.ts` exclusion hid from this gate. Only the
// literal fixture blobs (mock-data, monitor-data, host-data) stay out.
//
// Thresholds are a ratcheting FLOOR set just below the measured baseline: raise, never lower.
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    css: false,
    // Vitest's 5s default has no headroom for the user-event-driven page tests:
    // a full page render plus a typing sequence in jsdom runs comfortably under
    // a second idle, but blows past 5s on a loaded machine (a CI runner, or a
    // laptop also running the mutation suite), producing failures that look
    // like flakes but are pure starvation. This is a time allowance, not a
    // relaxed assertion — every expectation still executes and must pass.
    testTimeout: 15_000,
    // The suite renders the seeded demo tenants (src/test/setup.ts installs them), so it
    // runs as a demo-mode deployment would. Tests of the live and signed-out paths
    // switch it off themselves (vi.stubEnv / a rendered runtime-config block).
    env: { DOSI_DEMO_MODE: "1" },
    coverage: {
      provider: "v8",
      reporter: ["text-summary", "cobertura", "html"],
      reportsDirectory: "./coverage",
      include: ["src/lib/**/*.ts", "src/proxy.ts"],
      exclude: [
        "src/lib/types.ts",
        "**/*.d.ts",
        // Static fixture blobs: literal arrays of demo rows, no branching logic.
        "src/lib/mock-data.ts",
        "src/lib/monitor-data.ts",
        "src/lib/host-data.ts",
      ],
      // Measured baseline on this denominator (2026-10, Vitest 4): lines 99.2%, statements 99.0%,
      // functions 98.7%, branches 97.7%. Vitest 4's v8 provider remaps through the AST (its only
      // mode), counting branches the way istanbul does — every `??`, `||`/`&&` operand, ternary and
      // default argument — so the same tests read lower than Vitest 3's count did (branches fell
      // to 92.6% on the upgrade; the *.boundaries.test.ts suites and the edge cases added beside
      // them brought it back). Ratcheted up after the metrics-consistency work; raise, never lower.
      thresholds: {
        lines: 95,
        functions: 88,
        statements: 95,
        branches: 95,
      },
    },
  },
});
