"use client";

import { createContext, useCallback, useContext, useEffect, useMemo, useSyncExternalStore } from "react";
import { readStorage, writeStorage } from "@/lib/live-session";

type Theme = "light" | "dark";

interface ThemeContextValue {
  theme: Theme;
  toggle: () => void;
  setTheme: (t: Theme) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

/** Where the choice is kept (the pre-hydration script in app/layout reads it too). */
const THEME_STORAGE_KEY = "dosi-theme";
/** Raised when this tab changes the theme; other tabs hear the storage event. */
const THEME_EVENT = "dosi-theme-change";

/**
 * A choice the browser refused to store (site data blocked): it still holds
 * for this page view. Storage goes through lib/live-session's wrappers, which
 * never throw, so blocked storage can't take the app down with it.
 */
let unsaved: Theme | null = null;

const isTheme = (value: unknown): value is Theme => value === "light" || value === "dark";

/** The theme in force: this page's unsaved choice, the stored one, then the OS colour scheme. */
function currentTheme(): Theme {
  if (unsaved) return unsaved;
  const stored = readStorage(THEME_STORAGE_KEY);
  if (isTheme(stored)) return stored;
  try {
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  } catch {
    return "light";
  }
}

function subscribe(onChange: () => void): () => void {
  const onStorage = (event: StorageEvent) => {
    if (event.key === null || event.key === THEME_STORAGE_KEY) onChange();
  };
  window.addEventListener("storage", onStorage);
  window.addEventListener(THEME_EVENT, onChange);
  return () => {
    window.removeEventListener("storage", onStorage);
    window.removeEventListener(THEME_EVENT, onChange);
  };
}

const serverTheme = (): Theme => "light";
const noSubscription = () => () => {};

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  // The server renders the light theme; the browser reads the real one.
  const theme = useSyncExternalStore(subscribe, currentTheme, serverTheme);
  const hydrated = useSyncExternalStore(noSubscription, () => true, () => false);

  useEffect(() => {
    // The pre-hydration script has already applied the stored theme: the
    // hydration pass (still on the server's light theme) must not undo it.
    if (!hydrated) return;
    document.documentElement.classList.toggle("dark", theme === "dark");
    writeStorage(THEME_STORAGE_KEY, theme);
  }, [theme, hydrated]);

  const setTheme = useCallback((t: Theme) => {
    unsaved = writeStorage(THEME_STORAGE_KEY, t) ? null : t;
    window.dispatchEvent(new Event(THEME_EVENT));
  }, []);
  const toggle = useCallback(() => setTheme(currentTheme() === "dark" ? "light" : "dark"), [setTheme]);
  const value = useMemo(() => ({ theme, toggle, setTheme }), [theme, toggle, setTheme]);

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme() {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useTheme must be used within ThemeProvider");
  return ctx;
}
