"use client";

import { useEffect, useId, useState } from "react";
import { RefreshCw, Sparkles, X } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { AiDisclosurePopover } from "@/components/ai/ai-disclosure-popover";
import { AiFooter } from "@/components/ai/ai-footer";
import { useAiAssistant } from "@/components/ai/ai-provider";
import { AiUnavailableNote } from "@/components/ai/ai-unavailable-note";
import type { AiRequest } from "@/hooks/use-ai-request";
import { useBrowserTimeZone } from "@/hooks/use-reporting-zone";
import type { AiResultMetaDto } from "@/lib/ai-contract";
import { AI_COPY, quotaNote, resultBadge, type AiAvailability } from "@/lib/ai-assist";
import { cn } from "@/lib/utils";

/** After this long, a request still loading says it can be cancelled. */
export const AI_SLOW_AFTER_MS = 10_000;

export interface AiInsightCardProps<T extends { meta: AiResultMetaDto }> {
  title: string;
  /** One line on what Generate produces (the idle state). */
  purpose: string;
  /** From useAiRequest. */
  request: AiRequest<T>;
  /** The ready result's body (usually an AiResultBody). */
  renderResult: (result: T) => React.ReactNode;
  /** The ready result as plain text, for Copy. */
  resultText: (result: T) => string;
  /** From aiAvailability: a "quota" card keeps Generate and adds the allowance note. */
  availability?: AiAvailability;
  /** Announced when a result arrives (default "Summary ready"). */
  readyLabel?: string;
  /** Shown for a result with nothing tracked (default "Nothing was tracked for this period."). */
  noDataLabel?: string;
  className?: string;
}

function Skeleton() {
  return (
    <div aria-hidden className="space-y-2">
      <div className="h-3 w-3/4 rounded bg-muted motion-safe:animate-pulse" />
      <div className="h-3 w-full rounded bg-muted motion-safe:animate-pulse" />
      <div className="h-3 w-5/6 rounded bg-muted motion-safe:animate-pulse" />
    </div>
  );
}

/**
 * The shell every AI insight shares (AI assist spec §18.3/§18.4): a section
 * with a real heading and a visible badge ("AI"; "Basic summary" for the
 * fallback; "Sample" in the demo; none when nothing was tracked), and the
 * states
 *  - idle: what it does, "What's sent?" and Generate;
 *  - loading: a skeleton, a polite "Writing summary...", Cancel, and after
 *    10 s "Still working - you can cancel.";
 *  - ready: the result and its footer; "Summary ready" is announced, focus
 *    stays where it was;
 *  - error: an alert, with Retry when trying again can help.
 * Nothing generates until the person presses Generate.
 */
export function AiInsightCard<T extends { meta: AiResultMetaDto }>({
  title,
  purpose,
  request,
  renderResult,
  resultText,
  availability,
  readyLabel = AI_COPY.ready,
  noDataLabel = AI_COPY.noData,
  className,
}: AiInsightCardProps<T>) {
  const headingId = useId();
  const { live, status } = useAiAssistant();
  const timeZone = useBrowserTimeZone();
  const { state, run, cancel } = request;
  const loading = state.phase === "loading";

  // "Still working" belongs to one run: a new run starts its own 10 s.
  const [runs, setRuns] = useState(0);
  const [slowRun, setSlowRun] = useState(-1);
  useEffect(() => {
    if (!loading) return;
    const timer = setTimeout(() => setSlowRun(runs), AI_SLOW_AFTER_MS);
    return () => clearTimeout(timer);
  }, [loading, runs]);
  const slow = loading && slowRun === runs;

  const start = (regenerate: boolean) => {
    setRuns((n) => n + 1);
    run(regenerate);
  };

  const result = state.phase === "ready" ? state.result : null;
  const badge = result ? resultBadge(result.meta) : live ? resultBadge(null) : resultBadge({ status: "demo" });
  const noData = result?.meta.status === "no-data";
  const quota = availability?.reason === "quota" ? quotaNote(status?.quota?.resetsAt, timeZone) : null;

  let announcement = "";
  if (loading) announcement = slow ? `${AI_COPY.loading} ${AI_COPY.slow}` : AI_COPY.loading;
  else if (result) announcement = noData ? noDataLabel : readyLabel;

  return (
    <Card className={className}>
      <section aria-labelledby={headingId}>
        <CardHeader>
          <div className="flex min-w-0 items-center gap-2">
            <Sparkles aria-hidden className="h-4 w-4 shrink-0 text-primary" />
            <CardTitle id={headingId}>{title}</CardTitle>
            {badge && <Badge tone={badge.tone}>{badge.label}</Badge>}
          </div>
          <div className="flex items-center gap-2">
            {state.phase === "idle" && (
              <Button type="button" size="sm" onClick={() => start(false)}>
                {AI_COPY.generate}
              </Button>
            )}
            {loading && (
              <Button type="button" size="sm" variant="outline" onClick={cancel}>
                <X aria-hidden className="h-3.5 w-3.5" />
                {AI_COPY.cancel}
              </Button>
            )}
            {result && !noData && (
              <Button type="button" size="sm" variant="outline" onClick={() => start(true)}>
                <RefreshCw aria-hidden className="h-3.5 w-3.5" />
                {AI_COPY.regenerate}
              </Button>
            )}
          </div>
        </CardHeader>
        <CardContent className="space-y-3">
          {quota && state.phase !== "loading" && <AiUnavailableNote message={quota} />}
          {state.phase === "idle" && (
            <div className="flex flex-wrap items-center justify-between gap-2">
              <p className="text-sm text-muted-foreground">{purpose}</p>
              <AiDisclosurePopover />
            </div>
          )}
          {loading && <Skeleton />}
          <p role="status" aria-live="polite" className={cn(loading ? "text-sm text-muted-foreground" : "sr-only")}>
            {announcement}
          </p>
          {state.phase === "error" && (
            <div role="alert" className="flex flex-wrap items-center justify-between gap-2 rounded-xl bg-danger/10 px-3 py-2 text-sm text-danger-on-tint">
              <span>{state.error.message}</span>
              {state.error.retryable && (
                <Button type="button" size="sm" variant="outline" onClick={() => start(false)}>
                  {AI_COPY.retry}
                </Button>
              )}
            </div>
          )}
          {result && noData && <p className="text-sm text-muted-foreground">{noDataLabel}</p>}
          {result && !noData && (
            <div>
              {result.meta.status === "fallback" && <p className="mb-2 text-xs text-muted-foreground">{AI_COPY.fallbackNote}</p>}
              {renderResult(result)}
              <AiFooter meta={result.meta} copyText={resultText(result)} reused={state.phase === "ready" && state.reused} />
            </div>
          )}
        </CardContent>
      </section>
    </Card>
  );
}
