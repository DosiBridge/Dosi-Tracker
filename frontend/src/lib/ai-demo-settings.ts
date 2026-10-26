import type { AiUsageSummaryDto, AiWorkspaceSettingsDto } from "./ai-contract";
import { WHATS_SENT } from "./ai-assist";
import { DEMO_DISCLOSURE_VERSION, DEMO_PROVIDER, demoAiStatus } from "./ai-demo-status";
import { NOW } from "./demo-clock";
import type { Role } from "./types";

/* ============================================================================
 * The owner's AI settings and this month's usage in a demo workspace: what
 * Settings > Workspace shows when there is no server. Kept apart from
 * lib/ai-demo-status (which every session's status reads) because only the
 * settings card, which loads on demand, ever needs them.
 * ========================================================================== */

/** The owner's AI settings as a demo workspace starts them: on, with the code defaults (§8). */
export function demoAiSettings(): AiWorkspaceSettingsDto {
  return {
    isEnabled: true,
    allowManagers: true,
    allowMemberSelfService: true,
    allowWindowTitles: false,
    allowNotes: false,
    consentVersion: DEMO_DISCLOSURE_VERSION,
    enabledAt: new Date(NOW.getTime() - 14 * 86_400_000).toISOString(),
    enabledByName: null,
    disclosure: {
      version: DEMO_DISCLOSURE_VERSION,
      providerDisplayName: DEMO_PROVIDER,
      processingRegion: null,
      retentionNote: "Nothing is sent or kept: the demo writes its sample output in your browser.",
      alwaysSent: [...WHATS_SENT.always],
      sentWhenAllowed: [...WHATS_SENT.whenAllowed],
      neverSent: [...WHATS_SENT.never],
      whoCanUse: [
        "The workspace owner",
        "Managers, when the owner allows it",
        "Members, on their own data only, when the owner allows it",
      ],
    },
  };
}

/** This month's AI usage in a demo: the demo status's quota (the owner sees workspace totals and cost). */
export function demoAiUsage(role: Role): AiUsageSummaryDto {
  const quota = demoAiStatus(role).quota;
  const monthStart = new Date(Date.UTC(NOW.getUTCFullYear(), NOW.getUTCMonth(), 1)).toISOString();
  const resetsAt = new Date(Date.UTC(NOW.getUTCFullYear(), NOW.getUTCMonth() + 1, 1)).toISOString();
  return {
    monthStart,
    resetsAt,
    requestsUsed: quota?.requestsUsed ?? 3,
    requestsLimit: quota?.requestsLimit ?? null,
    tokensUsed: quota?.tokensUsed ?? 4_210,
    tokensLimit: quota?.tokensLimit ?? null,
    estimatedCostUsd: role === "owner" ? 0 : null,
    byFeature:
      role === "owner"
        ? [
            { featureId: "period-summary", requests: 18, tokens: 27_300 },
            { featureId: "member-day-summary", requests: 14, tokens: 19_880 },
            { featureId: "report-explanation", requests: 10, tokens: 14_300 },
          ]
        : null,
    myRequestsToday: 2,
    myDailyLimit: 50,
  };
}
