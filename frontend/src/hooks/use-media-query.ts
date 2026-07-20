"use client";

import { useEffect, useState } from "react";

/** Client-side media query — SSR-safe (starts false, updates after mount). */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(false);

  useEffect(() => {
    const mql = window.matchMedia(query);
    const onChange = () => setMatches(mql.matches);
    onChange();
    mql.addEventListener("change", onChange);
    return () => mql.removeEventListener("change", onChange);
  }, [query]);

  return matches;
}

export function useIsMdUp(): boolean {
  return useMediaQuery("(min-width: 768px)");
}
