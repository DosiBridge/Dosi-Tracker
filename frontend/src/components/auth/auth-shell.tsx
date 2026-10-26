"use client";

import Link from "next/link";
import { ArrowLeft, Radar } from "lucide-react";
import { ThemeToggle } from "@/components/ui/theme-toggle";

/**
 * The frame of the pages people open from an email or before they can sign in
 * (forgot / reset / confirm email): the brand, a way back to sign-in and the
 * theme switch. Works with or without a session.
 */
export function AuthShell({ children, back = { href: "/login", label: "Back to sign in" } }: { children: React.ReactNode; back?: { href: string; label: string } | null }) {
  return (
    <div className="relative flex min-h-screen items-start justify-center p-6 sm:items-center">
      <div className="absolute right-4 top-4">
        <ThemeToggle />
      </div>
      <main className="page-enter w-full max-w-sm py-10">
        <div className="mb-8 flex items-center gap-3">
          <div className="flex h-10 w-10 items-center justify-center rounded-xl brand-gradient text-white">
            <Radar aria-hidden className="h-6 w-6" />
          </div>
          <span className="font-display text-lg font-bold tracking-tight">Dosi-Tracker</span>
        </div>
        {back && (
          <Link
            href={back.href}
            className="mb-4 inline-flex items-center gap-1.5 rounded text-sm text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <ArrowLeft aria-hidden className="h-4 w-4" /> {back.label}
          </Link>
        )}
        {children}
      </main>
    </div>
  );
}

/** A problem on one of these pages, announced and readable at AA contrast on its tint. */
export function AuthError({ id, children }: { id?: string; children: React.ReactNode }) {
  return (
    <p id={id} role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-sm font-medium text-danger-on-tint">
      {children}
    </p>
  );
}

/** A neutral outcome (sent, saved), announced politely. */
export function AuthNotice({ children }: { children: React.ReactNode }) {
  return (
    <p role="status" className="rounded-lg border border-border bg-muted/50 px-3 py-3 text-sm text-foreground">
      {children}
    </p>
  );
}
