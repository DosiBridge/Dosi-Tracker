// ThemeToggle drives the real ThemeProvider: toggling must flip the document's
// dark class and persist dosi-theme; on mount the stored preference (or the OS
// color scheme) must win.
vi.mock("next/navigation", () => import("@/test/next-navigation-stub"));

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { act, screen, fireEvent } from "@testing-library/react";
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

  it("follows a theme chosen in another tab", () => {
    renderWithProviders(<ThemeToggle />);
    expect(isDark()).toBe(false);
    localStorage.setItem("dosi-theme", "dark");
    act(() => {
      window.dispatchEvent(new StorageEvent("storage", { key: "dosi-theme", newValue: "dark" }));
    });
    expect(isDark()).toBe(true);
    expect(screen.getByText("Light mode")).toBeInTheDocument();
  });

  // Last: the choice made while storage is blocked is held for the rest of this page view.
  it("keeps working when the browser blocks site data: nothing is stored, nothing crashes", () => {
    const blocked = () => {
      throw new DOMException("The operation is insecure.", "SecurityError");
    };
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(blocked);
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(blocked);
    renderWithProviders(<ThemeToggle />);
    expect(isDark()).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: "Toggle theme" }));
    expect(isDark()).toBe(true);
    expect(screen.getByText("Light mode")).toBeInTheDocument();
  });
});
