"use client";

import { Radar, ShieldCheck, Camera, Gauge } from "lucide-react";
import { parallaxStyle, usePointerParallax } from "@/hooks/use-pointer-parallax";

const features = [
  { icon: Gauge, text: "Live productivity & focus signals" },
  { icon: Camera, text: "Optional captures — blur by default" },
  { icon: ShieldCheck, text: "Counts only — never keystroke content" },
];

/** Login left panel — pointer parallax + ambient drift (nested so transforms don't clash). */
export function LoginHero() {
  const { ref, offset, onMove, onLeave, reduced } = usePointerParallax(1);

  return (
    <div
      ref={ref}
      onMouseMove={onMove}
      onMouseLeave={onLeave}
      className="relative hidden overflow-hidden brand-gradient lg:block"
    >
      <div
        className="pointer-events-none absolute -left-24 -top-24 h-80 w-80 parallax-layer"
        style={reduced ? undefined : parallaxStyle(offset, 0.35, 28)}
      >
        <div className="h-full w-full rounded-full bg-teal-400/25 blur-3xl animate-aurora" />
      </div>
      <div
        className="pointer-events-none absolute -bottom-20 -right-16 h-96 w-96 parallax-layer"
        style={reduced ? undefined : parallaxStyle(offset, 0.55, 32)}
      >
        <div className="h-full w-full rounded-full bg-sky-500/20 blur-3xl animate-aurora" style={{ animationDelay: "2s" }} />
      </div>

      <div
        className="absolute inset-0 grid-bg opacity-40 parallax-layer"
        style={reduced ? undefined : parallaxStyle(offset, 0.15, 12)}
      />

      <div
        className="pointer-events-none absolute -right-16 bottom-24 parallax-layer"
        style={reduced ? undefined : parallaxStyle(offset, 0.7, 22)}
      >
        <div className="h-72 w-[28rem] rounded-3xl border border-white/10 bg-white/5 backdrop-blur-sm animate-float-slow" />
      </div>
      <div
        className="pointer-events-none absolute right-8 bottom-40 parallax-layer"
        style={reduced ? undefined : parallaxStyle(offset, 1, 26)}
      >
        <div className="h-44 w-72 rounded-2xl border border-white/10 bg-white/[0.07] animate-float" style={{ animationDelay: "1.2s" }} />
      </div>
      <div
        className="pointer-events-none absolute right-16 bottom-52 flex w-56 flex-col gap-2 parallax-layer"
        style={reduced ? undefined : parallaxStyle(offset, 1.15, 28)}
      >
        {[72, 48, 88, 36].map((w, i) => (
          <div key={i} className="h-2 rounded-full bg-white/15" style={{ width: `${w}%` }} />
        ))}
      </div>

      <div
        className="relative flex h-full flex-col justify-between p-12 text-white parallax-layer"
        style={reduced ? undefined : parallaxStyle(offset, 0.25, 10)}
      >
        <div className="flex items-center gap-3 animate-fade-in">
          <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-teal-500/25 ring-1 ring-white/20 backdrop-blur">
            <Radar className="h-6 w-6" />
          </div>
          <span className="font-display text-2xl font-bold tracking-tight">Dosi-Tracker</span>
        </div>

        <div className="max-w-md">
          <p className="animate-fade-in text-sm font-medium uppercase tracking-[0.18em] text-teal-200/80" style={{ animationDelay: "80ms" }}>
            Activity intelligence
          </p>
          <h1 className="font-display mt-3 animate-fade-in text-4xl font-bold leading-[1.15] tracking-tight" style={{ animationDelay: "140ms" }}>
            See how work actually happens
          </h1>
          <p className="mt-4 animate-fade-in text-base leading-relaxed text-white/75" style={{ animationDelay: "200ms" }}>
            Time, apps, and screen captures — scoped by role, with privacy built in from the start.
          </p>

          <div className="stagger mt-10 space-y-3" style={{ ["--stagger-base" as string]: "70ms" }}>
            {features.map(({ icon: Icon, text }) => (
              <div
                key={text}
                className="flex items-center gap-3 rounded-xl border border-white/10 bg-white/5 px-3 py-2.5 backdrop-blur-sm transition-colors hover:bg-white/10"
              >
                <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-teal-400/20 text-teal-100">
                  <Icon className="h-5 w-5" />
                </div>
                <span className="text-sm text-white/90">{text}</span>
              </div>
            ))}
          </div>
        </div>

        <p className="animate-fade-in text-xs text-white/45" style={{ animationDelay: "400ms" }}>
          © 2026 DosiBridge · Open Source
        </p>
      </div>
    </div>
  );
}
