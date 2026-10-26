"use client";

import { Suspense, useEffect, useState } from "react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Check, KeyRound, Loader2 } from "lucide-react";
import { AuthError, AuthNotice, AuthShell } from "@/components/auth/auth-shell";
import { linkButtonClass } from "@/components/dashboard/link-button";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { completeRecoveryApi, logoutApi, verifyRecoveryLinkApi } from "@/hooks/useApi";
import {
  MIN_NEW_CREDENTIAL_LENGTH,
  NEW_CREDENTIAL_PROBLEM_TEXT,
  newCredentialProblem,
  parseRecoveryLink,
  recoveryFailure,
  type NewCredentialProblem,
} from "@/lib/account-recovery";

type Phase = "checking" | "ready" | "dead" | "done";

/**
 * /reset-password?userId=&resetToken=[&returnUrl=] — the page the recovery
 * email opens (backend WebAppLinks). It checks the link first, so an expired
 * one says so before anything is typed, then sets the new sign-in. Any session
 * open in this browser is ended: it may belong to someone else, and the
 * person signs in with the new one next.
 */
export default function ResetPage() {
  return (
    <Suspense fallback={null}>
      <ResetContent />
    </Suspense>
  );
}

function ResetContent() {
  const params = useSearchParams();
  const link = parseRecoveryLink(params);
  const [phase, setPhase] = useState<Phase>(link ? "checking" : "dead");
  const [value, setValue] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [problem, setProblem] = useState<NewCredentialProblem | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const userId = link?.userId ?? null;
  const token = link?.resetToken ?? null;

  useEffect(() => {
    if (!userId || !token) return;
    let alive = true;
    verifyRecoveryLinkApi(userId, token).then(
      (usable) => alive && setPhase(usable ? "ready" : "dead"),
      // The check itself failed (offline, an older server): let the reset decide.
      () => alive && setPhase("ready"),
    );
    return () => {
      alive = false;
    };
  }, [userId, token]);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!link) return;
    const found = newCredentialProblem(value, confirmation);
    setProblem(found);
    setError(null);
    if (found) return;
    setBusy(true);
    try {
      await completeRecoveryApi({ userId: link.userId, resetToken: link.resetToken, password: value });
      logoutApi();
      setPhase("done");
    } catch (err) {
      const failure = recoveryFailure(err, "Couldn't save it. Try again, or ask for a new link.");
      if (failure.linkDead) setPhase("dead");
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  const describedBy = problem || error ? "reset-error" : undefined;
  const signIn = link?.returnTo ? `/login?next=${encodeURIComponent(link.returnTo)}` : "/login";

  return (
    <AuthShell>
      <div className="flex h-11 w-11 items-center justify-center rounded-full bg-primary/10 text-primary-strong">
        <KeyRound aria-hidden className="h-5 w-5" />
      </div>
      <h1 className="mt-4 font-display text-2xl font-bold tracking-tight">Choose a new password</h1>

      {phase === "checking" && (
        <p role="status" className="mt-6 flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 aria-hidden className="h-4 w-4 animate-spin motion-reduce:animate-none" /> Checking your link…
        </p>
      )}

      {phase === "dead" && (
        <div className="mt-6 space-y-4">
          <AuthError>
            {error ?? "This link is incomplete, has expired or was already used. Links work once and only for a limited time."}
          </AuthError>
          <Link href="/forgot-password" className={`${linkButtonClass("primary", "md")} w-full`}>
            Ask for a new link
          </Link>
        </div>
      )}

      {phase === "done" && (
        <div className="mt-6 space-y-4">
          <AuthNotice>
            <span className="flex items-start gap-2">
              <Check aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-success-on-tint" />
              <span>Done. Sign in with your new password here and in the desktop agent.</span>
            </span>
          </AuthNotice>
          <Link href={signIn} className={`${linkButtonClass("primary", "md")} w-full`}>
            Continue to sign in
          </Link>
        </div>
      )}

      {phase === "ready" && (
        <form onSubmit={(e) => void submit(e)} className="mt-8 space-y-4" noValidate>
          <div className="space-y-1.5">
            <label htmlFor="reset-new" className="text-sm font-medium">
              New password
            </label>
            <Input
              id="reset-new"
              type="password"
              value={value}
              onChange={(e) => setValue(e.target.value)}
              autoComplete="new-password"
              required
              autoFocus
              aria-invalid={problem === "short" || problem === "long" || (problem === "missing" && !value) ? true : undefined}
              aria-describedby={describedBy ? `reset-rule ${describedBy}` : "reset-rule"}
            />
            <p id="reset-rule" className="text-xs text-muted-foreground">
              At least {MIN_NEW_CREDENTIAL_LENGTH} characters. The server may ask for more.
            </p>
          </div>
          <div className="space-y-1.5">
            <label htmlFor="reset-confirm" className="text-sm font-medium">
              Confirm new password
            </label>
            <Input
              id="reset-confirm"
              type="password"
              value={confirmation}
              onChange={(e) => setConfirmation(e.target.value)}
              autoComplete="new-password"
              required
              aria-invalid={problem === "mismatch" || (problem === "missing" && !confirmation) ? true : undefined}
              aria-describedby={describedBy}
            />
          </div>
          {(problem || error) && <AuthError id="reset-error">{problem ? NEW_CREDENTIAL_PROBLEM_TEXT[problem] : error}</AuthError>}
          <Button type="submit" size="lg" className="w-full" disabled={busy}>
            {busy && <Loader2 aria-hidden className="h-4 w-4 animate-spin motion-reduce:animate-none" />}
            {busy ? "Saving…" : "Save and continue"}
          </Button>
        </form>
      )}
    </AuthShell>
  );
}
