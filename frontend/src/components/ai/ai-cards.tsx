"use client";

import { AiInsightCard } from "@/components/ai/ai-insight-card";
import { useAiAssistant } from "@/components/ai/ai-provider";
import { AiResultBody, aiResultText } from "@/components/ai/ai-result-body";
import { useAiRequest } from "@/hooks/use-ai-request";
import type {
  AiMemberDaySummaryDto,
  AiMemberDaySummaryInput,
  AiPeriodSummaryDto,
  AiPeriodSummaryInput,
  AiReportExplanationDto,
  AiReportExplanationInput,
} from "@/lib/ai-contract";
import type { AiAvailability } from "@/lib/ai-assist";

/* ============================================================================
 * The bodies of the AI insight cards, behind their gates' one dynamic import
 * (use-ai-cards): each runs its request (useAiRequest: explicit click only,
 * cancellable, demo output with no network) and renders the shared card
 * shell. The gates (ai-period-summary-card, ai-day-summary-card,
 * report-explainer) decide whether a card shows at all.
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
