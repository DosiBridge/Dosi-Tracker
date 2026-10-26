"use client";

import Link from "next/link";
import { ArrowUpRight } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { AiFooter } from "@/components/ai/ai-footer";
import { useAiAssistant } from "@/components/ai/ai-provider";
import type { AiAnswerSourceDto, AiFactRefDto, AiResultMetaDto } from "@/lib/ai-contract";
import { AI_ANSWER_LABEL, AI_COPY, DEMO_NOTE, aiAnswerBlocks, routeIdToHref } from "@/lib/ai-assist";
import { canAccess } from "@/lib/roles";

export interface AiAnswerProps {
  /** The answer's plain text; lines starting "- " are list items. */
  text: string;
  sources: readonly AiAnswerSourceDto[];
  facts: readonly AiFactRefDto[];
  isClarification?: boolean;
  meta?: AiResultMetaDto | null;
}

interface SourceChip {
  label: string;
  /** null: shown as text (no route, or a page this role can't open). */
  href: string | null;
}

/**
 * The sources an answer cites, as the viewer may follow them: a source on an
 * allowlisted route (§5.9) the role can open is a link, one without a route
 * is plain text, and one naming a route the web doesn't know is dropped.
 * Links come only from route ids: never from the answer's text.
 */
function sourceChips(sources: readonly AiAnswerSourceDto[], role: Parameters<typeof canAccess>[0]): SourceChip[] {
  return sources.flatMap((s): SourceChip[] => {
    if (!s.routeId) return [{ label: s.label, href: null }];
    const href = routeIdToHref(s.routeId, s.routeArg);
    if (!href) return [];
    return [{ label: s.label, href: canAccess(role, href) ? href : null }];
  });
}

const chipClass = "inline-flex max-w-full items-center gap-0.5 rounded-full border border-border px-2.5 py-0.5 text-xs";

/**
 * One Ask AI answer (§18.4): text nodes only — paragraphs, and the lists its
 * "- " lines make — then the sources it used, "Figures used", and a label
 * saying it is AI-generated (or a demo sample). No markup from the server is
 * ever interpreted.
 */
export function AiAnswer({ text, sources, facts, isClarification = false, meta = null }: AiAnswerProps) {
  const { role } = useAiAssistant();
  const blocks = aiAnswerBlocks(text);
  const chips = sourceChips(sources, role);
  const demo = meta?.status === "demo";
  return (
    <div className="space-y-2 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <Badge tone={demo ? "muted" : "info"}>{demo ? AI_COPY.sampleBadge : AI_COPY.aiBadge}</Badge>
        <span className="text-xs text-muted-foreground">{demo ? DEMO_NOTE : AI_ANSWER_LABEL}</span>
      </div>
      {blocks.map((block, i) =>
        block.kind === "paragraph" ? (
          <p key={i} className="text-foreground">
            {block.text}
          </p>
        ) : (
          <ul key={i} className="list-disc space-y-0.5 pl-5 text-foreground">
            {block.items.map((item, j) => (
              <li key={j}>{item}</li>
            ))}
          </ul>
        ),
      )}
      {chips.length > 0 && (
        <div>
          <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Sources</p>
          <ul className="mt-1 flex flex-wrap gap-1.5">
            {chips.map((c, i) => (
              <li key={`${i}-${c.label}`} className="max-w-full">
                {c.href ? (
                  <Link href={c.href} className={`${chipClass} font-medium text-primary-strong hover:bg-muted/60`}>
                    <span className="truncate">{c.label}</span>
                    <ArrowUpRight aria-hidden className="h-3 w-3 shrink-0" />
                  </Link>
                ) : (
                  <span className={`${chipClass} text-muted-foreground`}>
                    <span className="truncate">{c.label}</span>
                  </span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
      {facts.length > 0 && (
        <details className="rounded-xl border border-border px-3 py-2">
          <summary className="cursor-pointer text-xs font-medium text-foreground">Figures used</summary>
          <dl className="mt-2 grid grid-cols-[minmax(0,1fr)_auto] gap-x-4 gap-y-1 text-xs">
            {facts.map((f, i) => (
              <div key={`${i}-${f.id}`} className="contents">
                <dt className="text-muted-foreground">{f.label}</dt>
                <dd className="text-right font-medium tabular-nums text-foreground">{f.display}</dd>
              </div>
            ))}
          </dl>
        </details>
      )}
      {meta && !isClarification && <AiFooter meta={meta} copyText={text} compact />}
    </div>
  );
}
