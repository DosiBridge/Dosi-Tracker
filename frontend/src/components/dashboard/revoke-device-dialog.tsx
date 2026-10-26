"use client";

import { useId, useState } from "react";
import { Loader2, ShieldOff } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Modal } from "@/components/ui/modal";
import { DEVICE_REASON_MAX_LENGTH } from "@/lib/api-contract";

/**
 * Revoke a desktop agent install (E2E-09). Spells out what happens before it
 * does: the install is signed out at once, refused in this workspace, and has
 * to sign in again after a restore. The reason is optional and kept with the
 * revocation; the person the device belongs to may see it.
 */
export function RevokeDeviceDialog({
  device,
  own,
  onClose,
  onConfirm,
}: {
  device: { name: string; memberName: string };
  /** The viewer's own device (they can't restore it themselves). */
  own: boolean;
  onClose: () => void;
  onConfirm: (reason: string | null) => Promise<void>;
}) {
  const id = useId();
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const tooLong = reason.trim().length > DEVICE_REASON_MAX_LENGTH;

  async function confirm() {
    if (busy || tooLong) return;
    setBusy(true);
    setError(null);
    try {
      await onConfirm(reason.trim() || null);
    } catch (err) {
      setError(err instanceof Error && err.message ? err.message : "Couldn't revoke the device. Try again.");
      setBusy(false);
    }
  }

  return (
    <Modal open onClose={busy ? () => {} : onClose} title={`Revoke ${device.name}?`} className="max-w-md">
      <form
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          void confirm();
        }}
      >
        <ul className="list-disc space-y-1.5 pl-5 text-sm text-muted-foreground">
          <li>
            {own ? "Your" : `${device.memberName}'s`} desktop agent on it is signed out right away and can&apos;t track time or send its
            status in this workspace.
          </li>
          <li>Anything it hasn&apos;t uploaded yet stays on the computer; it isn&apos;t sent while the device is revoked.</li>
          <li>
            {own
              ? "Only a workspace owner or manager can restore it. After that, you sign in to the agent again."
              : "You can restore it later. The agent then has to sign in again."}
          </li>
        </ul>
        <div className="space-y-1.5">
          <label htmlFor={`${id}-reason`} className="text-sm font-medium">
            Reason <span className="font-normal text-muted-foreground">(optional)</span>
          </label>
          <textarea
            id={`${id}-reason`}
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            rows={2}
            maxLength={DEVICE_REASON_MAX_LENGTH + 50}
            placeholder="e.g. Laptop lost"
            aria-describedby={`${id}-hint${error || tooLong ? ` ${id}-error` : ""}`}
            className="w-full rounded-xl border border-border bg-card px-3 py-2 text-sm placeholder:text-muted-foreground focus:border-transparent focus:outline-none focus:ring-2 focus:ring-ring"
          />
          <p id={`${id}-hint`} className="text-xs text-muted-foreground">
            Kept with the revocation. Up to {DEVICE_REASON_MAX_LENGTH} characters.
          </p>
        </div>
        {(error || tooLong) && (
          <p id={`${id}-error`} role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
            {tooLong ? `Use at most ${DEVICE_REASON_MAX_LENGTH} characters.` : error}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" variant="danger" disabled={busy || tooLong}>
            {busy ? <Loader2 aria-hidden className="h-4 w-4 animate-spin motion-reduce:animate-none" /> : <ShieldOff aria-hidden className="h-4 w-4" />}
            Revoke device
          </Button>
        </div>
      </form>
    </Modal>
  );
}
