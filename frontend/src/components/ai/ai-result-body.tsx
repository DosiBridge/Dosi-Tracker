"use client";

import Link from "next/link";
import { ArrowUpRight } from "lucide-react";
import { useAiAssistant } from "@/components/ai/ai-provider";
import type { AiFactRefDto, AiInsightItemDto, AiNextStepDto } from "@/lib/ai-contract";
import { AI_COPY, routeIdToHref } from "@/lib/ai-assist";
import { canAccess } from "@/lib/roles";

/** The parts of a result the body shows; every one is optional. */
export interface AiResultParts {
  headline?: string | null;
  summary?: string | null;
  items?: readonly AiInsightItemDto[];
  nextSteps?: readonly AiNextStepDto[];
  caveats?: readonly string[];
  /** Further questions (report explanations). */
  questions?: readonly string[];
  /** Figures used beyond the items' own (e.g. a data-quality item's). */
  facts?: readonly AiFactRefDto[];
}

/** Every figure the result used, once each, in the order first mentioned. */
function factsOf(parts: AiResultParts): AiFactRefDto[] {
  const seen = new Map<string, AiFactRefDto>();
  for (const fact of [...(parts.items ?? []).flatMap((i) => i.facts), ...(parts.facts ?? [])]) {
    if (!seen.has(fact.id)) seen.set(fact.id, fact);
  }
  return [...seen.values()];
}

/** The result as plain text, for Copy. */
export function aiResultText(parts: AiResultParts): string {
  const list = (label: string, rows: readonly string[] | undefined) => (rows && rows.length > 0 ? [label, ...rows.map((r) => `- ${r}`)] : []);
  return [
    parts.headline,
    parts.summary,
    ...(parts.items ?? []).map((i) => `- ${i.text}`),
    ...list("Next steps:", parts.nextSteps?.map((s) => s.label)),
    ...list("Keep in mind:", parts.caveats),
    ...list("Questions to explore:", parts.questions),
  ]
    .filter((line): line is string => !!line)
    .join("\n");
}

/** A route id as a link the viewer may open; null when the id is unknown or their role can't open it. */
function useRouteHref(): (routeId: string | null | undefined, arg?: string | null) => string | null {
  const { role } = useAiAssistant();
  return (routeId, arg) => {
    const href = routeIdToHref(routeId, arg);
    return href && canAccess(role, href) ? href : null;
  };
}

const linkClass = "inline-flex items-center gap-0.5 font-medium text-primary underline-offset-2 hover:underline";

/**
 * A result's text: headline, summary, items (each with its link when the
 * viewer may open it), next steps, caveats and questions, then "Show the
 * figures used". Text nodes only: no markup from the server is ever
 * interpreted.
 */
export function AiResultBody(parts: AiResultParts) {
  const hrefFor = useRouteHref();
  const facts = factsOf(parts);
  const steps = (parts.nextSteps ?? []).map((s) => ({ ...s, href: hrefFor(s.routeId) })).filter((s) => s.href);
  return (
    <div className="space-y-3 text-sm">
      {parts.headline && <p className="font-medium text-foreground">{parts.headline}</p>}
      {parts.summary && <p className="text-muted-foreground">{parts.summary}</p>}
      {parts.items && parts.items.length > 0 && (
        <ul className="space-y-1.5">
          {parts.items.map((item, i) => {
            const href = hrefFor(item.routeId);
            return (
              <li key={`${i}-${item.kind}`} className="flex gap-2">
                <span aria-hidden className="mt-2 h-1.5 w-1.5 shrink-0 rounded-full bg-primary/60" />
                <span>
                  {item.text}
                  {href && (
                    <>
                      {" "}
                      <Link href={href} className={linkClass}>
                        Open
                        <ArrowUpRight aria-hidden className="h-3 w-3" />
                      </Link>
                    </>
                  )}
                </span>
              </li>
            );
          })}
        </ul>
      )}
      {steps.length > 0 && (
        <div>
          <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Next steps</p>
          <ul className="mt-1 flex flex-wrap gap-x-4 gap-y-1">
            {steps.map((s) => (
              <li key={s.routeId}>
                <Link href={s.href!} className={linkClass}>
                  {s.label}
                  <ArrowUpRight aria-hidden className="h-3 w-3" />
                </Link>
              </li>
            ))}
          </ul>
        </div>
      )}
      {parts.caveats && parts.caveats.length > 0 && (
        <div>
          <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Keep in mind</p>
          <ul className="mt-1 list-disc space-y-0.5 pl-5 text-muted-foreground">
            {parts.caveats.map((c) => (
              <li key={c}>{c}</li>
            ))}
          </ul>
        </div>
      )}
      {parts.questions && parts.questions.length > 0 && (
        <div>
          <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Questions to explore</p>
          <ul className="mt-1 list-disc space-y-0.5 pl-5 text-muted-foreground">
            {parts.questions.map((q) => (
              <li key={q}>{q}</li>
            ))}
          </ul>
        </div>
      )}
      {facts.length > 0 && (
        <details className="group rounded-xl border border-border px-3 py-2">
          <summary className="cursor-pointer text-xs font-medium text-foreground">{AI_COPY.figuresUsed}</summary>
          <dl className="mt-2 grid grid-cols-[minmax(0,1fr)_auto] gap-x-4 gap-y-1 text-xs">
            {facts.map((f) => (
              <div key={f.id} className="contents">
                <dt className="text-muted-foreground">{f.label}</dt>
                <dd className="text-right font-medium tabular-nums text-foreground">{f.display}</dd>
              </div>
            ))}
          </dl>
        </details>
      )}
    </div>
  );
}
