import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";

// Test runner config. Coverage is scoped to the business-LOGIC layer: src/lib logic + the route
// middleware. UI components and pages are deliberately NOT in the denominator — they are covered
// behaviorally by component/integration/e2e suites, and chasing a global percentage over JSX
// rewards volume over meaning.
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
    coverage: {
      provider: "v8",
      reporter: ["text-summary", "cobertura", "html"],
      reportsDirectory: "./coverage",
      include: ["src/lib/**/*.ts", "src/middleware.ts"],
      exclude: [
        "src/lib/types.ts",
        "**/*.d.ts",
        // Static fixture blobs: literal arrays of demo rows, no branching logic.
        "src/lib/mock-data.ts",
        "src/lib/monitor-data.ts",
        "src/lib/host-data.ts",
      ],
      // Measured baseline on this (widened) denominator: lines/statements 95.1%, functions 86.7%,
      // branches 92.8%. Floors sit just below so the gate is green today but any regression fails.
      thresholds: {
        lines: 94,
        functions: 85,
        statements: 94,
        branches: 91,
      },
    },
  },
});
