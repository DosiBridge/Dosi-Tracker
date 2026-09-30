"use client";

import { Modal } from "./modal";
import { Button } from "./button";

/**
 * Confirmation step for an action that is disruptive or hard to undo.
 *
 * Name the consequence in `description` — "this signs out 7 members" beats
 * "are you sure?" — and label the confirm button with the verb it performs
 * ("Suspend workspace"), never "OK", so the button itself states what happens.
 */
export function ConfirmDialog({
  open,
  onCancel,
  onConfirm,
  title,
  description,
  confirmLabel,
  cancelLabel = "Cancel",
  tone = "danger",
  busy = false,
}: {
  open: boolean;
  onCancel: () => void;
  onConfirm: () => void;
  title: string;
  description: string;
  confirmLabel: string;
  cancelLabel?: string;
  tone?: "danger" | "primary";
  busy?: boolean;
}) {
  return (
    <Modal open={open} onClose={onCancel} title={title} description={description} className="max-w-md">
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onCancel} disabled={busy}>
          {cancelLabel}
        </Button>
        <Button
          variant={tone === "danger" ? "danger" : "primary"}
          onClick={onConfirm}
          disabled={busy}
        >
          {confirmLabel}
        </Button>
      </div>
    </Modal>
  );
}
