/** Architecture rules — enforced in CI via `npm run test:arch`.
 *
 * Layering (allowed direction of imports):
 *   app  →  components  →  hooks  →  lib
 * `src/lib` is the pure logic/data layer: it must stay renderable-agnostic and
 * importable from anywhere (including tests and future non-React consumers).
 * Test helpers must never leak into production modules.
 */
module.exports = {
  forbidden: [
    {
      name: "no-circular",
      severity: "error",
      comment: "Circular dependencies make state initialization order-dependent and refactors dangerous.",
      from: {},
      to: { circular: true },
    },
    {
      name: "lib-stays-pure",
      severity: "error",
      comment: "src/lib is the pure logic layer — it must not depend on React components, hooks, or pages.",
      from: { path: "^src/lib" },
      to: { path: "^src/(components|hooks|app)" },
    },
    {
      name: "components-not-into-pages",
      severity: "error",
      comment: "Reusable components must not import route modules (pages import components, never the reverse).",
      from: { path: "^src/components" },
      to: { path: "^src/app" },
    },
    {
      name: "hooks-not-into-ui",
      severity: "error",
      comment: "Hooks are logic-layer: no imports from components or pages.",
      from: { path: "^src/hooks" },
      to: { path: "^src/(components|app)" },
    },
    {
      name: "no-test-code-in-prod",
      severity: "error",
      comment: "Production modules must never import test harnesses, stubs, or factories.",
      from: { path: "^src/(app|components|hooks|lib)", pathNot: "\\.(test|spec)\\.tsx?$" },
      to: { path: "^src/test" },
    },
    {
      name: "demo-data-loaded-on-demand",
      severity: "error",
      comment:
        "The demo dataset (lib/demo-tenants and its fixtures in lib/mock-data) is only for deployments with demo mode on: " +
        "the app may reach it through a dynamic import() alone, so a live deployment never downloads or generates it. " +
        "Live code that needs the demo's frozen clock imports lib/demo-clock.",
      from: { path: "^src/(app|components|hooks|lib)", pathNot: ["\\.(test|spec)\\.tsx?$", "^src/lib/demo-tenants\\.ts$"] },
      to: { path: "^src/lib/(demo-tenants|mock-data)\\.ts$", dependencyTypesNot: ["dynamic-import"] },
    },
    {
      name: "demo-ai-loaded-on-demand",
      severity: "error",
      comment:
        "The demo's sample AI output (lib/ai-demo) is only for demo sessions: the app may reach it through a dynamic import() alone, " +
        "so a live deployment never downloads the generators (type-only imports are erased, so they are fine). " +
        "The demo's AI status, which every session may read, is lib/ai-demo-status.",
      from: { path: "^src/(app|components|hooks|lib)", pathNot: ["\\.(test|spec)\\.tsx?$"] },
      to: { path: "^src/lib/ai-demo\\.ts$", dependencyTypesNot: ["dynamic-import", "type-only"] },
    },
    {
      name: "no-orphans",
      severity: "warn",
      comment: "Unreferenced modules are dead weight — delete or wire them up.",
      from: { orphan: true, pathNot: ["\\.(test|spec)\\.tsx?$", "\\.d\\.ts$", "^src/test", "^src/proxy\\.ts$", "^src/app"] },
      to: {},
    },
  ],
  options: {
    doNotFollow: { path: "node_modules" },
    tsConfig: { fileName: "tsconfig.json" },
    tsPreCompilationDeps: true,
    exclude: { path: ["\\.next", "node_modules", "^e2e", "^scripts"] },
  },
};
