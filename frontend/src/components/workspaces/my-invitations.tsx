"use client";

import { useCallback, useEffect, useState } from "react";
import { Loader2, MailCheck, MailQuestion } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { toast } from "@/components/toast";
import { requestErrorText } from "@/components/layout/request-error";
import {
  ApiError,
  acceptMyInvitationApi,
  declineMyInvitationApi,
  getMyInvitationsApi,
  sendEmailConfirmationApi,
} from "@/hooks/useApi";
import type { MyInvitationDto, MyInvitationsDto } from "@/lib/api-contract";
import { workspaceRoleLabel } from "@/lib/sign-in";

function expiryLabel(iso: string): string | null {
  const at = Date.parse(iso);
  return Number.isFinite(at) ? new Date(at).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" }) : null;
}

/** Why accepting failed, in words (and whether the invitation is gone for good). */
function acceptFailure(err: unknown): { message: string; gone: boolean } {
  const code = err instanceof ApiError ? err.code : null;
  if (code === "Tracker:InvitationExpired" || code === "Tracker:InvitationNotFound") {
    return { message: "This invitation has expired or was withdrawn. Ask the workspace owner for a new one.", gone: true };
  }
  if (code === "Tracker:AlreadyWorkspaceMember") return { message: "You're already a member of this workspace.", gone: true };
  if (code === "Tracker:EmailNotConfirmed") {
    return { message: "Confirm your email address first, or open the invitation link you were sent.", gone: false };
  }
  if (code === "Tracker:WorkspaceArchived") return { message: "This workspace is archived, so it can't be joined right now.", gone: true };
  return { message: requestErrorText(err, "Couldn't answer the invitation. Try again."), gone: false };
}

/**
 * The invitations addressed to the signed-in person (GET /api/app/invitations/mine):
 * nobody is ever added to a workspace without saying yes here or on the
 * invitation link (E2E-01). They are matched by email, so they show only
 * once the address is confirmed; until then this explains why and offers the
 * confirmation email again.
 *
 * `onJoined` opens the workspace just joined; `onChanged` lets the page re-read
 * its list after an answer.
 */
export function MyInvitations({
  onJoined,
  onChanged,
}: {
  onJoined: (workspaceId: string) => void;
  onChanged?: () => void;
}) {
  const [state, setState] = useState<MyInvitationsDto | null | "loading" | "failed">("loading");
  const [busy, setBusy] = useState<string | null>(null);
  const [confirmSent, setConfirmSent] = useState<"sending" | "sent" | "no-email" | "confirmed" | null>(null);

  const load = useCallback(async () => {
    try {
      setState(await getMyInvitationsApi());
    } catch {
      setState("failed");
    }
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void load();
  }, [load]);

  const remove = (id: string) =>
    setState((s) => (s && typeof s === "object" ? { ...s, items: s.items.filter((i) => i.id !== id) } : s));

  async function accept(inv: MyInvitationDto) {
    if (busy) return;
    setBusy(inv.id);
    try {
      const joined = await acceptMyInvitationApi(inv.id);
      toast({ tone: "success", title: `Joined ${joined.workspaceName || inv.workspaceName}` });
      onJoined(joined.workspaceId || inv.workspaceId);
    } catch (err) {
      const failure = acceptFailure(err);
      if (failure.gone) remove(inv.id);
      toast({ tone: "danger", title: "Couldn't join the workspace", description: failure.message });
      onChanged?.();
    } finally {
      setBusy(null);
    }
  }

  async function decline(inv: MyInvitationDto) {
    if (busy) return;
    setBusy(inv.id);
    try {
      await declineMyInvitationApi(inv.id);
      remove(inv.id);
      toast({ title: "Invitation declined", description: `You won't join ${inv.workspaceName}.` });
    } catch (err) {
      const failure = acceptFailure(err);
      if (failure.gone) remove(inv.id);
      toast({ tone: "danger", title: "Couldn't decline the invitation", description: failure.message });
    } finally {
      setBusy(null);
    }
  }

  async function resendConfirmation() {
    setConfirmSent("sending");
    try {
      const result = await sendEmailConfirmationApi();
      if (result?.alreadyConfirmed) {
        setConfirmSent("confirmed");
        void load();
      } else setConfirmSent(result?.emailSent ? "sent" : "no-email");
    } catch (err) {
      setConfirmSent(null);
      toast({ tone: "danger", title: "Couldn't send the email", description: requestErrorText(err, "Try again in a minute.") });
    }
  }

  // An older server (null) or a list that failed to load stays out of the way: the workspace list still works.
  if (state === "loading" || state === null) return null;
  if (state === "failed") {
    return (
      <p role="status" className="mt-6 text-xs text-muted-foreground">
        Couldn&apos;t check for invitations.{" "}
        <button type="button" onClick={() => void load()} className="font-medium text-primary-strong underline underline-offset-2">
          Try again
        </button>
      </p>
    );
  }

  if (!state.emailConfirmed) {
    return (
      <section aria-labelledby="my-invitations-heading" className="mt-6 rounded-xl border border-border bg-muted/40 p-4">
        <h2 id="my-invitations-heading" className="flex items-center gap-2 text-sm font-semibold">
          <MailQuestion aria-hidden className="h-4 w-4 text-muted-foreground" /> Invitations for you
        </h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Confirm your email address to see invitations sent to it here. An invitation link you were sent works either way.
        </p>
        {confirmSent === "sent" ? (
          <p role="status" className="mt-3 text-sm text-foreground">
            We emailed you a confirmation link. Open it, then come back here.
          </p>
        ) : confirmSent === "no-email" ? (
          <p role="status" className="mt-3 text-sm text-foreground">
            This server doesn&apos;t send email. Open the invitation link you were given, or ask the workspace owner to share it.
          </p>
        ) : confirmSent === "confirmed" ? (
          <p role="status" className="mt-3 text-sm text-foreground">
            Your email is already confirmed.
          </p>
        ) : (
          <Button size="sm" variant="outline" className="mt-3" onClick={() => void resendConfirmation()} disabled={confirmSent === "sending"}>
            {confirmSent === "sending" && <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" />}
            Email me a confirmation link
          </Button>
        )}
      </section>
    );
  }

  if (state.items.length === 0) return null;
  return (
    <section aria-labelledby="my-invitations-heading" className="mt-6">
      <h2 id="my-invitations-heading" className="flex items-center gap-2 text-sm font-semibold">
        <MailCheck aria-hidden className="h-4 w-4 text-primary-strong" /> Invitations for you
      </h2>
      <p className="mt-0.5 text-xs text-muted-foreground">You join a workspace only when you accept its invitation.</p>
      <ul className="mt-3 space-y-2" aria-label="Invitations for you">
        {state.items.map((inv) => {
          const expires = expiryLabel(inv.expiresAt);
          return (
            <li key={inv.id} className="flex flex-wrap items-center gap-3 rounded-xl border border-primary/30 bg-primary/5 p-3">
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-semibold">{inv.workspaceName || "A workspace"}</span>
                <span className="block text-xs text-muted-foreground">
                  As {workspaceRoleLabel(inv.role)}
                  {expires ? ` · expires ${expires}` : ""}
                </span>
              </span>
              <Badge tone="info">Invited</Badge>
              <div className="flex gap-2">
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => void decline(inv)}
                  disabled={!!busy}
                  aria-label={`Decline the invitation to ${inv.workspaceName}`}
                >
                  Decline
                </Button>
                <Button size="sm" onClick={() => void accept(inv)} disabled={!!busy} aria-label={`Accept the invitation to ${inv.workspaceName}`}>
                  {busy === inv.id && <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" />}
                  Accept
                </Button>
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
