"use client";

import { useId, useRef, useState } from "react";
import { Loader2, Pencil, StickyNote } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ACTIVITY_NOTE_MAX_LENGTH, normalizeActivityNote } from "@/lib/api-contract";
import { cn } from "@/lib/utils";

/* ============================================================================
 * NOTE EDITOR — a block's task note ("What are you working on?"), shown and,
 * for its owner (within 30 days) or a team manager, edited in place.
 *
 * Saving sends the note through `onSave` (PUT /api/app/activity/{id}/note in a
 * live session); the server tidies it exactly as normalizeActivityNote does,
 * so an empty note clears it. A failure keeps the draft and says why. Enter
 * saves, Shift+Enter starts a new line, Escape cancels — without closing the
 * dialog the editor sits in.
 * ========================================================================== */

/** Show a character count once the note is this close to the server's limit. */
const COUNT_FROM = ACTIVITY_NOTE_MAX_LENGTH - 200;

export interface NoteEditing {
  /** Whether the viewer may change this block's note. */
  canEdit: boolean;
  /** Store the note (null clears it); reject with a readable message when it can't be saved. */
  onSave: (note: string | null) => Promise<void>;
}

export function NoteEditor({
  note,
  editing,
  emptyLabel = "No task note",
  className,
}: {
  /** The current note (null: none). */
  note: string | null;
  /** Omitted: the note is read-only. */
  editing?: NoteEditing;
  /** Shown when there is no note. */
  emptyLabel?: string;
  className?: string;
}) {
  const id = useId();
  const fieldId = `${id}note`;
  const errorId = `${id}error`;
  const [draft, setDraft] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const editButton = useRef<HTMLButtonElement>(null);
  const field = useRef<HTMLTextAreaElement>(null);
  const open = draft !== null;
  const canEdit = !!editing?.canEdit;

  function start() {
    setDraft(note ?? "");
    setError(null);
    setSaved(false);
  }

  function cancel() {
    setDraft(null);
    setError(null);
    // Back to the button that opened the editor, not to the top of the dialog.
    window.requestAnimationFrame(() => editButton.current?.focus());
  }

  async function save() {
    if (!editing || draft === null || saving) return;
    const next = normalizeActivityNote(draft);
    if (next === (note ?? null)) {
      cancel();
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await editing.onSave(next);
      setDraft(null);
      setSaved(true);
      window.requestAnimationFrame(() => editButton.current?.focus());
    } catch (err) {
      setError(err instanceof Error && err.message ? err.message : "Couldn't save the note. Try again.");
      // The field was disabled while saving: give it focus back so the draft can be fixed or retried.
      window.requestAnimationFrame(() => field.current?.focus());
    } finally {
      setSaving(false);
    }
  }

  if (open) {
    const length = draft.length;
    return (
      <form
        className={cn("space-y-2", className)}
        onSubmit={(e) => {
          e.preventDefault();
          e.stopPropagation();
          void save();
        }}
      >
        <label htmlFor={fieldId} className="text-xs font-medium text-muted-foreground">
          Task note
        </label>
        <textarea
          ref={field}
          id={fieldId}
          value={draft}
          rows={2}
          maxLength={ACTIVITY_NOTE_MAX_LENGTH}
          autoFocus
          disabled={saving}
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? errorId : undefined}
          placeholder="What was this block about?"
          onChange={(e) => {
            setDraft(e.target.value);
            setError(null);
          }}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              // Cancel the edit only: the drawer or viewer around it stays open.
              e.preventDefault();
              e.stopPropagation();
              cancel();
            } else if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void save();
            }
          }}
          className={cn(
            "block w-full resize-y rounded-xl border border-input bg-card px-3 py-2 text-sm",
            "placeholder:text-muted-foreground focus:border-transparent focus:outline-none focus:ring-2 focus:ring-ring",
            "disabled:opacity-60",
            error && "border-danger",
          )}
        />
        {error && (
          <p id={errorId} role="alert" className="text-xs font-medium text-danger-on-tint">
            {error}
          </p>
        )}
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p className="text-[11px] text-muted-foreground">
            {length >= COUNT_FROM ? `${length} / ${ACTIVITY_NOTE_MAX_LENGTH}` : "Enter to save · Esc to cancel"}
          </p>
          <div className="flex gap-2">
            <Button type="button" variant="ghost" size="sm" onClick={cancel} disabled={saving}>
              Cancel
            </Button>
            <Button type="submit" size="sm" disabled={saving} aria-busy={saving || undefined}>
              {saving && <Loader2 aria-hidden className="h-3.5 w-3.5 motion-safe:animate-spin" />}
              {saving ? "Saving…" : "Save note"}
            </Button>
          </div>
        </div>
      </form>
    );
  }

  return (
    <div className={cn("flex items-start gap-2", className)}>
      <StickyNote aria-hidden className={cn("mt-0.5 h-4 w-4 shrink-0", note ? "text-primary" : "text-muted-foreground")} />
      <p className={cn("min-w-0 flex-1 text-sm", note ? "font-medium" : "text-muted-foreground")}>
        <span className="sr-only">Task note: </span>
        {note ?? emptyLabel}
      </p>
      {canEdit && (
        <button
          ref={editButton}
          type="button"
          onClick={start}
          className="inline-flex shrink-0 items-center gap-1 rounded-lg px-2 py-1 text-xs font-medium text-primary-strong transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <Pencil aria-hidden className="h-3.5 w-3.5" />
          {note ? "Edit note" : "Add note"}
        </button>
      )}
      <span className="sr-only" aria-live="polite">
        {saved ? "Note saved." : ""}
      </span>
    </div>
  );
}
