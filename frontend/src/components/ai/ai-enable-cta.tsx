"use client";

import Link from "next/link";
import { Sparkles, X } from "lucide-react";
import { Card } from "@/components/ui/card";
import { linkButtonClass } from "@/components/dashboard/link-button";
import { useAiAssistant } from "@/components/ai/ai-provider";
import { useStoredChoice } from "@/hooks/use-stored-choice";
import { AI_COPY, aiSetupNotice } from "@/lib/ai-assist";

/** Where the dismissal is remembered (per viewer, through use-stored-choice). */
export const AI_CTA_KEY = "dosi-ai-cta";
const CHOICES = ["shown", "dismissed"] as const;

/**
 * The dashboard's prompt about AI assist itself, while the workspace hasn't
 * opted in (or must re-accept a changed disclosure; AI assist spec §18.3):
 * the owner gets "Turn on AI assist", which opens Settings > Workspace; a
 * manager learns the owner can turn it on. Members and clients never see it,
 * nor does anyone once AI assist is on or the server has none. Dismissible;
 * the dismissal is remembered for this viewer.
 */
export function AiEnableCta({ className }: { className?: string }) {
  const { status, role } = useAiAssistant();
  const [choice, setChoice] = useStoredChoice(AI_CTA_KEY, CHOICES, "shown");
  const notice = aiSetupNotice(status, role);
  if (choice === "dismissed" || (notice !== "turn-on" && notice !== "ask-owner")) return null;

  const owner = notice === "turn-on";
  const reconsent = status?.reason === "reconsent-required";
  return (
    <Card className={className}>
      <aside aria-label="AI assist" className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center">
        <Sparkles aria-hidden className="h-5 w-5 shrink-0 text-primary" />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">{owner ? (reconsent ? "Review the updated AI disclosure" : AI_COPY.turnOn) : AI_COPY.askOwner}</p>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {owner
              ? "Optional summaries of the week, a day or a report, written from your tracker data. You'll see exactly what is sent before anything is turned on."
              : "Optional summaries of the week, a day or a report, written from your tracker data. It is off until the owner turns it on."}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {owner && (
            <Link href="/settings?tab=workspace" className={linkButtonClass("primary", "md")}>
              {reconsent ? "Review and turn on" : AI_COPY.turnOn}
            </Link>
          )}
          <button
            type="button"
            onClick={() => setChoice("dismissed")}
            aria-label="Dismiss the AI assist note"
            className="inline-flex h-9 w-9 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <X aria-hidden className="h-4 w-4" />
          </button>
        </div>
      </aside>
    </Card>
  );
}
