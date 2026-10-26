"use client";

import { useId, useState } from "react";
import { Loader2, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Modal } from "@/components/ui/modal";
import { ERASURE_REASON_LIMITS } from "@/lib/api-contract";
import { REASON_PROBLEM_TEXT, erasureReasonProblem } from "@/lib/erasure";

/**
 * Confirm deleting tracked data (a block with its captures, or one capture),
 * giving the reason the deletion record keeps (E2E-07). What goes, and that
 * it can't be undone, is said before the button is pressed.
 */
export function ErasureDialog({
  title,
  consequences,
  reasonRequired,
  confirmLabel,
  onClose,
  onConfirm,
}: {
  title: string;
  consequences: string[];
  /** Owners and managers must give one; a member deleting their own capture may leave it out. */
  reasonRequired: boolean;
  confirmLabel: string;
  onClose: () => void;
  /** Resolves when deleted; rejects with an Error whose message is shown. */
  onConfirm: (reason: string | null) => Promise<void>;
}) {
  const id = useId();
  const [reason, setReason] = useState("");
  const [touched, setTouched] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const problem = erasureReasonProblem(reason, reasonRequired);
  const showProblem = touched && problem;

  async function confirm() {
    setTouched(true);
    if (problem || busy) return;
    setBusy(true);
    setError(null);
    try {
      await onConfirm(reason.trim() || null);
    } catch (err) {
      setError(err instanceof Error && err.message ? err.message : "Couldn't delete it. Try again.");
      setBusy(false);
    }
  }

  return (
    <Modal open onClose={busy ? () => {} : onClose} title={title} className="max-w-md">
      <form
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          void confirm();
        }}
        noValidate
      >
        <ul className="list-disc space-y-1.5 pl-5 text-sm text-muted-foreground">
          {consequences.map((c) => (
            <li key={c}>{c}</li>
          ))}
        </ul>
        <div className="space-y-1.5">
          <label htmlFor={`${id}-reason`} className="text-sm font-medium">
            Reason {!reasonRequired && <span className="font-normal text-muted-foreground">(optional)</span>}
          </label>
          <textarea
            id={`${id}-reason`}
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            onBlur={() => setTouched(true)}
            rows={2}
            maxLength={ERASURE_REASON_LIMITS.max + 50}
            required={reasonRequired}
            placeholder="e.g. Captured a personal banking page"
            aria-invalid={showProblem ? true : undefined}
            aria-describedby={`${id}-hint${showProblem || error ? ` ${id}-error` : ""}`}
            className="w-full rounded-xl border border-border bg-card px-3 py-2 text-sm placeholder:text-muted-foreground focus:border-transparent focus:outline-none focus:ring-2 focus:ring-ring"
          />
          <p id={`${id}-hint`} className="text-xs text-muted-foreground">
            Kept in the deletion record with who deleted it and when. The deleted content itself is not kept.
          </p>
        </div>
        {(showProblem || error) && (
          <p id={`${id}-error`} role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
            {showProblem ? REASON_PROBLEM_TEXT[problem] : error}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" variant="danger" disabled={busy}>
            {busy ? <Loader2 aria-hidden className="h-4 w-4 animate-spin motion-reduce:animate-none" /> : <Trash2 aria-hidden className="h-4 w-4" />}
            {confirmLabel}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
