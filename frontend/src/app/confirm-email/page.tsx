"use client";

import { Suspense, useEffect, useRef, useState, useSyncExternalStore } from "react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { Loader2, MailCheck } from "lucide-react";
import { AuthError, AuthNotice, AuthShell } from "@/components/auth/auth-shell";
import { linkButtonClass } from "@/components/dashboard/link-button";
import { confirmEmailApi, isLiveSession } from "@/hooks/useApi";
import { parseConfirmationLink, recoveryFailure } from "@/lib/account-recovery";

const subscribeNothing = () => () => {};
const signedOutOnServer = () => false;

type Phase = { state: "working" } | { state: "done" } | { state: "failed"; message: string };

/**
 * /confirm-email?userId=&token= — the link the sign-up email (or "Resend
 * confirmation") sends. Confirming the address is what lets invitations sent
 * to it show up on the workspace chooser (they are matched by verified email
 * only, E2E-01). It confirms once, on open; no session is needed.
 */
export default function ConfirmEmailPage() {
  return (
    <Suspense fallback={null}>
      <ConfirmContent />
    </Suspense>
  );
}

function ConfirmContent() {
  const params = useSearchParams();
  const link = parseConfirmationLink(params);
  const [phase, setPhase] = useState<Phase>(
    link ? { state: "working" } : { state: "failed", message: "This confirmation link is incomplete. Open the newest email we sent you." },
  );
  const sent = useRef(false);
  const userId = link?.userId ?? null;
  const token = link?.token ?? null;

  useEffect(() => {
    // Once per page view: a re-render must not post the one-time token again.
    if (!userId || !token || sent.current) return;
    sent.current = true;
    confirmEmailApi({ userId, token }).then(
      () => setPhase({ state: "done" }),
      (err) => setPhase({ state: "failed", message: recoveryFailure(err, "Couldn't confirm your email. Try the link again in a moment.").message }),
    );
  }, [userId, token]);

  // Read after hydration: the server never knows about this browser's session.
  const signedIn = useSyncExternalStore(subscribeNothing, isLiveSession, signedOutOnServer);
  const next = signedIn ? "/workspaces" : "/login";

  return (
    <AuthShell back={signedIn ? { href: "/workspaces", label: "Back to your workspaces" } : undefined}>
      <div className="flex h-11 w-11 items-center justify-center rounded-full bg-primary/10 text-primary-strong">
        <MailCheck aria-hidden className="h-5 w-5" />
      </div>
      <h1 className="mt-4 font-display text-2xl font-bold tracking-tight">Confirm your email</h1>
      <div className="mt-6 space-y-4">
        {phase.state === "working" && (
          <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 aria-hidden className="h-4 w-4 animate-spin motion-reduce:animate-none" /> Confirming…
          </p>
        )}
        {phase.state === "done" && (
          <>
            <AuthNotice>Your email address is confirmed. Invitations sent to it now appear when you choose a workspace.</AuthNotice>
            <Link href={next} className={`${linkButtonClass("primary", "md")} w-full`}>
              {signedIn ? "See your workspaces" : "Continue to sign in"}
            </Link>
          </>
        )}
        {phase.state === "failed" && (
          <>
            <AuthError>{phase.message}</AuthError>
            <p className="text-sm text-muted-foreground">
              Signed in? Ask for a new link from <span className="font-medium text-foreground">Choose a workspace</span>.
            </p>
            <Link href={next} className={`${linkButtonClass("outline", "md")} w-full`}>
              {signedIn ? "Back to your workspaces" : "Go to sign in"}
            </Link>
          </>
        )}
      </div>
    </AuthShell>
  );
}
