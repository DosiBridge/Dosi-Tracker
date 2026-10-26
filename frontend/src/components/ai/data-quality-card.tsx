"use client";

import { useAiAssistant, useAiPageContext } from "@/components/ai/ai-provider";
import { useAiCards } from "@/components/ai/use-ai-cards";
import type { AiDataQualityKind, AiPage } from "@/lib/ai-contract";
import { aiAvailability } from "@/lib/ai-assist";

const toIso = (at: Date | string) => (typeof at === "string" ? at : at.toISOString());

/**
 * A tracking check (data-quality-review, AI assist spec §3.1, §18.4): what in
 * a period's tracking is worth a look — gaps in tracked days, a device clock
 * that is off, input that wasn't measured or came from software, members who
 * never tracked, outdated agents — each with possible reasons and, where one
 * helps, an action. "Week review" on the timesheet, "Agent health" on the
 * team page.
 *
 * The period is [from, to) in UTC (whole days of `timeZone`); `kinds` limits
 * the checks. Hidden unless the server allows this caller the review for the
 * scope; while shown it tells Ask AI which period the page is about. Written
 * only on Generate; the card itself downloads only once it would show.
 */
export function DataQualityCard({
  title,
  purpose,
  page,
  scope,
  kinds,
  from,
  to,
  timeZone,
  className,
}: {
  title: string;
  purpose: string;
  /** The page the card sits on, for Ask AI's page context. */
  page: AiPage;
  scope: "team" | "self";
  kinds: readonly AiDataQualityKind[];
  from: Date | string;
  to: Date | string;
  timeZone: string;
  className?: string;
}) {
  const { status } = useAiAssistant();
  const availability = aiAvailability(status, "data-quality-review", scope);
  const input = { from: toIso(from), to: toIso(to), scope, projectId: null, timeZone, kinds: [...kinds] };
  useAiPageContext(availability.visible ? { page, from: input.from, to: input.to, timeZone } : null);
  const cards = useAiCards(availability.visible);
  if (!availability.visible || !cards) return null;

  return <cards.DataQualityBody title={title} purpose={purpose} input={input} availability={availability} timeZone={timeZone} className={className} />;
}
