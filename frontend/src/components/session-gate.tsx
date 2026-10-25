"use client";

import { useEffect, useId, useState } from "react";
import { createPortal } from "react-dom";
import { useRouter } from "next/navigation";
import { AlertTriangle, Clock, Loader2, LogIn, RotateCw } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useDialogFocus } from "@/components/ui/use-dialog-focus";
import { useSession } from "@/components/session-provider";
import { signInFailureMessage, type SignInFailure } from "@/lib/sign-in";

/**
 * Holds the app shell until the session is known, so a signed-in owner never
 * sees a frame of demo people while their real identity loads, and shows an
 * honest error (with retry) when the backend can't be reached, instead of
 * quietly falling back to demo data. After a sign-out it shows a neutral
 * screen until /login loads — never the demo workspace.
 *
 * While the shell is up it also looks after the sign-in itself: a few
 * minutes before a session that can't renew itself runs out it says so, and
 * when it has run out it asks for the password in place, so nothing typed on
 * the page is lost.
 *
 * `area` picks the home a mismatched role is sent to: a live platform admin
 * who lands on the tenant app goes to /host, and vice versa.
 */
export function SessionGate({ area, children }: { area: "tenant" | "host"; children: React.ReactNode }) {
  const router = useRouter();
  const { status, liveError, refreshSession, logout, user, isLive } = useSession();

  const wrongArea = isLive && status === "ready" && (area === "tenant" ? user.role === "host" : user.role !== "host");
  useEffect(() => {
    if (wrongArea) router.replace(area === "tenant" ? "/host" : "/dashboard");
  }, [wrongArea, area, router]);

  if (status === "signedout") {
    return (
      <div className="flex min-h-screen flex-col items-center justify-center gap-3 p-4" role="status" aria-live="polite">
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin motion-reduce:animate-none" aria-hidden="true" /> Signing you out…
        </div>
        <a href="/login" className="text-xs font-medium text-primary-strong hover:underline">
          Continue to sign in
        </a>
      </div>
    );
  }

  if (status === "unknown" || status === "loading" || wrongArea) {
    return (
      <div className="flex min-h-screen items-center justify-center p-4" role="status" aria-live="polite">
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading your workspace…
        </div>
      </div>
    );
  }

  if (status === "error") {
    return (
      <div className="flex min-h-screen items-center justify-center p-4">
        <Card className="flex max-w-md flex-col items-center gap-4 py-12 text-center">
          <div className="flex h-14 w-14 items-center justify-center rounded-full bg-danger/10 text-danger">
            <AlertTriangle className="h-7 w-7" aria-hidden="true" />
          </div>
          <div className="px-6">
            <h2 className="text-lg font-semibold">Can&apos;t reach Dosi-Tracker</h2>
            <p className="mt-1 text-sm text-muted-foreground">{liveError}</p>
          </div>
          <div className="flex gap-2">
            <Button onClick={() => void refreshSession()}>
              <RotateCw className="h-4 w-4" aria-hidden="true" /> Try again
            </Button>
            <Button variant="outline" onClick={logout}>
              Sign out
            </Button>
          </div>
        </Card>
      </div>
    );
  }

  return (
    <>
      {children}
      {isLive && <SessionExpiryGuard />}
    </>
  );
}

/** Slim, dismiss-free notice when some (not all) live data failed to load. */
export function LiveWarningBanner() {
  const { liveWarning, refreshSession } = useSession();
  if (!liveWarning) return null;
  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-warning/30 bg-warning/10 px-4 py-2 text-sm lg:px-6" role="status">
      <AlertTriangle className="h-4 w-4 shrink-0 text-warning-on-tint" aria-hidden="true" />
      <span className="min-w-0 flex-1">{liveWarning}</span>
      <button
        type="button"
        onClick={() => void refreshSession({ silent: true })}
        className="rounded text-xs font-medium text-primary-strong hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        Retry
      </button>
    </div>
  );
}

/* ----------------------------- Session expiry ----------------------------- */

const timeFormat = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });

/**
 * The "your session is ending" notice and the in-place sign-in. A session
 * with a refresh token renews itself and never shows either; one without
 * gets a heads-up a few minutes ahead, and the dialog opens by itself when
 * the session has run out (the page stays underneath, as it was).
 */
function SessionExpiryGuard() {
  const { expiry } = useSession();
  const [requested, setRequested] = useState(false);
  const expired = expiry.phase === "expired";
  const open = expired || requested;

  return (
    <>
      {expiry.phase === "expiring" && !requested && (
        <div className="pointer-events-none fixed inset-x-0 bottom-0 z-40 flex justify-center p-3 sm:p-4">
          <div
            role="status"
            className="pointer-events-auto flex w-full max-w-xl flex-wrap items-center gap-3 rounded-2xl border border-warning/40 bg-card px-4 py-3 text-sm card-elev-lg animate-fade-in"
          >
            <Clock className="h-4 w-4 shrink-0 text-warning-on-tint" aria-hidden="true" />
            <span className="min-w-0 flex-1">
              Your session ends{expiry.expiresAt ? ` at ${timeFormat.format(expiry.expiresAt)}` : " soon"}. Sign in again to keep
              working — this page stays as it is.
            </span>
            <Button size="sm" onClick={() => setRequested(true)}>
              <LogIn className="h-3.5 w-3.5" aria-hidden="true" /> Sign in again
            </Button>
          </div>
        </div>
      )}
      {open && <ReauthDialog expired={expired} onDone={() => setRequested(false)} />}
    </>
  );
}

/** Sign in again without leaving the page: same account, same workspace, just the password. */
function ReauthDialog({ expired, onDone }: { expired: boolean; onDone: () => void }) {
  const { reauthIdentity, reauthenticate, logout } = useSession();
  const dialogRef = useDialogFocus<HTMLDivElement>(true);
  const titleId = useId();
  const descriptionId = useId();
  const passwordId = useId();
  const errorId = useId();
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<SignInFailure | null>(null);

  // Until it has run out, the dialog can be put away (Escape) and opened again from the notice.
  useEffect(() => {
    if (expired) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onDone();
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [expired, onDone]);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!password || busy) return;
    setBusy(true);
    setFailure(null);
    const result = await reauthenticate(password);
    setBusy(false);
    if (result.ok) {
      setPassword("");
      onDone();
    } else {
      setFailure(result.failure ?? { kind: "server" });
    }
  }

  return createPortal(
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div className="absolute inset-0 bg-black/50 backdrop-blur-sm animate-fade-in" aria-hidden="true" />
      <div
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={descriptionId}
        className="relative z-10 w-full max-w-sm rounded-2xl border border-border bg-card p-6 card-elev-lg animate-scale-in outline-none"
      >
        <div className="flex h-11 w-11 items-center justify-center rounded-full bg-primary/10 text-primary-strong">
          <LogIn className="h-5 w-5" aria-hidden="true" />
        </div>
        <h2 id={titleId} className="mt-4 text-base font-semibold">
          {expired ? "Your session has ended" : "Sign in again"}
        </h2>
        <p id={descriptionId} className="mt-1 text-sm text-muted-foreground">
          {expired
            ? "For your security, sign in again to keep working. This page stays exactly as you left it."
            : "Renew your session now so it doesn't end while you work. This page stays exactly as it is."}
        </p>

        {reauthIdentity && (
          <dl className="mt-4 space-y-1 rounded-xl border border-border bg-muted/40 px-3 py-2 text-sm">
            {reauthIdentity.workspace && (
              <div className="flex gap-2">
                <dt className="w-20 shrink-0 text-muted-foreground">Workspace</dt>
                <dd className="min-w-0 break-all font-medium">{reauthIdentity.workspace}</dd>
              </div>
            )}
            <div className="flex gap-2">
              <dt className="w-20 shrink-0 text-muted-foreground">Account</dt>
              <dd className="min-w-0 break-all font-medium">{reauthIdentity.username}</dd>
            </div>
          </dl>
        )}

        <form onSubmit={submit} className="mt-4 space-y-3">
          <div className="space-y-1.5">
            <label htmlFor={passwordId} className="text-sm font-medium">
              Password
            </label>
            <Input
              id={passwordId}
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="current-password"
              required
              aria-invalid={failure ? true : undefined}
              aria-describedby={failure ? errorId : undefined}
            />
          </div>
          {failure && (
            <p id={errorId} role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-sm text-danger-on-tint">
              {signInFailureMessage(failure)}
            </p>
          )}
          <div className="flex flex-wrap justify-end gap-2 pt-1">
            {expired ? (
              <Button type="button" variant="outline" onClick={logout}>
                Sign out
              </Button>
            ) : (
              <Button type="button" variant="ghost" onClick={onDone}>
                Not now
              </Button>
            )}
            <Button type="submit" disabled={busy || !password}>
              {busy && <Loader2 className="h-4 w-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />}
              {busy ? "Signing in…" : "Sign in"}
            </Button>
          </div>
        </form>
      </div>
    </div>,
    document.body,
  );
}
