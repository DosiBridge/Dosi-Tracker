"use client";

import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { usePrefersReducedMotion } from "@/hooks/use-pointer-parallax";

/** Fade/slide in when scrolled into view (or immediately if reduced motion). */
export function Reveal({
  children,
  className,
  delay = 0,
}: {
  children: React.ReactNode;
  className?: string;
  delay?: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const reduced = usePrefersReducedMotion();
  const [seen, setSeen] = useState(false);
  // With reduced motion the content is simply there — no entrance to wait for.
  const shown = reduced || seen;

  useEffect(() => {
    if (reduced) return;
    const el = ref.current;
    if (!el) return;
    const io = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setSeen(true);
          io.disconnect();
        }
      },
      { threshold: 0.08, rootMargin: "0px 0px -24px 0px" }
    );
    io.observe(el);
    return () => io.disconnect();
  }, [reduced]);

  return (
    <div
      ref={ref}
      className={cn("reveal", shown && "reveal-in", className)}
      style={{ transitionDelay: shown ? `${delay}ms` : undefined }}
    >
      {children}
    </div>
  );
}

/** Stagger children with CSS custom property --stagger-base */
export function Stagger({
  children,
  className,
  baseDelay = 40,
}: {
  children: React.ReactNode;
  className?: string;
  baseDelay?: number;
}) {
  return (
    <div className={cn("stagger", className)} style={{ ["--stagger-base" as string]: `${baseDelay}ms` }}>
      {children}
    </div>
  );
}

/** Remount-friendly page entrance for route / workspace changes. */
export function PageEnter({
  children,
  className,
  motionKey,
}: {
  children: React.ReactNode;
  className?: string;
  motionKey: string;
}) {
  return (
    <div key={motionKey} className={cn("page-enter", className)}>
      {children}
    </div>
  );
}
