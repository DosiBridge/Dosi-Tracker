"use client";

import { useEffect, useLayoutEffect, useRef, useState } from "react";

/** A layout effect in the browser (it runs before paint); a plain effect while rendering on the server. */
const useBeforePaint = typeof window === "undefined" ? useEffect : useLayoutEffect;

/**
 * The rendered width of an element, kept up to date with a ResizeObserver —
 * for SVG diagrams laid out in real pixels (so their text never scales). The
 * first measurement is taken before the browser paints, so a diagram never
 * flashes at the wrong size (and the page never jumps) on a narrow screen.
 * Until measured (and where layout can't be read) it reads `fallback`.
 */
export function useElementWidth<T extends HTMLElement>(fallback = 640): [React.RefObject<T | null>, number] {
  const ref = useRef<T | null>(null);
  const [width, setWidth] = useState(fallback);
  useBeforePaint(() => {
    const el = ref.current;
    if (!el) return;
    // Reading layout before paint is what a layout effect is for; the state follows the DOM, not the reverse.
    const measured = Math.round(el.getBoundingClientRect().width);
    if (measured > 0) setWidth(measured);
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width ?? 0;
      if (w > 0) setWidth(Math.round(w));
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  return [ref, width];
}
