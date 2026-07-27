import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import nextTs from "eslint-config-next/typescript";

const eslintConfig = defineConfig([
  ...nextVitals,
  ...nextTs,
  // Override default ignores of eslint-config-next.
  globalIgnores([
    // Default ignores of eslint-config-next:
    ".next/**",
    "out/**",
    "build/**",
    "next-env.d.ts",
    "coverage/**",
  ]),

  // --- Grandfathered lint baseline (see docs/QUALITY.md §10 "Lint debt") ---
  // These rules are currently violated by PRE-EXISTING app code — chiefly data-fetch effects that
  // synchronously set state (react-hooks/set-state-in-effect). Refactoring untested effects is
  // deferred until component tests exist (Phase 3), so we do it safely rather than blind. Until
  // then they are WARNINGS, and `npm run lint` caps the total with `--max-warnings` so the count
  // can only DECREASE: any NEW violation breaks the build. Ratchet the cap down to 0, then delete
  // this block and let the rules return to their default `error` severity.
  {
    rules: {
      "react-hooks/set-state-in-effect": "warn",
      "react-hooks/exhaustive-deps": "warn",
      "react-hooks/immutability": "warn",
      // A handful of `any` in data-shaping code and unused symbols travel with the same legacy
      // files; capped alongside the effect debt so they too can only decrease.
      "@typescript-eslint/no-explicit-any": "warn",
      "@typescript-eslint/no-unused-vars": "warn",
    },
  },
]);

export default eslintConfig;
