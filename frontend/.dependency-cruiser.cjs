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
      name: "no-orphans",
      severity: "warn",
      comment: "Unreferenced modules are dead weight — delete or wire them up.",
      from: { orphan: true, pathNot: ["\\.(test|spec)\\.tsx?$", "\\.d\\.ts$", "^src/test", "^src/middleware\\.ts$", "^src/app"] },
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
