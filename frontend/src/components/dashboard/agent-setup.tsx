"use client";

import { useEffect, useId, useRef, useState, useSyncExternalStore } from "react";
import { Check, Copy } from "lucide-react";
import { Button } from "@/components/ui/button";
import { toast } from "@/components/toast";
import { cn } from "@/lib/utils";

/* ============================================================================
 * AGENT SETUP — what a person needs to connect a desktop agent: the server
 * address (the API this web app talks to — agents can't guess it), the
 * workspace name, their email and, after an invite or a reset, a one-time
 * password. Shared by the Download page, the invite result and the Team
 * page's password reset, so the instructions read the same everywhere.
 * ========================================================================== */

/** The address agents sign in to: the same API this web app uses (useApi's default for local development). */
export const AGENT_SERVER_URL = (process.env.NEXT_PUBLIC_API_URL || "https://localhost:6006").replace(/\/+$/, "");

/** Where the agents are published. */
export const AGENT_RELEASES_URL = "https://github.com/DosiBridge/Dosi-Tracker/releases/latest";

/** In-app and public installation & onboarding guide URLs. */
export const WINDOWS_GUIDE_URL = "/install/windows";
export const MACOS_GUIDE_URL = "/install/macos";
export const LINUX_GUIDE_URL = "/install/linux";
export const ALL_GUIDES_URL = "/install";

/** The visitor's desktop OS, for recommending an agent; "unknown" before hydration. */
export type VisitorOs = "windows" | "macos" | "linux" | "other" | "unknown";

/**
 * Which agent fits the browser's computer. iPads report a Mac platform but
 * accept touch, and phones run neither agent, so both read as "other".
 */
export function visitorOs(input: { platform?: string | null; userAgent?: string | null; maxTouchPoints?: number | null }): VisitorOs {
  const platform = (input.platform ?? "").toLowerCase();
  const agent = (input.userAgent ?? "").toLowerCase();
  if (/iphone|ipad|ipod|android/.test(agent)) return "other";
  if (platform.startsWith("win") || (!platform && /windows nt/.test(agent))) return "windows";
  if (platform.startsWith("mac") || (!platform && /mac os x|macintosh/.test(agent))) {
    return (input.maxTouchPoints ?? 0) > 1 ? "other" : "macos";
  }
  if (platform.startsWith("linux") || (!platform && /linux|x11/.test(agent))) return "linux";
  return platform || agent ? "other" : "unknown";
}

const subscribeNothing = () => () => {};

function browserOs(): VisitorOs {
  const nav = navigator as Navigator & { userAgentData?: { platform?: string } };
  return visitorOs({
    platform: nav.userAgentData?.platform || nav.platform,
    userAgent: nav.userAgent,
    maxTouchPoints: nav.maxTouchPoints,
  });
}

/** The visitor's OS ("unknown" while rendering on the server, then the real one). */
export function useVisitorOs(): VisitorOs {
  return useSyncExternalStore(subscribeNothing, browserOs, () => "unknown");
}

/** This web app's own address, for the "get the agent" link ("" while rendering on the server). */
function useAppOrigin(): string {
  return useSyncExternalStore(subscribeNothing, () => window.location.origin, () => "");
}

/**
 * Copies `value` (or what `value()` returns at the moment of the click). Its
 * accessible name is "Copy" followed by the element named by `labelledBy`
 * ("Copy Server address"); success is announced politely.
 */
export function CopyButton({
  value,
  labelledBy,
  className,
  size = "sm",
}: {
  value: string | (() => string);
  labelledBy?: string;
  className?: string;
  size?: "sm" | "md";
}) {
  const id = useId();
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  async function copy() {
    try {
      await navigator.clipboard.writeText(typeof value === "function" ? value() : value);
      setCopied(true);
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => setCopied(false), 1800);
    } catch {
      toast({ tone: "warning", title: "Couldn't copy", description: "Your browser blocked clipboard access. Select the text and copy it instead." });
    }
  }

  return (
    <>
      <Button
        id={id}
        type="button"
        size={size}
        variant="outline"
        className={cn("shrink-0", className)}
        onClick={() => void copy()}
        aria-labelledby={labelledBy ? `${id} ${labelledBy}` : undefined}
      >
        {copied ? <Check aria-hidden className="h-3.5 w-3.5 text-success" /> : <Copy aria-hidden className="h-3.5 w-3.5" />}
        {copied ? "Copied" : "Copy"}
      </Button>
      {/* Outside the button, so the announcement never becomes part of its name. */}
      <span role="status" aria-live="polite" className="sr-only">
        {copied ? "Copied to the clipboard" : ""}
      </span>
    </>
  );
}

/** One labelled value with its own copy button, inside a SignInDetails list. */
function DetailRow({ label, value, mono = true }: { label: React.ReactNode; value: string; mono?: boolean }) {
  const labelId = useId();
  return (
    <div data-copy-row className="flex flex-wrap items-center gap-x-3 gap-y-1.5 py-2.5 first:pt-0 last:pb-0">
      <dt id={labelId} className="w-full text-xs font-medium text-muted-foreground sm:w-36 sm:shrink-0">
        {label}
      </dt>
      <dd className="flex min-w-0 flex-1 items-center gap-2">
        <span data-copy-value className={cn("min-w-0 flex-1 break-all text-sm text-foreground", mono && "font-mono")}>
          {value}
        </span>
        <CopyButton value={value} labelledBy={labelId} />
      </dd>
    </div>
  );
}

/**
 * Everything a member types into the desktop agent's sign-in screen, each
 * value copyable, plus "Copy all" for pasting into a message. `secret` is the
 * one-time password after an invite or a reset; pass its label as JSX.
 */
export function SignInDetails({
  email,
  secret,
  heading = "Sign-in details",
  headingLevel = 3,
  agentLink = true,
  className,
  children,
}: {
  email?: string | null;
  secret?: { label: React.ReactNode; value: string } | null;
  heading?: React.ReactNode;
  /** The heading's level in the page outline: 3 inside a dialog (under its h2), 2 directly on a page. */
  headingLevel?: 2 | 3;
  /** List where to get the desktop agent (off on the Download page itself). */
  agentLink?: boolean;
  className?: string;
  /** A note under the list (how to share it, what happens next). */
  children?: React.ReactNode;
}) {
  const headingId = useId();
  const listRef = useRef<HTMLDListElement>(null);
  const origin = useAppOrigin();
  const Heading = headingLevel === 2 ? "h2" : "h3";

  /** "Copy all": exactly what the list shows, one "Label: value" line per row. */
  function allText(): string {
    const rows = listRef.current ? Array.from(listRef.current.querySelectorAll<HTMLElement>("[data-copy-row]")) : [];
    return rows
      .map((row) => {
        const label = row.querySelector("dt")?.textContent?.trim() ?? "";
        const value = row.querySelector("[data-copy-value]")?.textContent?.trim() ?? "";
        return `${label}: ${value}`;
      })
      .join("\n");
  }

  return (
    <section aria-labelledby={headingId} className={cn("rounded-2xl border border-border bg-card/80 p-4", className)}>
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <Heading id={headingId} className="font-display text-sm font-semibold tracking-tight">
          {heading}
        </Heading>
        <CopyButton value={allText} labelledBy={headingId} />
      </div>
      <dl ref={listRef} className="divide-y divide-border/70">
        <DetailRow label="Server address" value={AGENT_SERVER_URL} />
        {email ? <DetailRow label="Email" value={email} mono={false} /> : null}
        {secret ? <DetailRow label={secret.label} value={secret.value} /> : null}
        {agentLink && origin ? <DetailRow label="Desktop agent" value={`${origin}/download`} mono={false} /> : null}
      </dl>
      {children && <div className="mt-3 text-xs leading-relaxed text-muted-foreground">{children}</div>}
    </section>
  );
}
