// Global test setup: registers jest-dom matchers (toBeInTheDocument, etc.) and unmounts React
// trees after every test so component tests never leak DOM state into one another.
import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

afterEach(() => {
  cleanup();
});
