"use client";

import { useCallback, useEffect, useId, useState } from "react";
import Link from "next/link";
import { Loader2, Trash2, UserX } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Modal } from "@/components/ui/modal";
import { deleteMyAccountApi, getAccountDeletionCheckApi } from "@/hooks/useApi";
import type { AccountDeletionCheckDto } from "@/lib/api-contract";
import { erasureErrorText } from "@/lib/erasure";

/**
 * Delete your own account (E2E-07). The server first says whether it can be
 * done now: not while you are a workspace's only owner (those are listed, to
 * hand over or delete first), never for a platform administrator. Deleting
 * ends every session at once, removes you from every workspace, deletes your
 * captures and strips identifying details from your tracked time.
 */
export function DeleteAccountCard({ live, onDeleted }: { live: boolean; onDeleted: () => void }) {
  const id = useId();
  const [check, setCheck] = useState<AccountDeletionCheckDto | null | "loading" | "failed">(live ? "loading" : null);
  const [open, setOpen] = useState(false);
  const [current, setCurrent] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setCheck(await getAccountDeletionCheckApi());
    } catch {
      setCheck("failed");
    }
  }, []);

  useEffect(() => {
    if (!live) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void load();
  }, [live, load]);

  async function remove() {
    if (!current || busy) return;
    setBusy(true);
    setError(null);
    try {
      await deleteMyAccountApi(current);
      onDeleted();
    } catch (err) {
      setError(erasureErrorText(err, "Couldn't delete your account. Try again."));
      setBusy(false);
      // The blockers may have changed (an ownership handed over meanwhile): ask again.
      void load();
    }
  }

  const ready = check && typeof check === "object" ? check : null;

  return (
    <Card className="border-danger/30">
      <CardHeader>
        <div>
          <CardTitle>Delete your account</CardTitle>
          <p className="mt-0.5 text-xs text-muted-foreground">
            You leave every workspace and are signed out everywhere, desktop agents included. Your screenshots and webcam frames are
            deleted, and your tracked time keeps no name, window titles or notes.
          </p>
        </div>
      </CardHeader>
      <CardContent className="space-y-3">
        {!live ? (
          <p className="rounded-xl bg-muted/60 px-3 py-3 text-sm text-muted-foreground">The demo workspace has no account of yours to delete.</p>
        ) : check === "loading" ? (
          <p role="status" className="text-sm text-muted-foreground">
            Checking whether your account can be deleted…
          </p>
        ) : check === "failed" || check === null ? (
          <div role="alert" className="flex flex-wrap items-center gap-3 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-xs text-warning-on-tint">
            <span className="min-w-0 flex-1">Couldn&apos;t check whether your account can be deleted.</span>
            <Button size="sm" variant="outline" onClick={() => void load()}>
              Try again
            </Button>
          </div>
        ) : ready?.isPlatformAdministrator ? (
          <p className="text-sm text-muted-foreground">A platform administrator&apos;s account can&apos;t be deleted here.</p>
        ) : ready && ready.soleOwnerOf.length > 0 ? (
          <div className="space-y-2 text-sm">
            <p className="text-muted-foreground">You&apos;re the only owner of these workspaces. Make someone else an owner, or delete them, first:</p>
            <ul className="list-disc pl-5" aria-label="Workspaces you are the only owner of">
              {ready.soleOwnerOf.map((w) => (
                <li key={w.id} className="font-medium">
                  {w.name}
                </li>
              ))}
            </ul>
            <Link href="/workspaces" className="inline-block text-sm font-medium text-primary-strong underline underline-offset-2">
              Go to your workspaces
            </Link>
          </div>
        ) : !ready?.canDelete ? (
          <p className="text-sm text-muted-foreground">Your account can&apos;t be deleted right now. Try again later.</p>
        ) : (
          <Button variant="outline" className="border-danger/40 text-danger-on-tint hover:bg-danger/10" onClick={() => setOpen(true)}>
            <UserX aria-hidden className="h-4 w-4" /> Delete my account…
          </Button>
        )}
      </CardContent>

      {open && (
        <Modal open onClose={busy ? () => {} : () => setOpen(false)} title="Delete your account?" className="max-w-md">
          <form
            className="space-y-4"
            onSubmit={(e) => {
              e.preventDefault();
              void remove();
            }}
          >
            <p className="text-sm text-muted-foreground">This can&apos;t be undone. Enter your current password to confirm.</p>
            <div className="space-y-1.5">
              <label htmlFor={`${id}-current`} className="text-sm font-medium">
                Current password
              </label>
              <Input
                id={`${id}-current`}
                type="password"
                autoComplete="current-password"
                value={current}
                onChange={(e) => setCurrent(e.target.value)}
                autoFocus
                aria-describedby={error ? `${id}-error` : undefined}
                aria-invalid={error ? true : undefined}
              />
            </div>
            {error && (
              <p id={`${id}-error`} role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
                {error}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <Button type="button" variant="ghost" onClick={() => setOpen(false)} disabled={busy}>
                Cancel
              </Button>
              <Button type="submit" variant="danger" disabled={!current || busy}>
                {busy ? <Loader2 aria-hidden className="h-4 w-4 animate-spin motion-reduce:animate-none" /> : <Trash2 aria-hidden className="h-4 w-4" />}
                Delete my account
              </Button>
            </div>
          </form>
        </Modal>
      )}
    </Card>
  );
}
