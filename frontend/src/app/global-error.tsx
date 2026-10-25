"use client";

import "./globals.css";
import { ErrorFallback } from "@/components/error-fallback";

/**
 * The last line of defence: a crash in the root layout itself (the providers
 * every page shares). It replaces the whole document, so it brings its own
 * <html>, <body> and stylesheet; the theme follows the operating system here.
 */
export default function GlobalError({ error, retry }: { error: unknown; retry: () => void }) {
  return (
    <html lang="en">
      <body className="min-h-screen bg-background font-sans text-foreground antialiased">
        <title>Something went wrong — Dosi-Tracker</title>
        <main className="flex min-h-screen items-center justify-center">
          <ErrorFallback error={error} retry={retry} boundary="global" home={{ href: "/", label: "Reload Dosi-Tracker" }} />
        </main>
      </body>
    </html>
  );
}
