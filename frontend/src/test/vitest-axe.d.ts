// Teach vitest's expect about the jest-axe matcher registered in setup.ts.
import "vitest";

declare module "vitest" {
  interface Assertion<T> {
    toHaveNoViolations(): T;
  }
  interface AsymmetricMatchersContaining {
    toHaveNoViolations(): void;
  }
}
