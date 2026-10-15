"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { Loader2, MailCheck, Radar } from "lucide-react";
import { Button } from "@/components/ui/button";
import { linkButtonClass } from "@/components/dashboard/link-button";
import { ApiError, acceptInvitationApi, getApi, getInvitationApi, logoutApi } from "@/hooks/useApi";
import { isLiveSession, switchToWorkspace } from "@/lib/live-session";
import { LIVE_ENDPOINTS } from "@/lib/live-loader";
import type { AppConfiguration } from "@/lib/live-identity";
import type { InvitationInfoDto } from "@/lib/api-contract";
import { workspaceRoleLabel } from "@/lib/sign-in";

type Load =
  | { state: "loading" }
  | { state: "missing" }
  | { state: "failed" }
  | { state: "ready"; info: InvitationInfoDto; signedInEmail: string | null };

const sameEmail = (a: string | null | undefined, b: string | null | undefined) =>
  !!a && !!b && a.trim().toLowerCase() === b.trim().toLowerCase();

export function InviteView({ token }: { token: string }) {
  const [load, setLoad] = useState<Load>({ state: "loading" });
  const [accepting, setAccepting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    (async () => {
      let info: InvitationInfoDto;
      try {
        info = await getInvitationApi(token);
      } catch (err) {
        if (alive) setLoad({ state: err instanceof ApiError && err.status === 404 ? "missing" : "failed" });
        return;
      }
      let signedInEmail: string | null = null;
      if (isLiveSession()) {
        try {
          const config = (await getApi(LIVE_ENDPOINTS.configuration)) as AppConfiguration;
          signedInEmail = config.currentUser?.isAuthenticated ? (config.currentUser.email ?? null) : null;
        } catch {
          signedInEmail = null;
        }
      }
      if (alive) setLoad({ state: "ready", info, signedInEmail });
    })();
    return () => {
      alive = false;
    };
  }, [token]);

  async function accept() {
    setAccepting(true);
    setError(null);
    try {
      const result = await acceptInvitationApi(token);
      switchToWorkspace(result.workspaceId);
    } catch (err) {
      const code = err instanceof ApiError ? err.code : null;
      setError(
        code === "Tracker:InvitationExpired"
          ? "This invitation has expired. Ask the workspace owner for a new one."
          : code === "Tracker:InvitationEmailMismatch"
            ? "This invitation was sent to a different email address."
            : code === "Tracker:InvitationNotFound"
              ? "This invitation is no longer valid."
              : "We couldn't accept the invitation. Try again.",
      );
      setAccepting(false);
    }
  }

  function signOut() {
    logoutApi();
    window.location.assign(`/invite/${encodeURIComponent(token)}`);
  }

  const back = `/invite/${encodeURIComponent(token)}`;

  return (
    <div className="flex min-h-screen items-center justify-center p-6">
      <div className="page-enter w-full max-w-md py-10">
        <div className="mb-8 flex items-center gap-3">
          <div className="flex h-10 w-10 items-center justify-center rounded-xl brand-gradient text-white">
            <Radar className="h-6 w-6" />
          </div>
          <span className="font-display text-lg font-bold tracking-tight">Dosi-Tracker</span>
        </div>

        {load.state === "loading" ? (
          <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading invitation…
          </p>
        ) : load.state === "missing" ? (
          <Notice title="Invitation not found">This invitation link isn&apos;t valid. It may have been revoked — ask the workspace owner for a new one.</Notice>
        ) : load.state === "failed" ? (
          <Notice title="Couldn't load the invitation">Check your connection and reload the page.</Notice>
        ) : (
          <>
            <div className="flex h-11 w-11 items-center justify-center rounded-full bg-primary/10 text-primary-strong">
              <MailCheck className="h-5 w-5" aria-hidden="true" />
            </div>
            <h1 className="mt-4 font-display text-2xl font-bold tracking-tight">Join {load.info.workspaceName}</h1>
            <p className="mt-1 text-sm text-muted-foreground">
              You&apos;ve been invited as <span className="font-medium text-foreground">{workspaceRoleLabel(load.info.role)}</span> with{" "}
              <span className="font-medium text-foreground" data-testid="invite-email">{load.info.email}</span>.
            </p>

            {load.info.expired ? (
              <p role="alert" className="mt-6 rounded-lg bg-warning/15 px-3 py-2 text-sm text-warning-on-tint">
                This invitation has expired. Ask the workspace owner for a new one.
              </p>
            ) : load.signedInEmail && sameEmail(load.signedInEmail, load.info.email) ? (
              <div className="mt-6 space-y-3">
                <Button size="lg" className="w-full" onClick={() => void accept()} disabled={accepting}>
                  {accepting && <Loader2 className="h-4 w-4 animate-spin motion-reduce:animate-none" aria-hidden="true" />}
                  {accepting ? "Joining…" : "Accept invitation"}
                </Button>
              </div>
            ) : load.signedInEmail ? (
              <div className="mt-6 space-y-3">
                <p role="alert" className="rounded-lg bg-warning/15 px-3 py-2 text-sm text-warning-on-tint">
                  You&apos;re signed in as <span className="font-medium">{load.signedInEmail}</span>, but this invitation is for{" "}
                  <span className="font-medium">{load.info.email}</span>. Sign out, then sign in with that email to accept it.
                </p>
                <Button variant="outline" className="w-full" onClick={signOut}>
                  Sign out
                </Button>
              </div>
            ) : (
              <div className="mt-6 space-y-3">
                <Link
                  href={`/login?next=${encodeURIComponent(back)}&email=${encodeURIComponent(load.info.email)}`}
                  className={`${linkButtonClass("primary", "md")} w-full`}
                >
                  Sign in to accept
                </Link>
                <Link
                  href={`/login?mode=signup&invite=${encodeURIComponent(token)}&email=${encodeURIComponent(load.info.email)}`}
                  className={`${linkButtonClass("outline", "md")} w-full`}
                >
                  Create an account
                </Link>
              </div>
            )}
            {error && (
              <p role="alert" className="mt-4 rounded-lg bg-danger/10 px-3 py-2 text-sm font-medium text-danger-on-tint">
                {error}
              </p>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function Notice({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div role="alert">
      <h1 className="font-display text-2xl font-bold tracking-tight">{title}</h1>
      <p className="mt-2 text-sm text-muted-foreground">{children}</p>
    </div>
  );
}
