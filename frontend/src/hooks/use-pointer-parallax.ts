"use client";

import { useCallback, useEffect, useRef, useState } from "react";

export type ParallaxOffset = { x: number; y: number };

export function usePrefersReducedMotion(): boolean {
  const [reduced, setReduced] = useState(false);
  useEffect(() => {
    const mql = window.matchMedia("(prefers-reduced-motion: reduce)");
    const sync = () => setReduced(mql.matches);
    sync();
    mql.addEventListener("change", sync);
    return () => mql.removeEventListener("change", sync);
  }, []);
  return reduced;
}

/**
 * Pointer-driven parallax. Offsets are normalized ~[-1, 1].
 * Smoothly eases toward the pointer; returns to center on leave.
 */
export function usePointerParallax(strength = 1) {
  const ref = useRef<HTMLDivElement>(null);
  const [offset, setOffset] = useState<ParallaxOffset>({ x: 0, y: 0 });
  const reduced = usePrefersReducedMotion();
  const target = useRef<ParallaxOffset>({ x: 0, y: 0 });
  const current = useRef<ParallaxOffset>({ x: 0, y: 0 });
  const raf = useRef(0);
  const active = useRef(false);

  const tick = useCallback(() => {
    const c = current.current;
    const t = target.current;
    c.x += (t.x - c.x) * 0.08;
    c.y += (t.y - c.y) * 0.08;
    setOffset({ x: c.x, y: c.y });
    const settled = Math.abs(t.x - c.x) < 0.001 && Math.abs(t.y - c.y) < 0.001;
    if (!settled || active.current) {
      raf.current = requestAnimationFrame(tick);
    } else {
      raf.current = 0;
    }
  }, []);

  const startLoop = useCallback(() => {
    if (!raf.current) raf.current = requestAnimationFrame(tick);
  }, [tick]);

  const onMove = useCallback(
    (e: React.MouseEvent) => {
      if (reduced || !ref.current) return;
      active.current = true;
      const rect = ref.current.getBoundingClientRect();
      target.current = {
        x: (((e.clientX - rect.left) / rect.width) * 2 - 1) * strength,
        y: (((e.clientY - rect.top) / rect.height) * 2 - 1) * strength,
      };
      startLoop();
    },
    [reduced, strength, startLoop]
  );

  const onLeave = useCallback(() => {
    active.current = false;
    target.current = { x: 0, y: 0 };
    startLoop();
  }, [startLoop]);

  useEffect(() => () => cancelAnimationFrame(raf.current), []);

  return { ref, offset, onMove, onLeave, reduced };
}

export function parallaxStyle(
  offset: ParallaxOffset,
  depth: number,
  unit = 18
): React.CSSProperties {
  return {
    transform: `translate3d(${offset.x * depth * unit}px, ${offset.y * depth * unit}px, 0)`,
    willChange: "transform",
  };
}
