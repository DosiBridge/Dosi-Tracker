"use client";

import { useId, useState } from "react";
import { ChevronDown, Sparkles } from "lucide-react";
import { useAiAssistant, useAiPageContext } from "@/components/ai/ai-provider";
import { useAiCards } from "@/components/ai/use-ai-cards";
import { ALL, type ReportingQuery } from "@/lib/api-contract";
import { aiAvailability } from "@/lib/ai-assist";
import { cn } from "@/lib/utils";

/** What a report's filters name: a filter left on "all" (or empty) names nothing. */
const scoped = (id: string | null | undefined) => (id && id !== ALL ? id : null);

/**
 * "Explain this report": a collapsed panel under a report's header (AI assist
 * spec §3.1, §18.5) that explains the report for exactly the period and
 * filters on screen. It never prints (`no-print`), opens only when asked
 * (the card downloads then, use-ai-cards), and generates only on Generate.
 * Hidden for anyone the server doesn't allow report explanations (members
 * and clients). Tells Ask AI which report and period the page shows.
 */
export function ReportExplainer({ slug, query, className }: { slug: string; query: ReportingQuery; className?: string }) {
  const panelId = useId();
  const [open, setOpen] = useState(false);
  const { status } = useAiAssistant();
  const availability = aiAvailability(status, "report-explanation");
  const timeZone = query.timeZone || "UTC";
  const from = query.from.toISOString();
  const to = query.to.toISOString();
  const projectId = scoped(query.projectId);
  const userId = scoped(query.userId);
  useAiPageContext(availability.visible ? { page: "reports", reportSlug: slug, from, to, projectId, memberId: userId, timeZone } : null);
  const cards = useAiCards(availability.visible && open);
  if (!availability.visible) return null;

  return (
    <div className={cn("no-print space-y-3", className)}>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((o) => !o)}
        className="inline-flex items-center gap-1.5 rounded-lg text-sm font-medium text-primary-strong transition-colors hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <Sparkles aria-hidden className="h-4 w-4" />
        Explain this report
        <ChevronDown aria-hidden className={cn("h-4 w-4 transition-transform", open && "rotate-180")} />
      </button>
      <div id={panelId} hidden={!open}>
        {open &&
          (cards ? (
            <cards.ReportExplanationBody
              title="Report explanation"
              purpose="A plain-language reading of this report's figures for the period and filters shown, with what to keep in mind."
              input={{ reportSlug: slug, from, to, projectId, userId, timeZone, dayBasis: query.dayBasis ?? null }}
              availability={availability}
              timeZone={timeZone}
            />
          ) : (
            <p role="status" className="text-sm text-muted-foreground">
              Loading…
            </p>
          ))}
      </div>
    </div>
  );
}
