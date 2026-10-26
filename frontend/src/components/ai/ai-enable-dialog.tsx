"use client";

import { useId, useState } from "react";
import { Button } from "@/components/ui/button";
import { Modal } from "@/components/ui/modal";
import type { AiDisclosureDto } from "@/lib/ai-contract";
import type { AiAcknowledgementReason } from "@/lib/ai-assist";

function DisclosureList({ label, items }: { label: string; items: readonly string[] }) {
  if (items.length === 0) return null;
  return (
    <div>
      <h3 className="text-sm font-semibold text-foreground">{label}</h3>
      <ul className="mt-1 list-disc space-y-0.5 pl-5 text-sm text-muted-foreground">
        {items.map((text) => (
          <li key={text}>{text}</li>
        ))}
      </ul>
    </div>
  );
}

function titleFor(reasons: readonly AiAcknowledgementReason[]): { title: string; confirm: string } {
  if (reasons.includes("enable")) return { title: "Turn on AI assist", confirm: "Turn on AI assist" };
  if (reasons.includes("reconsent")) return { title: "Review the updated AI disclosure", confirm: "Accept and turn on" };
  return { title: "Send more to AI assist", confirm: "Accept and save" };
}

/**
 * What the owner reads before AI assist sends anything (AI assist spec §8,
 * §18.6): the server's own disclosure — the processor, where it processes
 * and what it keeps, what is always sent, sent only when allowed and never
 * sent, who can use it — plus that members are notified and that output can
 * be wrong. Nothing is saved until the owner ticks the acknowledgement; the
 * request then carries the version they read.
 */
export function AiEnableDialog({
  disclosure,
  reasons,
  busy,
  error,
  onCancel,
  onConfirm,
}: {
  disclosure: AiDisclosureDto;
  /** Why the acknowledgement is needed (lib/ai-assist aiAcknowledgementReasons). */
  reasons: readonly AiAcknowledgementReason[];
  busy: boolean;
  /** Why the last attempt failed (shown as an alert inside the dialog). */
  error: string | null;
  onCancel: () => void;
  /** Called with the disclosure version the owner acknowledged. */
  onConfirm: (version: string) => void;
}) {
  const id = useId();
  const [accepted, setAccepted] = useState(false);
  const { title, confirm } = titleFor(reasons);
  const provider = disclosure.providerDisplayName;
  const extras = [
    reasons.includes("window-titles") ? "window titles" : null,
    reasons.includes("notes") ? "task notes" : null,
  ].filter((x): x is string => x !== null);

  return (
    <Modal open onClose={busy ? () => {} : onCancel} title={title} description={`Disclosure version ${disclosure.version}`} className="max-w-xl">
      <div className="space-y-4">
        <p className="text-sm">
          AI summaries are written by <span className="font-medium">{provider}</span>
          {disclosure.processingRegion ? <>, processed in {disclosure.processingRegion}</> : null}. Only aggregated tracker data is sent, and only when
          someone presses Generate.
        </p>
        {disclosure.retentionNote && <p className="text-sm text-muted-foreground">{disclosure.retentionNote}</p>}
        {extras.length > 0 && (
          <p role="note" className="rounded-xl bg-warning/10 px-3 py-2 text-sm text-warning-on-tint">
            This change adds {extras.join(" and ")} to what may be sent. {extras.length > 1 ? "They are" : "It is"} written by people and can hold
            private details.
          </p>
        )}
        <DisclosureList label="Always sent" items={disclosure.alwaysSent} />
        <DisclosureList label="Sent only when you allow it" items={disclosure.sentWhenAllowed} />
        <DisclosureList label="Never sent" items={disclosure.neverSent} />
        <DisclosureList label="Who can use it" items={disclosure.whoCanUse} />
        <p className="text-sm text-muted-foreground">
          Every active member gets a notice when AI assist is turned on, and when window titles or task notes are included.
        </p>
        <p className="text-sm text-muted-foreground">
          AI output can be wrong. It is labelled as AI-generated, and it is never meant for decisions about people on its own. Names are replaced with
          aliases before anything is sent; that is pseudonymization, not anonymity.
        </p>
        <label htmlFor={`${id}-ack`} className="flex items-start gap-2.5 rounded-xl border border-border p-3 text-sm">
          <input
            id={`${id}-ack`}
            type="checkbox"
            required
            checked={accepted}
            onChange={(e) => setAccepted(e.target.checked)}
            className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--primary)]"
          />
          <span>I&apos;ve read this and our organisation may process this data with {provider}</span>
        </label>
        {error && (
          <p role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-sm text-danger-on-tint">
            {error}
          </p>
        )}
        <div className="flex flex-wrap justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
          <Button type="button" onClick={() => onConfirm(disclosure.version)} disabled={!accepted || busy}>
            {busy ? "Saving…" : confirm}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
