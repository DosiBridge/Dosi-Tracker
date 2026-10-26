"use client";

import { useEffect, useRef } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { AlertTriangle, RotateCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { linkButtonClass } from "@/components/dashboard/link-button";
import { describeClientError, errorReference, reportClientError } from "@/lib/error-report";
import { cn } from "@/lib/utils";

/**
 * What an error boundary shows instead of a white screen: a branded card that
 * says what happened in plain words, a Retry that re-renders the segment, and
 * a way back. The page's surroundings (sidebar, header) stay up because each
 * segment has its own boundary. The error is reported once, without personal
 * data (lib/error-report); its message is never shown, only the server's
 * reference when there is one.
 */
export function ErrorFallback({
  error,
  retry,
  boundary,
  home = { href: "/dashboard", label: "Go to dashboard" },
  className,
}: {
  error: unknown;
  retry: () => void;
  /** Which boundary caught it, for the report. */
  boundary: string;
  home?: { href: string; label: string };
  className?: string;
}) {
  const pathname = usePathname() ?? "/";
  const reported = useRef<unknown>(null);
  const reference = errorReference(error);
  const headingRef = useRef<HTMLHeadingElement>(null);

  useEffect(() => {
    if (reported.current === error) return;
    reported.current = error;
    reportClientError(describeClientError(error, { pathname, boundary }));
  }, [error, pathname, boundary]);

  // Move focus to the explanation so keyboard and screen-reader users aren't left on a control that vanished.
  useEffect(() => {
    headingRef.current?.focus();
  }, []);

  return (
    <div role="alert" className={cn("flex min-h-[50vh] items-center justify-center p-6", className)}>
      <div className="w-full max-w-md rounded-2xl border border-border bg-card p-8 text-center card-elev">
        <div className="mx-auto flex h-14 w-14 items-center justify-center rounded-full bg-danger/10 text-danger">
          <AlertTriangle className="h-7 w-7" aria-hidden="true" />
        </div>
        <h2 ref={headingRef} tabIndex={-1} className="mt-5 font-display text-xl font-bold tracking-tight outline-none">
          Something went wrong
        </h2>
        <p className="mt-2 text-sm text-muted-foreground">
          This part of the page couldn&apos;t be shown. Your data is safe — try again, and if it keeps happening, let whoever runs this
          site know{reference ? " the reference below" : ""}.
        </p>
        {reference && (
          <p className="mt-3 text-xs text-muted-foreground">
            Reference: <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-foreground">{reference}</code>
          </p>
        )}
        <div className="mt-6 flex flex-wrap justify-center gap-2">
          <Button onClick={() => retry()}>
            <RotateCw className="h-4 w-4" aria-hidden="true" /> Try again
          </Button>
          <Link href={home.href} className={linkButtonClass("outline", "md")}>
            {home.label}
          </Link>
        </div>
      </div>
    </div>
  );
}
