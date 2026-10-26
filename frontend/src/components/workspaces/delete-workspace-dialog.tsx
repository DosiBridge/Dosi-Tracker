"use client";

import { useId, useState } from "react";
import { Loader2, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Modal } from "@/components/ui/modal";
import { deleteWorkspaceApi } from "@/hooks/useApi";
import type { DeletedWorkspaceDto } from "@/lib/api-contract";
import { erasureErrorText, workspaceNameConfirmed } from "@/lib/erasure";

/**
 * Delete a workspace for good (owners; E2E-07). Typing its name confirms it —
 * the server checks the same thing. The consequences are spelled out before
 * the button unlocks: everyone loses it at once, agents stop, invitations are
 * withdrawn, the data is purged after the grace period.
 */
export function DeleteWorkspaceDialog({
  workspace,
  live,
  onClose,
  onDeleted,
}: {
  workspace: { id: string; name: string };
  /** False in the demo: nothing is sent, the page says what would happen. */
  live: boolean;
  onClose: () => void;
  onDeleted: (result: DeletedWorkspaceDto | null) => void;
}) {
  const id = useId();
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const confirmed = workspaceNameConfirmed(typed, workspace.name);

  async function remove() {
    if (!confirmed || busy) return;
    if (!live) {
      onDeleted(null);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      onDeleted(await deleteWorkspaceApi(workspace.id, typed.trim()));
    } catch (err) {
      setError(erasureErrorText(err, "Couldn't delete the workspace. Try again."));
      setBusy(false);
    }
  }

  return (
    <Modal open onClose={busy ? () => {} : onClose} title={`Delete ${workspace.name}?`} className="max-w-md">
      <form
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          void remove();
        }}
      >
        <ul className="list-disc space-y-1.5 pl-5 text-sm text-muted-foreground">
          <li>It disappears for everyone right away: members lose access and their desktop agents stop tracking for it.</li>
          <li>Pending invitations are withdrawn, and any subscription is canceled.</li>
          <li>
            Its time blocks, screenshots and webcam frames are erased after a short grace period. They can&apos;t be recovered
            afterwards. Invoices already issued are kept.
          </li>
        </ul>
        <p className="text-sm text-muted-foreground">
          To keep the data instead, <span className="font-medium text-foreground">archive</span> the workspace.
        </p>
        <div className="space-y-1.5">
          <label htmlFor={`${id}-name`} className="text-sm font-medium">
            Type <span className="font-mono">{workspace.name}</span> to confirm
          </label>
          <Input
            id={`${id}-name`}
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            autoComplete="off"
            spellCheck={false}
            autoFocus
            aria-describedby={error ? `${id}-error` : undefined}
          />
        </div>
        {error && (
          <p id={`${id}-error`} role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" variant="danger" disabled={!confirmed || busy}>
            {busy ? <Loader2 aria-hidden className="h-4 w-4 animate-spin motion-reduce:animate-none" /> : <Trash2 aria-hidden className="h-4 w-4" />}
            Delete workspace
          </Button>
        </div>
      </form>
    </Modal>
  );
}
