"use client";

import { Suspense, useState } from "react";
import { useSearchParams } from "next/navigation";
import { Loader2, MailCheck } from "lucide-react";
import { AuthError, AuthNotice, AuthShell } from "@/components/auth/auth-shell";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ApiError, sendRecoveryEmailApi } from "@/hooks/useApi";
import { looksLikeEmail, recoveryRequestedMessage } from "@/lib/account-recovery";
import { safeRedirectPath } from "@/lib/safe-redirect";
import { classifySignInError, signInFailureMessage } from "@/lib/sign-in";

/**
 * /forgot-password (FE-09): ask for an emailed link to choose a new sign-in.
 * Whatever the address, the page says the same once the request reached the
 * server — it never tells whether an account exists. Only a request that
 * could not be made at all (offline, rate-limited, no server) reads differently.
 */
export default function ForgotPage() {
  return (
    <Suspense fallback={null}>
      <ForgotContent />
    </Suspense>
  );
}

function ForgotContent() {
  const params = useSearchParams();
  const [email, setEmail] = useState(() => params.get("email") ?? "");
  const [busy, setBusy] = useState(false);
  const [sentTo, setSentTo] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const returnTo = safeRedirectPath(params.get("next"));

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    const address = email.trim();
    if (!looksLikeEmail(address)) {
      setError("Enter the email address you sign in with.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await sendRecoveryEmailApi(address, returnTo);
      setSentTo(address);
    } catch (err) {
      if (err instanceof ApiError && err.status === 429) {
        setError("Too many requests from this network. Wait a minute, then try again.");
      } else if (err instanceof ApiError && err.status < 500) {
        // Any other answer from the server reads the same as success: nothing about the account leaks.
        setSentTo(address);
      } else {
        const failure = classifySignInError(err);
        setError(
          failure.kind === "unreachable" || failure.kind === "misconfigured"
            ? signInFailureMessage(failure)
            : "The server couldn't send the email right now. Try again in a few minutes.",
        );
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <AuthShell back={{ href: email ? `/login?email=${encodeURIComponent(email.trim())}` : "/login", label: "Back to sign in" }}>
      <div className="flex h-11 w-11 items-center justify-center rounded-full bg-primary/10 text-primary-strong">
        <MailCheck aria-hidden className="h-5 w-5" />
      </div>
      <h1 className="mt-4 font-display text-2xl font-bold tracking-tight">Forgot your password?</h1>
      {sentTo ? (
        <div className="mt-6 space-y-4">
          <AuthNotice>{recoveryRequestedMessage(sentTo)}</AuthNotice>
          <Button
            variant="outline"
            className="w-full"
            onClick={() => {
              setSentTo(null);
              setError(null);
            }}
          >
            Use another address
          </Button>
        </div>
      ) : (
        <>
          <p className="mt-1 text-sm text-muted-foreground">
            Enter the email you sign in with. If it belongs to an account here, we&apos;ll email it a link to choose a new password.
          </p>
          <form onSubmit={(e) => void submit(e)} className="mt-8 space-y-4" noValidate>
            <div className="space-y-1.5">
              <label htmlFor="forgot-email" className="text-sm font-medium">
                Email
              </label>
              <Input
                id="forgot-email"
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                autoComplete="username"
                required
                autoFocus
                aria-invalid={error ? true : undefined}
                aria-describedby={error ? "forgot-error" : undefined}
              />
            </div>
            {error && <AuthError id="forgot-error">{error}</AuthError>}
            <Button type="submit" size="lg" className="w-full" disabled={busy}>
              {busy && <Loader2 aria-hidden className="h-4 w-4 animate-spin motion-reduce:animate-none" />}
              {busy ? "Sending…" : "Email me a link"}
            </Button>
          </form>
          <p className="mt-6 text-xs text-muted-foreground">
            The desktop agent uses the same sign-in: after you choose a new one, sign in to it again.
          </p>
        </>
      )}
    </AuthShell>
  );
}
