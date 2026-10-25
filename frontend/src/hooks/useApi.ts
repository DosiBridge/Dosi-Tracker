import { useState, useEffect, useCallback, useRef } from 'react';
import {
  REFRESH_TOKEN_STORAGE_KEY,
  SESSION_UNAUTHORIZED_EVENT,
  TOKEN_EXPIRES_STORAGE_KEY,
  TOKEN_STORAGE_KEY,
  clearCurrentWorkspace,
  currentWorkspaceId,
  isLiveSession,
  readStorage,
  redirectToWorkspaceChooser,
  removeStorage,
  writeStorage,
} from '@/lib/live-session';
import { API_BASE_URL, WEB_SIGN_IN_CLIENT, tokenEndpoint, type WorkspaceMembership } from '@/lib/sign-in';
import {
  API_ROUTES,
  TEAM_ROUTES,
  activateSubscriptionRoute,
  activityNoteRoute,
  acceptInvitationRoute,
  changeWorkspacePlanRoute,
  invitationRoute,
  leaveWorkspaceRoute,
  archiveWorkspaceRoute,
  restoreWorkspaceRoute,
  revokeInvitationRoute,
  TEAM_INVITATIONS_ROUTE,
  WORKSPACE_ROUTES,
  WORKSPACE_SUBSCRIPTIONS_ROUTE,
  workspaceRoleRoute,
  collectPages,
  markInvoicePaidRoute,
  parseApiErrorBody,
  parseRetryAfter,
  projectMembersRoute,
  projectRoute,
  PROJECT_TASK_ROUTES,
  projectTaskRoute,
  projectTasksRoute,
  removeFromWorkspaceRoute,
  removeMemberRoute,
  reportingRoute,
  suspendSubscriptionRoute,
  toPagedResult,
  updateMemberRoute,
  type ActivityDto,
  type ApiErrorInfo,
  type CreateProjectTaskDto,
  type GetProjectTasksQuery,
  type ProjectTaskDto,
  type UpdateProjectTaskDto,
  type ChangePasswordInput,
  type CollectedPages,
  type CollectPagesOptions,
  type CreateProjectMemberDto,
  type CreateUpdateProjectDto,
  type DeviceReportRowDto,
  type HourlyReportRowDto,
  type InviteMemberDto,
  type InviteMemberResultDto,
  type MyPreferencesDto,
  type PagedResultDto,
  type PayrollReportDto,
  type PresenceDto,
  type ProjectDto,
  type ProjectMemberDto,
  type ReportingQuery,
  type AcceptInvitationResultDto,
  type InvitationInfoDto,
  type RegisterWorkspaceInput,
  type TeamMemberDto,
  type WorkspaceInvitationDto,
  type WorkspaceRegistrationResultDto,
  type WorkspaceSubscriptionDto,
  type UpdateProjectMemberDto,
  type WorkspaceSettingsDto,
} from '@/lib/api-contract';

export { isLiveSession };

// The storage keys, the API host and the sign-in client come from one place
// each (lib/live-session, lib/sign-in): SessionProvider watches the same keys,
// and the change-at-sign-in exchange talks to the same token endpoint.

/**
 * The request never reached the server (offline, DNS, TLS, server down) — as
 * opposed to the server answering with a rejection. Callers must tell users
 * these apart: "check your password" is wrong and misleading when the real
 * problem is that nothing is listening.
 */
export class ApiUnreachableError extends Error {
  constructor() {
    super("Couldn't reach the server");
    this.name = "ApiUnreachableError";
  }
}

/**
 * The server answered with an error status. `message` keeps the shape callers
 * already show ("API PUT Error: Forbidden: You can't change your own rate."),
 * and the fields carry what ABP said, so a page can branch on `code`
 * ("Tracker:CannotModifySelf") or list `validationErrors` instead of parsing
 * the text.
 */
export class ApiError extends Error {
  readonly status: number;
  /** ABP `error.code`, e.g. "Tracker:SeatLimitReached"; null when the body had none. */
  readonly code: string | null;
  /** ABP `error.message`: already written for people. */
  readonly serverMessage: string | null;
  readonly details: string | null;
  readonly validationErrors: ApiErrorInfo['validationErrors'];
  /** Seconds the server asked to wait before retrying (Retry-After), when it said. */
  readonly retryAfterSeconds: number | null;

  constructor(message: string, status: number, info: ApiErrorInfo, retryAfterSeconds: number | null = null) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = info.code;
    this.serverMessage = info.message;
    this.details = info.details;
    this.validationErrors = info.validationErrors;
    this.retryAfterSeconds = retryAfterSeconds;
  }
}

/** Sign-in refused: the email, password or workspace didn't match (OAuth `invalid_grant`, or another 4xx). */
export class InvalidCredentialsError extends Error {
  readonly status: number;
  /** The OAuth `error` ("invalid_grant"), when the server sent one. */
  readonly error: string | null;
  readonly errorDescription: string | null;

  constructor(status: number, error: string | null, errorDescription: string | null) {
    super('Invalid credentials or login failed');
    this.name = "InvalidCredentialsError";
    this.status = status;
    this.error = error;
    this.errorDescription = errorDescription;
  }
}

/** Too many sign-in attempts (429). Nothing is wrong with the credentials: wait and try again. */
export class RateLimitedError extends Error {
  /** Seconds to wait, when the server said (Retry-After). */
  readonly retryAfterSeconds: number | null;

  constructor(retryAfterSeconds: number | null) {
    super(
      retryAfterSeconds
        ? `Too many sign-in attempts. Try again in ${retryAfterSeconds} seconds.`
        : 'Too many sign-in attempts. Try again in a moment.',
    );
    this.name = "RateLimitedError";
    this.retryAfterSeconds = retryAfterSeconds;
  }
}

/** The server failed while signing in (5xx): an outage, not the user's credentials. */
export class SignInServerError extends Error {
  readonly status: number;
  readonly errorDescription: string | null;

  constructor(status: number, errorDescription: string | null) {
    super(
      errorDescription
        ? `The server couldn't sign you in: ${errorDescription}`
        : `The server couldn't sign you in (HTTP ${status}). Try again shortly.`,
    );
    this.name = "SignInServerError";
    this.status = status;
    this.errorDescription = errorDescription;
  }
}

/** The browser refuses to store the session (site data blocked), so a sign-in can't last. */
export class StorageUnavailableError extends Error {
  constructor() {
    super("This browser is blocking site data, so you can't stay signed in. Allow cookies and site data for this site, then try again.");
    this.name = "StorageUnavailableError";
  }
}

/*
 * Web storage can be blocked (privacy settings, sandboxed frames): through
 * lib/live-session's wrappers reads then see nothing and nothing throws. A
 * sign-in is the exception — it can't last without storage, so its writes
 * must land (`keep`), or it fails with StorageUnavailableError.
 */

const keep = (key: string, value: string) => {
  if (!writeStorage(key, value)) throw new StorageUnavailableError();
};

/* ----------------------------- Session tokens ----------------------------- */
// The web signs in with `offline_access`, so the token response carries a
// refresh token next to the one-hour access token. A request made when the
// access token is about to expire renews it first, and a 401 renews it once
// and retries — a working session never ends mid-task. Only a refused renewal
// (refresh token expired or revoked) signs the user out.

/** Renew this long before the access token expires. */
const RENEW_AHEAD_MS = 60_000;
/** How long the route gate's (src/proxy.ts) session flag lasts while a refresh token can renew the session (OpenIddict's default refresh-token lifetime). */
const RENEWABLE_SESSION_SECONDS = 14 * 24 * 60 * 60;

interface TokenResponse {
  access_token?: unknown;
  expires_in?: unknown;
  refresh_token?: unknown;
}

/**
 * Keep a token response: the access token and when it expires, the refresh
 * token (a renewal that brings none keeps the current one), and the session
 * flag the route gate (src/proxy.ts) reads. Throws when the browser won't store anything.
 */
const storeTokens = (json: TokenResponse & { access_token: string }, renewal: boolean) => {
  const lifetime = typeof json.expires_in === 'number' && json.expires_in > 0 ? json.expires_in : 3600;
  keep(TOKEN_STORAGE_KEY, json.access_token);
  keep(TOKEN_EXPIRES_STORAGE_KEY, String(Date.now() + lifetime * 1000));
  if (typeof json.refresh_token === 'string' && json.refresh_token) {
    keep(REFRESH_TOKEN_STORAGE_KEY, json.refresh_token);
  } else if (!renewal) {
    removeStorage(REFRESH_TOKEN_STORAGE_KEY);
  }
  const renewable = !!readStorage(REFRESH_TOKEN_STORAGE_KEY);
  const secure = window.location.protocol === 'https:' ? '; Secure' : '';
  document.cookie = `dosi-token=1; Path=/; Max-Age=${renewable ? RENEWABLE_SESSION_SECONDS : lifetime}; SameSite=Lax${secure}`;
};

/** Whether the access token expires within RENEW_AHEAD_MS (unknown expiry: no). */
const tokenDue = (): boolean => {
  const at = Number(readStorage(TOKEN_EXPIRES_STORAGE_KEY));
  return Number.isFinite(at) && at > 0 && at - Date.now() <= RENEW_AHEAD_MS;
};

/** Trade the refresh token for a new access token; false when there is none or it was refused. */
const exchangeRefreshToken = async (): Promise<boolean> => {
  const refreshToken = readStorage(REFRESH_TOKEN_STORAGE_KEY);
  if (!refreshToken) return false;
  const params = new URLSearchParams();
  params.append('grant_type', 'refresh_token');
  params.append('refresh_token', refreshToken);
  params.append('client_id', WEB_SIGN_IN_CLIENT.clientId);
  let res: Response;
  try {
    res = await fetch(tokenEndpoint(), {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: params.toString(),
    });
  } catch {
    return false; // Unreachable: nothing renewed; the request that asked fails on its own terms.
  }
  if (!res.ok) {
    // Refused (expired, revoked or already used): it will never renew anything again.
    if (res.status === 400 || res.status === 401) removeStorage(REFRESH_TOKEN_STORAGE_KEY);
    return false;
  }
  let json: TokenResponse;
  try {
    json = (await res.json()) as TokenResponse;
  } catch {
    return false;
  }
  if (typeof json?.access_token !== 'string' || !json.access_token) return false;
  try {
    storeTokens({ ...json, access_token: json.access_token }, true);
  } catch {
    return false;
  }
  return true;
};

let renewal: Promise<boolean> | null = null;

/**
 * Renew the access token with the refresh token: true once a fresh token is
 * stored. One renewal at a time per tab; across tabs a Web Lock (where the
 * browser has them) lets a tab that waited reuse the token another just got
 * rather than spending the refresh token twice.
 */
export const refreshAccessToken = (): Promise<boolean> => {
  if (!renewal) {
    const startedWith = readStorage(TOKEN_STORAGE_KEY);
    const run = async () => {
      const current = readStorage(TOKEN_STORAGE_KEY);
      if (current && current !== startedWith && !tokenDue()) return true;
      return exchangeRefreshToken();
    };
    const locks = typeof navigator !== 'undefined' ? navigator.locks : undefined;
    const renew = async (): Promise<boolean> => (locks ? await locks.request('dosi-token-renewal', run) : run());
    renewal = renew().finally(() => {
      renewal = null;
    });
  }
  return renewal;
};

/** Routes that are not scoped to a workspace: they never carry X-Workspace-Id. */
const UNSCOPED_PREFIXES = [
  '/connect/token',
  WORKSPACE_ROUTES.create, // also covers /mine and /{id}/leave
  WORKSPACE_ROUTES.register,
  '/api/app/invitations',
  '/api/account/my-profile',
];

/** The X-Workspace-Id for a request: the current workspace, except on unscoped routes. */
export const workspaceHeaderFor = (endpoint: string): string | null => {
  const path = endpoint.split('?')[0];
  if (UNSCOPED_PREFIXES.some((p) => path.startsWith(p))) return null;
  return currentWorkspaceId();
};

const headersFor = (token: string | null, json: boolean, workspaceId: string | null): Record<string, string> => ({
  ...(json ? { 'Content-Type': 'application/json' } : {}),
  ...(token ? { Authorization: `Bearer ${token}` } : {}),
  ...(workspaceId ? { 'X-Workspace-Id': workspaceId } : {}),
});

/**
 * One API request with the session's token: renewed first when it is about
 * to expire, and once more (then retried) when the server answers 401.
 */
const send = async (
  endpoint: string,
  init: { method?: string; body?: string; signal?: AbortSignal; json?: boolean } = {},
): Promise<Response> => {
  if (readStorage(REFRESH_TOKEN_STORAGE_KEY) && readStorage(TOKEN_STORAGE_KEY) && tokenDue()) {
    await refreshAccessToken();
  }
  const attempt = (token: string | null) =>
    fetch(`${API_BASE_URL}${endpoint}`, {
      method: init.method,
      body: init.body,
      signal: init.signal,
      headers: headersFor(token, init.json !== false, workspaceHeaderFor(endpoint)),
    });
  const sentWith = readStorage(TOKEN_STORAGE_KEY);
  const res = await attempt(sentWith);
  if (res.status !== 401 || !sentWith) return res;
  // Another tab may already hold a newer token; otherwise renew this one.
  const current = readStorage(TOKEN_STORAGE_KEY);
  const renewed = (current !== null && current !== sentWith) || (await refreshAccessToken());
  if (!renewed || init.signal?.aborted) return res;
  return attempt(readStorage(TOKEN_STORAGE_KEY));
};

/**
 * A 401 that renewal couldn't cure: clear the session and send the user back
 * to /login. While a refresh token is still held it was never refused — the
 * renewal failed for now (offline, rate-limited, server error) and the next
 * request tries again — so the session is kept.
 *
 * First the page is asked (a cancelable SESSION_UNAUTHORIZED_EVENT): an open
 * live page (SessionProvider) cancels it and asks for the password in place,
 * so whatever is being edited survives; only when nobody does is the user
 * signed out and sent to /login.
 */
const handleUnauthorized = (res: Response) => {
  if (res.status === 401 && typeof window !== 'undefined' && !readStorage(REFRESH_TOKEN_STORAGE_KEY)) {
    const keptInPlace = !window.dispatchEvent(new Event(SESSION_UNAUTHORIZED_EVENT, { cancelable: true }));
    if (keptInPlace) return;
    logoutApi();
    if (!window.location.pathname.startsWith('/login')) {
      // eslint-disable-next-line @next/next/no-location-assign-relative-destination -- a full navigation drops every in-memory copy of the ended session's data
      window.location.href = '/login';
    }
  }
};

/**
 * The current workspace was refused (no longer a member, or none chosen): forget
 * it and go choose another.
 */
const handleWorkspaceRefused = (status: number, code: string | null) => {
  const refused = (status === 403 && code === 'Tracker:NotWorkspaceMember') || (status === 400 && code === 'Tracker:WorkspaceRequired');
  if (!refused || typeof window === 'undefined') return;
  clearCurrentWorkspace();
  redirectToWorkspaceChooser();
};

const headerOf = (res: Response, name: string): string | null =>
  typeof res.headers?.get === 'function' ? res.headers.get(name) : null;

const bodyText = async (res: Response): Promise<string> => {
  try {
    return await res.text();
  } catch {
    return '';
  }
};

/** A failed response as an ApiError ("<prefix>: <status text>[: <server message>]"); a 401 also ends the session. */
const apiFailure = async (prefix: string, res: Response): Promise<ApiError> => {
  handleUnauthorized(res);
  const info = parseApiErrorBody(await bodyText(res));
  handleWorkspaceRefused(res.status, info.code);
  const statusText = res.statusText || `HTTP ${res.status}`;
  return new ApiError(
    `${prefix}: ${statusText}${info.message ? `: ${info.message}` : ''}`,
    res.status,
    info,
    parseRetryAfter(headerOf(res, 'Retry-After')),
  );
};

interface ApiState<T> {
  data: T | null;
  error: Error | null;
  isLoading: boolean;
}

export interface UseApiOptions {
  /**
   * Skip the request entirely (default true). Pass `enabled: isLive` so a
   * demo session never touches the network.
   */
  enabled?: boolean;
}

/**
 * Declarative GET. Mirrors getApi: a 401 the session can't renew ends it
 * (handleUnauthorized), `items` is unwrapped. The in-flight request is aborted
 * when the component unmounts or the endpoint changes, so a late response can
 * never set state on an unmounted tree or overwrite a newer request's result.
 */
export function useApi<T>(endpoint: string, options: UseApiOptions = {}) {
  const enabled = options.enabled ?? true;
  const [state, setState] = useState<ApiState<T>>({
    data: null,
    error: null,
    isLoading: enabled,
  });
  const controllerRef = useRef<AbortController | null>(null);

  const fetchData = useCallback(async () => {
    if (!enabled) {
      setState({ data: null, error: null, isLoading: false });
      return;
    }
    controllerRef.current?.abort();
    const controller = new AbortController();
    controllerRef.current = controller;
    setState(prev => ({ ...prev, isLoading: true, error: null }));
    try {
      const res = await send(endpoint, { signal: controller.signal });
      if (!res.ok) {
        throw await apiFailure('API Error', res);
      }
      const json = await res.json();
      if (controller.signal.aborted) return;
      setState({ data: json?.items || json, error: null, isLoading: false });
    } catch (err) {
      if (controller.signal.aborted) return; // superseded or unmounted: stay silent
      setState({
        data: null,
        error: err instanceof Error ? err : new Error(String(err)),
        isLoading: false,
      });
    }
  }, [endpoint, enabled]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void fetchData();
    return () => controllerRef.current?.abort();
  }, [fetchData]);

  return { ...state, refetch: fetchData };
}

/** GET; ABP's `items` envelope is unwrapped. Pass a `signal` to abandon the request. */
export const getApi = async (endpoint: string, init: { signal?: AbortSignal } = {}) => {
  const res = await send(endpoint, { signal: init.signal });
  if (!res.ok) {
    throw await apiFailure('API GET Error', res);
  }
  const json = await res.json();
  return json.items || json;
};

/**
 * GET one page of a paged list, KEEPING the server's `totalCount` (getApi
 * unwraps `items` and loses it). A bare array comes back as one full page.
 */
export const getPagedApi = async <T,>(endpoint: string, init: { signal?: AbortSignal } = {}): Promise<PagedResultDto<T>> => {
  const res = await send(endpoint, { signal: init.signal });
  if (!res.ok) {
    throw await apiFailure('API GET Error', res);
  }
  return toPagedResult<T>(await res.json());
};

/**
 * GET every page of a paged list (SkipCount stepping, rows de-duplicated by
 * id) up to `maxItems`. `truncated` says when more rows exist than were read:
 * show "showing N of M", never a silently partial total.
 */
export const getAllPagesApi = <T,>(
  endpoint: string,
  options: CollectPagesOptions<T> & { signal?: AbortSignal } = {},
): Promise<CollectedPages<T>> => {
  const { signal, ...paging } = options;
  return collectPages<T>((url) => getPagedApi<T>(url, { signal }), endpoint, paging);
};

/**
 * Fetch a protected binary (e.g. screenshot content) with the bearer token and
 * return an object URL for <img src>. Callers should revoke it when done.
 * Pass a `signal` to abandon the download (e.g. its card unmounted).
 */
export const getAuthedBlobUrl = async (endpoint: string, init: { signal?: AbortSignal } = {}): Promise<string> =>
  (await getAuthedBlob(endpoint, init)).url;

/**
 * getAuthedBlobUrl with the content type the server sent ("image/jpeg",
 * "image/png"), so a download can be named for what it is — agents send JPEG
 * captures or PNG (contract §1). Revoke `url` when done.
 */
export const getAuthedBlob = async (
  endpoint: string,
  init: { signal?: AbortSignal } = {},
): Promise<{ url: string; type: string }> => {
  const res = await send(endpoint, { signal: init.signal, json: false });
  if (!res.ok) {
    throw await apiFailure('API blob Error', res);
  }
  const blob = await res.blob();
  return { url: URL.createObjectURL(blob), type: blob.type || headerOf(res, 'Content-Type') || '' };
};

export const delApi = async (endpoint: string) => {
  const res = await send(endpoint, { method: 'DELETE' });
  if (!res.ok) {
    throw await apiFailure('API DELETE Error', res);
  }
};

export const postApi = async (endpoint: string, body: unknown) => {
  const res = await send(endpoint, { method: 'POST', body: JSON.stringify(body) });
  if (!res.ok) {
    throw await apiFailure('API POST Error', res);
  }
  const text = await res.text();
  return text ? JSON.parse(text) : null; // 204-safe
};

export const putApi = async (endpoint: string, body: unknown) => {
  const res = await send(endpoint, { method: 'PUT', body: JSON.stringify(body) });
  if (!res.ok) {
    throw await apiFailure('API PUT Error', res);
  }
  const text = await res.text();
  return text ? JSON.parse(text) : null; // 204-safe
};

/**
 * OpenIddict resource-owner password flow. Accounts are global: the token
 * names no workspace; requests choose one with X-Workspace-Id.
 *
 * The token is mirrored into a cookie so the Next.js proxy (src/proxy.ts) can
 * gate routes server-side (localStorage is invisible to it). With
 * `offline_access` the response also carries a refresh token, which renews
 * the one-hour access token as it runs out (see `send`).
 *
 * Failures are told apart: InvalidCredentialsError (the details didn't
 * match), RateLimitedError (429 — wait, don't reset anything),
 * SignInServerError (5xx), ApiUnreachableError (no server) and
 * StorageUnavailableError (the browser won't keep the session).
 */
export const loginApi = async (username: string, password: string): Promise<string> => {
  const params = new URLSearchParams();
  params.append('grant_type', 'password');
  params.append('username', username);
  params.append('password', password);
  params.append('client_id', WEB_SIGN_IN_CLIENT.clientId);
  params.append('scope', WEB_SIGN_IN_CLIENT.scope);
  let res: Response;
  try {
    res = await fetch(tokenEndpoint(), {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: params.toString(),
    });
  } catch {
    throw new ApiUnreachableError();
  }

  if (!res.ok) {
    const info = parseApiErrorBody(await bodyText(res));
    if (res.status === 429) throw new RateLimitedError(parseRetryAfter(headerOf(res, 'Retry-After')));
    if (res.status >= 500) throw new SignInServerError(res.status, info.message);
    throw new InvalidCredentialsError(res.status, info.code, info.message);
  }

  const json = (await res.json()) as TokenResponse;
  const token = json.access_token;
  if (typeof window !== 'undefined' && typeof token === 'string' && token) {
    try {
      storeTokens({ ...json, access_token: token }, false);
    } catch {
      removeStorage(TOKEN_STORAGE_KEY);
      removeStorage(TOKEN_EXPIRES_STORAGE_KEY);
      removeStorage(REFRESH_TOKEN_STORAGE_KEY);
      throw new StorageUnavailableError();
    }
  }
  return token as string;
};

export const logoutApi = () => {
  if (typeof window !== 'undefined') {
    removeStorage(TOKEN_STORAGE_KEY);
    removeStorage(TOKEN_EXPIRES_STORAGE_KEY);
    removeStorage(REFRESH_TOKEN_STORAGE_KEY);
    // The current workspace is kept: the next sign-in reopens it when still a member.
    document.cookie = 'dosi-token=; Path=/; Max-Age=0; SameSite=Lax';
  }
};

/* ----------------------------- Workspaces ----------------------------- */

const listOf = <T,>(json: unknown): T[] => {
  if (Array.isArray(json)) return json as T[];
  const items = (json as { items?: unknown } | null)?.items;
  return Array.isArray(items) ? (items as T[]) : [];
};

/** The signed-in person's workspaces, with their role in each. */
export const getMyWorkspacesApi = async (): Promise<WorkspaceMembership[]> => listOf<WorkspaceMembership>(await getApi(WORKSPACE_ROUTES.mine));

/** Create a workspace owned by the signed-in person. */
export const createWorkspaceApi = (name: string, planId?: string | null): Promise<{ id: string; name: string }> =>
  postApi(WORKSPACE_ROUTES.create, planId ? { name, planId } : { name });

/** Leave a workspace (409 Tracker:LastOwnerCannotLeave for its last owner). */
export const leaveWorkspaceApi = async (workspaceId: string): Promise<void> => {
  await postApi(leaveWorkspaceRoute(workspaceId), {});
};

/** Archive a workspace (owner only). */
export const archiveWorkspaceApi = async (workspaceId: string): Promise<void> => {
  await postApi(archiveWorkspaceRoute(workspaceId), {});
};

/** Restore an archived workspace (owner only). */
export const restoreWorkspaceApi = async (workspaceId: string): Promise<void> => {
  await postApi(restoreWorkspaceRoute(workspaceId), {});
};

/** Anonymous JSON call: never a token, never a workspace. */
const anonymous = async <T,>(endpoint: string, init: { method?: string; body?: unknown } = {}): Promise<T> => {
  let res: Response;
  try {
    res = await fetch(`${API_BASE_URL}${endpoint}`, {
      method: init.method ?? 'GET',
      headers: { 'Content-Type': 'application/json' },
      credentials: 'omit',
      body: init.body === undefined ? undefined : JSON.stringify(init.body),
    });
  } catch {
    throw new ApiUnreachableError();
  }
  if (!res.ok) {
    const info = parseApiErrorBody(await bodyText(res));
    throw new ApiError(`API Error: ${res.statusText || `HTTP ${res.status}`}${info.message ? `: ${info.message}` : ''}`, res.status, info);
  }
  const text = await bodyText(res);
  return (text ? JSON.parse(text) : null) as T;
};

/**
 * Self-service sign-up: a new account and (unless joining by invitation) its
 * first workspace. 409 Tracker:EmailAlreadyRegistered / DuplicateWorkspaceName.
 */
export const registerWorkspaceApi = (input: RegisterWorkspaceInput): Promise<WorkspaceRegistrationResultDto> =>
  anonymous(WORKSPACE_ROUTES.register, { method: 'POST', body: input });

/** What an invitation link opens (anonymous). */
export const getInvitationApi = (token: string): Promise<InvitationInfoDto> => anonymous(invitationRoute(token));

/** Accept an invitation as the signed-in person. */
export const acceptInvitationApi = (token: string): Promise<AcceptInvitationResultDto> => postApi(acceptInvitationRoute(token), {});

/* ----------------------------- Contract v2 endpoints ----------------------------- */
// Typed wrappers over the helpers above (routes from lib/api-contract). Each
// throws ApiError on a failed response; read `code` for the business rule.

/** Who is working now: every member device for Activities.ViewAll holders, otherwise your own. */
export const getTeamPresence = async (): Promise<PresenceDto[]> => {
  const items = await getApi(API_ROUTES.presenceTeam);
  return Array.isArray(items) ? (items as PresenceDto[]) : [];
};

/** Set (or, with null, clear) a block's note; the server trims and tidies it. */
export const updateActivityNote = (activityId: string, description: string | null): Promise<ActivityDto> =>
  putApi(activityNoteRoute(activityId), { description });

/** Edit a project's name, colour and tracking settings. */
export const updateProject = (projectId: string, input: CreateUpdateProjectDto): Promise<ProjectDto> =>
  putApi(projectRoute(projectId), input);

/** A project's memberships (role, rate, and the member's names on newer servers). */
export const getProjectMembers = async (projectId: string): Promise<ProjectMemberDto[]> => {
  const items = await getApi(projectMembersRoute(projectId));
  return Array.isArray(items) ? (items as ProjectMemberDto[]) : [];
};

/** Add an existing workspace user to a project (404 Tracker:InvalidUser when they aren't in the workspace). */
export const addProjectMember = (input: CreateProjectMemberDto): Promise<ProjectMemberDto> => postApi(TEAM_ROUTES.member, input);

/** Invite someone by email: an existing account is added; anyone else gets an invitation link. */
export const inviteMember = (input: InviteMemberDto): Promise<InviteMemberResultDto> => postApi(TEAM_ROUTES.inviteMember, input);

/** Change a member's role and/or rate on one project (see membershipEditRules for who may). */
export const updateProjectMember = (projectId: string, userId: string, input: UpdateProjectMemberDto): Promise<ProjectMemberDto> =>
  putApi(updateMemberRoute(projectId, userId), input);

/** Remove one project membership, by its id from getProjectMembers. */
export const removeProjectMember = (membershipId: string): Promise<void> => delApi(removeMemberRoute(membershipId));

/** Owner: change a member's workspace role (owner | manager | member). */
export const changeWorkspaceRole = (userId: string, role: string): Promise<TeamMemberDto> => putApi(workspaceRoleRoute(userId, role), {});

/** Pending invitations of the current workspace. */
export const getInvitations = async (): Promise<WorkspaceInvitationDto[]> => listOf<WorkspaceInvitationDto>(await getApi(TEAM_INVITATIONS_ROUTE));

/** Revoke a pending invitation. */
export const revokeInvitation = (invitationId: string): Promise<void> => delApi(revokeInvitationRoute(invitationId));

/** End someone's membership of this workspace (their account stays). */
export const removeMemberFromWorkspace = (userId: string): Promise<void> => delApi(removeFromWorkspaceRoute(userId));

/** Change the signed-in user's own password. */
export const changeMyPassword = async (input: ChangePasswordInput): Promise<void> => {
  await postApi(API_ROUTES.changePassword, input);
};

/** Workspace settings (owners): default time zone and what new projects start with. */
export const getWorkspaceSettings = (): Promise<WorkspaceSettingsDto> => getApi(API_ROUTES.workspaceSettings);

export const updateWorkspaceSettings = (settings: WorkspaceSettingsDto): Promise<WorkspaceSettingsDto> =>
  putApi(API_ROUTES.workspaceSettings, settings);

/** The signed-in user's own preferences (their reporting time zone). */
export const getMyPreferences = (): Promise<MyPreferencesDto> => getApi(API_ROUTES.myPreferences);

export const updateMyPreferences = (preferences: MyPreferencesDto): Promise<MyPreferencesDto> =>
  putApi(API_ROUTES.myPreferences, preferences);

/** Platform: move a workspace to another plan. */
export const changeWorkspacePlan = async (workspaceId: string, planId: string): Promise<void> => {
  await postApi(changeWorkspacePlanRoute(workspaceId, planId), {});
};

/** Platform: mark a workspace's subscription active (e.g. after payment). */
export const activateSubscription = async (workspaceId: string): Promise<void> => {
  await postApi(activateSubscriptionRoute(workspaceId), {});
};

/** Platform: suspend a workspace's subscription (it can't record time until activated again). */
export const suspendSubscription = async (workspaceId: string): Promise<void> => {
  await postApi(suspendSubscriptionRoute(workspaceId), {});
};

/**
 * Platform: every workspace's plan, status and seats; null when the server has
 * no such endpoint (404), so the console can say "—".
 */
export const getWorkspaceSubscriptions = async (): Promise<WorkspaceSubscriptionDto[] | null> => {
  try {
    const items = await getApi(WORKSPACE_SUBSCRIPTIONS_ROUTE);
    return Array.isArray(items) ? (items as WorkspaceSubscriptionDto[]) : [];
  } catch (err) {
    if (err instanceof ApiError && (err.status === 404 || err.status === 405)) return null;
    throw err;
  }
};

/** Host: record an invoice as paid. */
export const markInvoicePaid = async (invoiceId: string): Promise<void> => {
  await postApi(markInvoicePaidRoute(invoiceId), {});
};

/** Pay per member and project, each block priced at the rate in effect when it started. */
export const getPayrollReport = (query: ReportingQuery): Promise<PayrollReportDto> =>
  getApi(reportingRoute('payroll', query));

/** Tracked and active minutes per weekday and hour of the reporting zone. */
export const getHourlyReport = async (query: ReportingQuery): Promise<HourlyReportRowDto[]> => {
  const rows = await getApi(reportingRoute('hourly', query));
  return Array.isArray(rows) ? (rows as HourlyReportRowDto[]) : [];
};

/** Tracked minutes per member device over the range. */
export const getDeviceReport = async (query: ReportingQuery): Promise<DeviceReportRowDto[]> => {
  const rows = await getApi(reportingRoute('devices', query));
  return Array.isArray(rows) ? (rows as DeviceReportRowDto[]) : [];
};

/** Project Tasks API */
export const getProjectTasksApi = (query: GetProjectTasksQuery): Promise<PagedResultDto<ProjectTaskDto>> =>
  getPagedApi<ProjectTaskDto>(projectTasksRoute(query));

export const createProjectTaskApi = (input: CreateProjectTaskDto): Promise<ProjectTaskDto> =>
  postApi(PROJECT_TASK_ROUTES.collection, input) as Promise<ProjectTaskDto>;

export const updateProjectTaskApi = (id: string, input: UpdateProjectTaskDto): Promise<ProjectTaskDto> =>
  putApi(projectTaskRoute(id), input) as Promise<ProjectTaskDto>;

