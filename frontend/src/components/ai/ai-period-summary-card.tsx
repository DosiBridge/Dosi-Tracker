"use client";

import { useAiAssistant, useAiPageContext } from "@/components/ai/ai-provider";
import { useAiCards } from "@/components/ai/use-ai-cards";
import { aiAvailability } from "@/lib/ai-assist";

const toIso = (at: Date | string) => (typeof at === "string" ? at : at.toISOString());

function copyFor(scope: "team" | "self", projectId: string | null | undefined): { title: string; purpose: string } {
  if (projectId) return { title: "Project pulse", purpose: "A short summary of this project's tracked time: where it went and what to check." };
  if (scope === "self") return { title: "My week", purpose: "A short summary of your tracked time this week: where it went and how it compares." };
  return { title: "This week at a glance", purpose: "A short summary of the team's tracked time this week: where it went, how it compares, and what to check." };
}

/**
 * "This week at a glance" (team), "My week" (self) or "Project pulse" (a
 * project): an AI summary of a period, written only when the person presses
 * Generate (AI assist spec §3.1, §18.5). The period is [from, to) in UTC —
 * whole days of the reporting zone, which the summary's days are read in.
 * Hidden unless the server allows this caller the period summary for this
 * scope; while shown, it tells Ask AI which period the page is about. The
 * card itself downloads only once it would show (use-ai-cards).
 */
export function AiPeriodSummaryCard({
  scope,
  projectId = null,
  from,
  to,
  timeZone,
  title,
  className,
}: {
  scope: "team" | "self";
  projectId?: string | null;
  from: Date | string;
  to: Date | string;
  timeZone: string;
  title?: string;
  className?: string;
}) {
  const { status } = useAiAssistant();
  const availability = aiAvailability(status, "period-summary", scope);
  const input = { from: toIso(from), to: toIso(to), scope, projectId, timeZone };
  useAiPageContext(
    availability.visible ? { page: projectId ? "projects" : "dashboard", from: input.from, to: input.to, projectId, timeZone } : null,
  );
  const cards = useAiCards(availability.visible);
  if (!availability.visible || !cards) return null;

  const copy = copyFor(scope, projectId);
  return (
    <cards.PeriodSummaryBody
      title={title ?? copy.title}
      purpose={copy.purpose}
      input={input}
      availability={availability}
      timeZone={timeZone}
      className={className}
    />
  );
}
