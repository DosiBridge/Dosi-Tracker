import { ApiError, getApi, postApi, putApi } from "./useApi";
import {
  AI_ROUTES,
  aiFeatureRoute,
  type AiAnswerDto,
  type AiAskInput,
  type AiFeatureInputs,
  type AiFeatureResults,
  type AiFeedbackInput,
  type AiInsightFeatureId,
  type AiStatusDto,
  type AiUsageSummaryDto,
  type AiWorkspaceSettingsDto,
  type PlatformAiOverviewDto,
  type PlatformAiStatusDto,
  type UpdateAiWorkspaceSettingsDto,
} from "@/lib/ai-contract";

/* ============================================================================
 * AI assist calls (contract in lib/ai-contract). Every one goes to the ABP
 * backend through useApi's `send()` — the session's bearer token and
 * X-Workspace-Id — and throws ApiError on a refusal; lib/ai-assist's
 * aiErrorState turns that into what the card says. Nothing here talks to an
 * AI provider: the browser never holds a provider connection.
 * ========================================================================== */

/**
 * The caller's AI status in the current workspace; null when the server has
 * no AI assist (an older server: 404 or 405), so every AI surface stays hidden.
 */
export const getAiStatus = async (init: { signal?: AbortSignal } = {}): Promise<AiStatusDto | null> => {
  try {
    return (await getApi(AI_ROUTES.status, init)) as AiStatusDto;
  } catch (err) {
    if (err instanceof ApiError && (err.status === 404 || err.status === 405)) return null;
    throw err;
  }
};

/** Owner and manager: the workspace's AI settings and the server's disclosure. */
export const getAiSettings = (): Promise<AiWorkspaceSettingsDto> => getApi(AI_ROUTES.settings);

/** Owner: change the AI settings (400 Tracker:AiDisclosureNotAccepted without the current acknowledgement). */
export const updateAiSettings = (input: UpdateAiWorkspaceSettingsDto): Promise<AiWorkspaceSettingsDto> => putApi(AI_ROUTES.settings, input);

/** This month's AI usage: workspace totals for the owner, the caller's own otherwise. */
export const getAiUsage = (): Promise<AiUsageSummaryDto> => getApi(AI_ROUTES.usage);

/** Run one insight feature. Pass a `signal` so Cancel (or leaving the page) abandons the request. */
export const requestAiFeature = <F extends AiInsightFeatureId>(
  featureId: F,
  body: AiFeatureInputs[F],
  init: { signal?: AbortSignal } = {},
): Promise<AiFeatureResults[F]> => postApi(aiFeatureRoute(featureId), body, init) as Promise<AiFeatureResults[F]>;

/** One Ask AI turn. */
export const askAi = (input: AiAskInput, init: { signal?: AbortSignal } = {}): Promise<AiAnswerDto> =>
  postApi(AI_ROUTES.ask, input, init) as Promise<AiAnswerDto>;

/** Helpful / Not helpful on a result the caller received (204). */
export const sendAiFeedback = async (input: AiFeedbackInput): Promise<void> => {
  await postApi(AI_ROUTES.feedback, input);
};

/** Platform admin: provider state, month usage and cost, top workspaces. */
export const getPlatformAiOverview = (): Promise<PlatformAiOverviewDto> => getApi(AI_ROUTES.platformOverview);

/** Platform admin: the runtime switch (takes effect on the next request). */
export const updatePlatformAiStatus = (input: PlatformAiStatusDto): Promise<PlatformAiStatusDto> => putApi(AI_ROUTES.platformStatus, input);
