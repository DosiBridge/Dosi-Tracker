"use client";

import { useEffect, useRef, useState } from "react";
import { Check, Copy, ThumbsDown, ThumbsUp } from "lucide-react";
import { Button } from "@/components/ui/button";
import { AiDisclosurePopover } from "@/components/ai/ai-disclosure-popover";
import { useAiAssistant } from "@/components/ai/ai-provider";
import { sendAiFeedback } from "@/hooks/ai-api";
import { useBrowserTimeZone } from "@/hooks/use-reporting-zone";
import type { AiFeedbackRating, AiFeedbackReason, AiResultMetaDto } from "@/lib/ai-contract";
import { AI_COPY, DEMO_NOTE, DISCLAIMER } from "@/lib/ai-assist";
import { formatInZone } from "@/lib/time-zone";
import { cn } from "@/lib/utils";

const REASONS: { value: AiFeedbackReason; label: string }[] = [
  { value: "inaccurate", label: "Inaccurate" },
  { value: "not-useful", label: "Not useful" },
  { value: "tone", label: "Tone" },
  { value: "other", label: "Other" },
];

const chip = (pressed: boolean) =>
  cn(
    "rounded-full border px-2.5 py-0.5 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
    pressed ? "border-primary bg-accent text-accent-foreground" : "border-border hover:bg-muted/60",
  );

/**
 * Under every AI result: the disclaimer (the demo note for sample output),
 * when it was generated and whether it was reused, "What's sent?", Helpful /
 * Not helpful (with a reason), and Copy. Feedback goes to the server only
 * for a live result; a demo keeps it on the page.
 */
export function AiFooter({ meta, copyText, reused = false }: { meta: AiResultMetaDto; copyText: string; reused?: boolean }) {
  const { live } = useAiAssistant();
  const timeZone = useBrowserTimeZone();
  const [rating, setRating] = useState<AiFeedbackRating | null>(null);
  const [reason, setReason] = useState<AiFeedbackReason | null>(null);
  const [feedbackNote, setFeedbackNote] = useState("");
  const [copied, setCopied] = useState<"copied" | "failed" | null>(null);
  const copyTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (copyTimer.current) clearTimeout(copyTimer.current);
    },
    [],
  );

  const demo = meta.status === "demo";
  const generatedAt = Date.parse(meta.generatedAt);
  const cached = reused || meta.status === "cached";
  const note = copied === "failed" ? "Couldn't copy: your browser blocked clipboard access." : feedbackNote;

  function feedback(nextRating: AiFeedbackRating, nextReason: AiFeedbackReason | null) {
    setRating(nextRating);
    setReason(nextReason);
    setFeedbackNote("Thanks for the feedback.");
    if (!live || demo) return;
    sendAiFeedback({ resultId: meta.resultId, rating: nextRating, reason: nextReason }).catch(() => {
      setFeedbackNote("Your feedback couldn't be sent.");
    });
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(copyText);
      setCopied("copied");
    } catch {
      setCopied("failed");
    }
    if (copyTimer.current) clearTimeout(copyTimer.current);
    copyTimer.current = setTimeout(() => setCopied(null), 1800);
  }

  return (
    <div className="mt-4 space-y-3 border-t border-border pt-3 text-xs text-muted-foreground">
      <p>{demo ? DEMO_NOTE : meta.disclaimer || DISCLAIMER}</p>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        {Number.isFinite(generatedAt) && (
          <span>
            Generated {formatInZone(generatedAt, timeZone, { hour: "2-digit", minute: "2-digit", hourCycle: "h23" })}
            {cached && `, ${AI_COPY.cachedNote}`}
          </span>
        )}
        <AiDisclosurePopover dataCategories={meta.dataCategories} />
        <div className="ml-auto flex flex-wrap items-center gap-1">
          <Button type="button" variant="ghost" size="sm" aria-pressed={rating === "up"} onClick={() => feedback("up", null)}>
            <ThumbsUp aria-hidden className="h-3.5 w-3.5" /> Helpful
          </Button>
          <Button type="button" variant="ghost" size="sm" aria-pressed={rating === "down"} onClick={() => feedback("down", null)}>
            <ThumbsDown aria-hidden className="h-3.5 w-3.5" /> Not helpful
          </Button>
          <Button type="button" variant="ghost" size="sm" onClick={() => void copy()}>
            {copied === "copied" ? <Check aria-hidden className="h-3.5 w-3.5" /> : <Copy aria-hidden className="h-3.5 w-3.5" />}
            {copied === "copied" ? "Copied" : "Copy"}
          </Button>
        </div>
      </div>
      {rating === "down" && (
        <div role="group" aria-label="What was wrong?" className="flex flex-wrap items-center gap-1.5">
          <span>What was wrong?</span>
          {REASONS.map((r) => (
            <button key={r.value} type="button" aria-pressed={reason === r.value} className={chip(reason === r.value)} onClick={() => feedback("down", r.value)}>
              {r.label}
            </button>
          ))}
        </div>
      )}
      <p aria-live="polite" className={cn(!note && "sr-only")}>
        {note}
      </p>
    </div>
  );
}
