import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";

// Test runner config. Coverage is scoped to the business-LOGIC layer (src/lib logic + the route
// middleware) — the static data blobs (*-data.ts) and UI components/pages are excluded here and
// covered by later phases (component + e2e). Thresholds ratchet up per docs/QUALITY.md; they are a
// floor set just below the measured baseline so the suite is green on day one.
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
      exclude: ["src/lib/**/*-data.ts", "src/lib/types.ts", "**/*.d.ts"],
      // Phase 1 floor for the logic layer — measured baseline is lines/statements 98%, functions
      // 100%, branches 87% (see docs/QUALITY.md §4). Floors sit just below to stay green while
      // forbidding regression. Raise, never lower. As components/pages enter `include` in later
      // phases, keep these honest by ratcheting against the new (larger) denominator.
      thresholds: {
        lines: 95,
        functions: 95,
        statements: 95,
        branches: 85,
      },
    },
  },
});
