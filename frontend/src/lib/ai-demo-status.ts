import {
  AI_FEATURE_IDS,
  type AiFeatureStatusDto,
  type AiInsightFeatureId,
  type AiScope,
  type AiStatusDto,
} from "./ai-contract";
import { NOW } from "./demo-clock";
import type { Role } from "./types";

/* ============================================================================
 * The AI status a demo session reads: what the server would say to each role
 * in a workspace that opted in. Kept apart from lib/ai-demo (the sample
 * results), which the app loads only on demand: every session reads a status,
 * only a demo session ever needs the sample generators.
 * ========================================================================== */

/** The demo's own disclosure version (a server sends "{code}.{operator}"). */
export const DEMO_DISCLOSURE_VERSION = "1.0";

/** The processor a demo names: none, since nothing leaves the browser. */
export const DEMO_PROVIDER = "Demo (no AI provider)";

const FEATURE_SCOPES: Partial<Record<AiInsightFeatureId, { team: AiScope[]; self: AiScope[] }>> = {
  "period-summary": { team: ["team", "self"], self: ["self"] },
  "member-day-summary": { team: ["member", "self"], self: ["self"] },
  "data-quality-review": { team: ["team", "self"], self: ["self"] },
};

/** What the server's status would say to each role in an opted-in workspace (§4.2). */
export function demoAiStatus(role: Role): AiStatusDto {
  const manager = role === "owner" || role === "admin";
  const tracked = manager || role === "worker";
  const features: AiFeatureStatusDto[] = AI_FEATURE_IDS.map((featureId) => {
    const allowed = tracked && (manager || featureId !== "report-explanation");
    const scopes = featureId === "ask" ? undefined : FEATURE_SCOPES[featureId];
    return {
      featureId,
      allowed,
      reason: allowed ? null : "role",
      scopes: allowed && scopes ? (manager ? scopes.team : scopes.self) : [],
    };
  });
  return {
    available: tracked,
    reason: tracked ? null : "role-not-allowed",
    workspaceEnabled: true,
    canManage: role === "owner",
    providerDisplayName: DEMO_PROVIDER,
    disclosureVersion: DEMO_DISCLOSURE_VERSION,
    allowWindowTitles: false,
    allowNotes: false,
    features,
    quota: manager
      ? {
          requestsUsed: 42,
          requestsLimit: 2000,
          tokensUsed: 61_480,
          tokensLimit: 4_000_000,
          resetsAt: new Date(Date.UTC(NOW.getUTCFullYear(), NOW.getUTCMonth() + 1, 1)).toISOString(),
        }
      : null,
  };
}
