"use client";

import { useEffect, useRef, useState } from "react";
import { AppGlyph } from "@/components/screen-mock";
import { getAuthedBlobUrl } from "@/hooks/useApi";
import { createLimiter } from "@/lib/limiter";
import { cn } from "@/lib/utils";

/** Shared by every thumbnail on the page: at most this many image downloads in flight. */
const MAX_IN_FLIGHT = 4;
const downloads = createLimiter(MAX_IN_FLIGHT);

/** Start a download slightly before the tile scrolls into view. */
const PRELOAD_MARGIN = "200px";

/** The bearer-authed bytes of one stored capture. */
export const captureContentEndpoint = (id: string) => `/api/app/activity/screenshot/${encodeURIComponent(id)}/content`;

type ThumbState = { key: string; status: "ready"; url: string } | { key: string; status: "failed" };

/**
 * A REAL screenshot tile for a live activity. `captureIds` are tried in order
 * (the small "thumb" rendition first, then the full screen capture); with none,
 * or when every download fails, the tile falls back to the app glyph.
 *
 * Nothing is fetched until the tile nears the viewport, downloads share a
 * page-wide concurrency cap, and object URLs are revoked when the tile unmounts
 * or its captures change — a long list never holds every image in memory.
 */
export function CaptureThumb({
  captureIds,
  app,
  color,
  alt,
  className,
}: {
  captureIds: string[];
  /** the block's main app — the fallback glyph and the default alt text */
  app: string;
  color: string;
  alt?: string;
  /** sizing for the tile (applied to the glyph fallback too) */
  className?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  // Ids are GUIDs, so a joined key is a stable identity for the candidate list.
  const key = captureIds.join(",");
  const hasCaptures = key !== "";
  const [inView, setInView] = useState(false);
  const [state, setState] = useState<ThumbState | null>(null);

  // Lazy: flip `inView` once the tile nears the viewport, then stop observing.
  useEffect(() => {
    if (!hasCaptures || inView) return;
    const el = ref.current;
    if (!el) return;
    if (typeof IntersectionObserver === "undefined") {
      // No observer (very old browser): load now, but outside the effect body.
      const id = window.setTimeout(() => setInView(true), 0);
      return () => window.clearTimeout(id);
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          observer.disconnect();
          setInView(true);
        }
      },
      { rootMargin: PRELOAD_MARGIN },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [hasCaptures, inView]);

  useEffect(() => {
    if (!inView || !hasCaptures) return;
    const controller = new AbortController();
    let objectUrl: string | null = null;
    (async () => {
      for (const id of key.split(",")) {
        try {
          const url = await downloads.run(
            () => getAuthedBlobUrl(captureContentEndpoint(id), { signal: controller.signal }),
            controller.signal,
          );
          if (controller.signal.aborted) {
            URL.revokeObjectURL(url);
            return;
          }
          objectUrl = url;
          setState({ key, status: "ready", url });
          return;
        } catch {
          if (controller.signal.aborted) return;
          // This rendition is unreadable (e.g. no thumb was generated) — try the next one.
        }
      }
      setState({ key, status: "failed" });
    })();
    return () => {
      controller.abort();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [inView, hasCaptures, key]);

  const current = state?.key === key ? state : null;

  if (!hasCaptures || current?.status === "failed") {
    return <AppGlyph app={app} color={color} className={className} />;
  }

  return (
    <div ref={ref} className={cn("relative overflow-hidden bg-muted", className)}>
      {current?.status === "ready" ? (
        // A bearer-authed blob: next/image can't optimize it, so a plain <img> is right here.
        // eslint-disable-next-line @next/next/no-img-element
        <img
          src={current.url}
          alt={alt ?? `Screen capture — ${app}`}
          className="absolute inset-0 h-full w-full object-cover object-top"
          onError={() => setState({ key, status: "failed" })}
        />
      ) : (
        <div aria-hidden data-testid="capture-thumb-skeleton" className="absolute inset-0 bg-muted motion-safe:animate-pulse" />
      )}
    </div>
  );
}
