"use client";

import Link from "next/link";
import { AlertTriangle, Info, Loader2, Sparkles, X } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { linkButtonClass } from "@/components/dashboard/link-button";
import { AiInsightCard } from "@/components/ai/ai-insight-card";
import { useAiAssistant } from "@/components/ai/ai-provider";
import { AiResultBody, aiResultText } from "@/components/ai/ai-result-body";
import { useAiRequest } from "@/hooks/use-ai-request";
import { useBrowserTimeZone } from "@/hooks/use-reporting-zone";
import type {
  AiDataQualityItemDto,
  AiDataQualityReviewDto,
  AiDataQualityReviewInput,
  AiMemberDaySummaryDto,
  AiMemberDaySummaryInput,
  AiPeriodSummaryDto,
  AiPeriodSummaryInput,
  AiReportExplanationDto,
  AiReportExplanationInput,
} from "@/lib/ai-contract";
import {
  AI_ANSWER_LABEL,
  AI_COPY,
  DEMO_NOTE,
  dataQualityActionLabel,
  dataQualitySeverityLabel,
  routeIdToHref,
  type AiAvailability,
} from "@/lib/ai-assist";
import { canAccess } from "@/lib/roles";

/* ============================================================================
 * The bodies of the AI insight cards, behind their gates' one dynamic import
 * (use-ai-cards): each runs its request (useAiRequest: explicit click only,
 * cancellable, demo output with no network) and renders the shared card
 * shell. The gates (ai-period-summary-card, ai-day-summary-card,
 * report-explainer, data-quality-card, note-suggestions) decide whether a
 * card shows at all.
 * ========================================================================== */

interface BodyProps<I> {
  title: string;
  purpose: string;
  input: Omit<I, "regenerate">;
  availability: AiAvailability;
  timeZone: string;
  className?: string;
}

/** The request options every card shares: this session's workspace, cache and (demo) viewer. */
function useRequestOptions(availability: AiAvailability, timeZone: string) {
  const { live, workspaceId, cache, userId } = useAiAssistant();
  return { enabled: availability.visible, live, workspaceId, cache, demoContext: { userId, timeZone }, timeZone };
}

const periodBody = (r: AiPeriodSummaryDto) => ({ headline: r.headline, items: r.insights, nextSteps: r.nextSteps, caveats: r.caveats });

export function PeriodSummaryBody({ title, purpose, input, availability, timeZone, className }: BodyProps<AiPeriodSummaryInput>) {
  const request = useAiRequest("period-summary", input, useRequestOptions(availability, timeZone));
  return (
    <AiInsightCard
      title={title}
      purpose={purpose}
      request={request}
      availability={availability}
      className={className}
      renderResult={(r) => <AiResultBody {...periodBody(r)} />}
      resultText={(r) => aiResultText(periodBody(r))}
    />
  );
}

const dayBody = (r: AiMemberDaySummaryDto) => ({ headline: r.headline, summary: r.summary, items: r.highlights, caveats: r.caveats });

export function DaySummaryBody({ title, purpose, input, availability, timeZone, className }: BodyProps<AiMemberDaySummaryInput>) {
  const request = useAiRequest("member-day-summary", input, useRequestOptions(availability, timeZone));
  return (
    <AiInsightCard
      title={title}
      purpose={purpose}
      request={request}
      availability={availability}
      className={className}
      noDataLabel="Nothing was tracked on this day."
      renderResult={(r) => <AiResultBody {...dayBody(r)} />}
      resultText={(r) => aiResultText(dayBody(r))}
    />
  );
}

const reportBody = (r: AiReportExplanationDto) => ({ summary: r.summary, items: r.keyFindings, caveats: r.caveats, questions: r.questionsToExplore });

export function ReportExplanationBody({ title, purpose, input, availability, timeZone, className }: BodyProps<AiReportExplanationInput>) {
  const request = useAiRequest("report-explanation", input, useRequestOptions(availability, timeZone));
  return (
    <AiInsightCard
      title={title}
      purpose={purpose}
      request={request}
      availability={availability}
      className={className}
      readyLabel="Explanation ready"
      noDataLabel="Nothing was tracked for this report's period."
      renderResult={(r) => <AiResultBody {...reportBody(r)} />}
      resultText={(r) => aiResultText(reportBody(r))}
    />
  );
}

/* ----------------------------- Data quality ----------------------------- */

const NOTHING_UNUSUAL = "Nothing in this period's tracking looks unusual.";

/** A review as plain text, for Copy. */
function dataQualityText(r: AiDataQualityReviewDto): string {
  if (r.items.length === 0) return NOTHING_UNUSUAL;
  return r.items
    .flatMap((item) => [
      `- ${dataQualitySeverityLabel(item.severity)}: ${item.title}${item.subjectName ? ` (${item.subjectName})` : ""}. ${item.explanation}`,
      ...item.possibleReasons.map((reason) => `  - Possible reason: ${reason}`),
    ])
    .join("\n");
}

function DataQualityFinding({ item }: { item: AiDataQualityItemDto }) {
  const { role } = useAiAssistant();
  const href = routeIdToHref(item.actionRouteId);
  const action = href && canAccess(role, href) ? href : null;
  const notice = item.severity === "notice";
  const Icon = notice ? AlertTriangle : Info;
  return (
    <li className="space-y-1.5 rounded-xl border border-border p-3">
      <div className="flex flex-wrap items-center gap-2">
        {/* Severity is the icon AND its words: never colour alone. */}
        <Badge tone={notice ? "warning" : "muted"} className="gap-1">
          <Icon aria-hidden className="h-3 w-3" />
          {dataQualitySeverityLabel(item.severity)}
        </Badge>
        <span className="font-medium text-foreground">{item.title}</span>
        {item.subjectName && <span className="text-xs text-muted-foreground">· {item.subjectName}</span>}
      </div>
      <p className="text-muted-foreground">{item.explanation}</p>
      {item.possibleReasons.length > 0 && (
        <div>
          <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Possible reasons</p>
          <ul className="mt-0.5 list-disc space-y-0.5 pl-5 text-muted-foreground">
            {item.possibleReasons.map((reason) => (
              <li key={reason}>{reason}</li>
            ))}
          </ul>
        </div>
      )}
      {item.facts.length > 0 && (
        <dl className="flex flex-wrap gap-x-4 gap-y-0.5 text-xs">
          {item.facts.map((f, i) => (
            <div key={`${i}-${f.id}`} className="flex gap-1">
              <dt className="text-muted-foreground">{f.label}:</dt>
              <dd className="font-medium tabular-nums text-foreground">{f.display}</dd>
            </div>
          ))}
        </dl>
      )}
      {action && (
        <Link href={action} className={linkButtonClass("outline", "sm")}>
          {dataQualityActionLabel(item.actionId)}
        </Link>
      )}
    </li>
  );
}

function DataQualityResult({ result }: { result: AiDataQualityReviewDto }) {
  if (result.items.length === 0) return <p className="text-sm text-muted-foreground">{NOTHING_UNUSUAL}</p>;
  const more = result.candidateCount - result.items.length;
  return (
    <div className="space-y-2 text-sm">
      <ul className="space-y-2">
        {result.items.map((item) => (
          <DataQualityFinding key={item.candidateId} item={item} />
        ))}
      </ul>
      {more > 0 && (
        <p className="text-xs text-muted-foreground">
          {more} more {more === 1 ? "finding was" : "findings were"} left out; the ones above matter most.
        </p>
      )}
    </div>
  );
}

export function DataQualityBody({ title, purpose, input, availability, timeZone, className }: BodyProps<AiDataQualityReviewInput>) {
  const request = useAiRequest("data-quality-review", input, useRequestOptions(availability, timeZone));
  return (
    <AiInsightCard
      title={title}
      purpose={purpose}
      request={request}
      availability={availability}
      className={className}
      readyLabel="Review ready"
      // A review answers "no-data" whenever no rule found a candidate (spec: zero candidates, no call), tracked time or not.
      noDataLabel={NOTHING_UNUSUAL}
      renderResult={(r) => <DataQualityResult result={r} />}
      resultText={dataQualityText}
    />
  );
}

/* ----------------------------- Note suggestions ----------------------------- */

const suggestionClass =
  "max-w-full rounded-full border border-border px-2.5 py-1 text-left text-xs font-medium transition-colors hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50";

/**
 * The note editor's "Suggest": asked only on a click, then up to three notes
 * to choose from. Choosing fills the editor's draft and nothing else.
 */
export function NoteSuggestionsBody({ activityId, onPick, disabled }: { activityId: string; onPick: (text: string) => void; disabled: boolean }) {
  const { live, workspaceId, cache, userId } = useAiAssistant();
  const timeZone = useBrowserTimeZone();
  const { state, run, cancel } = useAiRequest("note-suggestions", { activityId }, { live, workspaceId, cache, demoContext: { userId, timeZone }, timeZone });
  const loading = state.phase === "loading";
  const result = state.phase === "ready" ? state.result : null;
  const suggestions = result?.suggestions.filter((s) => s.text.trim()) ?? [];
  const demo = result?.meta.status === "demo";
  let announcement = "";
  if (loading) announcement = "Finding suggestions...";
  else if (result) announcement = suggestions.length > 0 ? "Suggestions ready" : "No suggestions";

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        {loading && (
          <Button type="button" variant="ghost" size="sm" onClick={cancel}>
            <X aria-hidden className="h-3.5 w-3.5" /> Cancel suggestions
          </Button>
        )}
        {!loading && !result && (
          <Button type="button" variant="ghost" size="sm" onClick={() => run()} disabled={disabled}>
            <Sparkles aria-hidden className="h-3.5 w-3.5" /> Suggest
          </Button>
        )}
        {loading && <Loader2 aria-hidden className="h-3.5 w-3.5 text-muted-foreground motion-safe:animate-spin" />}
        <span role="status" aria-live="polite" className={loading ? "text-xs text-muted-foreground" : "sr-only"}>
          {announcement}
        </span>
      </div>
      {state.phase === "error" && (
        <div role="alert" className="flex flex-wrap items-center justify-between gap-2 rounded-lg bg-danger/10 px-3 py-2 text-xs text-danger-on-tint">
          <span>{state.error.message}</span>
          {state.error.retryable && (
            <Button type="button" size="sm" variant="outline" onClick={() => run()}>
              {AI_COPY.retry}
            </Button>
          )}
        </div>
      )}
      {result && suggestions.length === 0 && <p className="text-xs text-muted-foreground">No suggestions for this block.</p>}
      {suggestions.length > 0 && (
        <div className="space-y-1.5">
          <div role="group" aria-label="Suggested notes" className="flex flex-wrap gap-1.5">
            {suggestions.map((s, i) => (
              <button key={`${i}-${s.text}`} type="button" disabled={disabled} onClick={() => onPick(s.text)} className={suggestionClass}>
                {s.text}
              </button>
            ))}
          </div>
          <p className="flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
            <Badge tone={demo ? "muted" : "info"}>{demo ? AI_COPY.sampleBadge : AI_COPY.aiBadge}</Badge>
            {demo ? DEMO_NOTE : `${AI_ANSWER_LABEL}. Choosing one fills the note; edit it, then save.`}
          </p>
        </div>
      )}
    </div>
  );
}
