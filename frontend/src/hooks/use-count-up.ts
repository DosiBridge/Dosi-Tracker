"use client";

import { useEffect, useRef, useState } from "react";
import { useMediaQuery } from "./use-media-query";

/** True when the viewer asked the OS for less motion. */
export function usePrefersReducedMotion(): boolean {
  return useMediaQuery("(prefers-reduced-motion: reduce)");
}

/**
 * A number that counts up to `target` (ease-out), from 0 on first show and
 * from the previous value when the target changes — for headline figures.
 * Viewers who prefer reduced motion get the target straight away. Pair the
 * animated number (aria-hidden) with the final value for assistive tech: the
 * in-between values are decoration, not information.
 */
export function useCountUp(target: number, durationMs = 900): number {
  const reduced = usePrefersReducedMotion();
  const [shown, setShown] = useState<number | null>(null);
  const from = useRef(0);
  const goal = Number.isFinite(target) ? target : 0;

  useEffect(() => {
    if (reduced || typeof requestAnimationFrame !== "function") return;
    const begin = from.current;
    const startedAt = performance.now();
    let frame = 0;
    const tick = (now: number) => {
      const t = Math.min(1, (now - startedAt) / Math.max(1, durationMs));
      const value = begin + (goal - begin) * (1 - (1 - t) ** 3);
      from.current = value;
      setShown(value);
      if (t < 1) frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [goal, durationMs, reduced]);

  if (reduced) return goal;
  return shown ?? 0;
}
