// ThemeToggle drives the real ThemeProvider: toggling must flip the document's
// dark class and persist dosi-theme; on mount the stored preference (or the OS
// color scheme) must win.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { screen, fireEvent } from "@testing-library/react";
import { ThemeToggle } from "@/components/ui/theme-toggle";
import { renderWithProviders, resetPrototypeState } from "@/test/harness";

beforeEach(() => {
  resetPrototypeState();
  document.documentElement.classList.remove("dark");
});

afterEach(() => {
  vi.restoreAllMocks();
});

const isDark = () => document.documentElement.classList.contains("dark");

describe("ThemeToggle", () => {
  it("toggles dark mode on and off, flipping the document class and persisting dosi-theme", () => {
    renderWithProviders(<ThemeToggle />);
    expect(isDark()).toBe(false); // matchMedia stub reports light OS preference

    const toggle = screen.getByRole("button", { name: "Toggle theme" });
    fireEvent.click(toggle);
    expect(isDark()).toBe(true);
    expect(localStorage.getItem("dosi-theme")).toBe("dark");

    fireEvent.click(toggle);
    expect(isDark()).toBe(false);
    expect(localStorage.getItem("dosi-theme")).toBe("light");
  });

  it("honors a stored dark preference on mount", () => {
    localStorage.setItem("dosi-theme", "dark");
    renderWithProviders(<ThemeToggle />);
    expect(isDark()).toBe(true);
    // The toggle announces the mode it would switch TO.
    expect(screen.getByText("Light mode")).toBeInTheDocument();
  });

  it("falls back to the OS color scheme when nothing is stored", () => {
    vi.spyOn(window, "matchMedia").mockImplementation(
      (query: string) =>
        ({
          matches: query.includes("prefers-color-scheme: dark"),
          media: query,
          onchange: null,
          addListener: () => {},
          removeListener: () => {},
          addEventListener: () => {},
          removeEventListener: () => {},
          dispatchEvent: () => false,
        }) as MediaQueryList,
    );
    renderWithProviders(<ThemeToggle />);
    expect(isDark()).toBe(true);
    expect(localStorage.getItem("dosi-theme")).toBe("dark");
  });
});
