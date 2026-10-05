import { useState, useEffect, useCallback, useRef } from 'react';
import {
  REFRESH_TOKEN_STORAGE_KEY,
  SESSION_UNAUTHORIZED_EVENT,
  TENANT_STORAGE_KEY,
  TOKEN_EXPIRES_STORAGE_KEY,
  TOKEN_STORAGE_KEY,
  isLiveSession,
  readStorage,
  removeStorage,
  writeStorage,
} from '@/lib/live-session';
import { API_BASE_URL, WEB_SIGN_IN_CLIENT, tokenEndpoint } from '@/lib/sign-in';
import {
  API_ROUTES,
  TEAM_ROUTES,
  activateSubscriptionRoute,
  activityNoteRoute,
  changeTenantPlanRoute,
  collectPages,
  markInvoicePaidRoute,
  parseApiErrorBody,
  parseRetryAfter,
  projectMembersRoute,
  projectRoute,
  removeFromWorkspaceRoute,
  removeMemberRoute,
  reportingRoute,
  resetPasswordRoute,
  suspendSubscriptionRoute,
  TENANT_SUBSCRIPTIONS_ROUTE,
  toPagedResult,
  updateMemberRoute,
  type ActivityDto,
  type ApiErrorInfo,
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
  type ResetPasswordResultDto,
  type TenantSubscriptionDto,
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
/** How long the middleware's session flag lasts while a refresh token can renew the session (OpenIddict's default refresh-token lifetime). */
const RENEWABLE_SESSION_SECONDS = 14 * 24 * 60 * 60;

interface TokenResponse {
  access_token?: unknown;
  expires_in?: unknown;
  refresh_token?: unknown;
}

/**
 * Keep a token response: the access token and when it expires, the refresh
 * token (a renewal that brings none keeps the current one), and the session
 * flag the middleware reads. Throws when the browser won't store anything.
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
    res = await fetch(tokenEndpoint(readStorage(TENANT_STORAGE_KEY)), {
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

const headersFor = (token: string | null, json: boolean): Record<string, string> => ({
  ...(json ? { 'Content-Type': 'application/json' } : {}),
  ...(token ? { Authorization: `Bearer ${token}` } : {}),
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
      headers: headersFor(token, init.json !== false),
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
      window.location.href = '/login';
    }
  }
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
 * OpenIddict resource-owner password flow. For tenant users the token request
 * must carry the tenant (`__tenant`); afterwards the JWT itself resolves the
 * tenant on every API call.
 *
 * The token is mirrored into a cookie so the Next.js middleware can gate
 * routes server-side (localStorage is invisible to middleware). With
 * `offline_access` the response also carries a refresh token, which renews
 * the one-hour access token as it runs out (see `send`).
 *
 * Failures are told apart: InvalidCredentialsError (the details didn't
 * match), RateLimitedError (429 — wait, don't reset anything),
 * SignInServerError (5xx), ApiUnreachableError (no server) and
 * StorageUnavailableError (the browser won't keep the session).
 */
export const loginApi = async (username: string, password: string, tenant?: string): Promise<string> => {
  const params = new URLSearchParams();
  params.append('grant_type', 'password');
  params.append('username', username);
  params.append('password', password);
  params.append('client_id', WEB_SIGN_IN_CLIENT.clientId);
  params.append('scope', WEB_SIGN_IN_CLIENT.scope);

  // The tenant MUST be a query parameter, not a form field: ABP's tenant
  // resolvers read route/query/header/cookie and ignore the POST body, so a
  // body-only `__tenant` silently resolves to the host tenant.
  let res: Response;
  try {
    res = await fetch(tokenEndpoint(tenant), {
      method: 'POST',
      headers: {
        'Content-Type': 'application/x-www-form-urlencoded',
      },
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
      if (tenant) {
        keep(TENANT_STORAGE_KEY, tenant);
      } else {
        removeStorage(TENANT_STORAGE_KEY);
      }
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
    removeStorage(TENANT_STORAGE_KEY);
    document.cookie = 'dosi-token=; Path=/; Max-Age=0; SameSite=Lax';
  }
};

/**
 * Self-service SaaS signup: creates an isolated tenant with its own admin user
 * and starts a subscription on the chosen plan (backend: WorkspaceAppService).
 */
export const registerWorkspaceApi = async (
  name: string,
  adminEmail: string,
  adminPassword: string,
  planName?: string,
) => {
  let res: Response;
  try {
    res = await fetch(`${API_BASE_URL}/api/app/workspace/register`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ name, adminEmail, adminPassword, planName }),
    });
  } catch {
    throw new ApiUnreachableError();
  }

  if (!res.ok) {
    const errorText = await res.text();
    let message = `Workspace registration failed (HTTP ${res.status})`;
    try {
      const parsed = JSON.parse(errorText);
      message = parsed?.error?.message || message;
    } catch {}
    throw new Error(message);
  }
  return await res.json() as { tenantId: string; name: string };
};

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

/** Invite someone to a project by email; a new user comes back with a one-time password. */
export const inviteMember = (input: InviteMemberDto): Promise<InviteMemberResultDto> => postApi(TEAM_ROUTES.inviteMember, input);

/** Change a member's role and/or rate on one project (see membershipEditRules for who may). */
export const updateProjectMember = (projectId: string, userId: string, input: UpdateProjectMemberDto): Promise<ProjectMemberDto> =>
  putApi(updateMemberRoute(projectId, userId), input);

/** Remove one project membership, by its id from getProjectMembers. */
export const removeProjectMember = (membershipId: string): Promise<void> => delApi(removeMemberRoute(membershipId));

/** Issue a member a new one-time password; they must change it at their next sign-in. */
export const resetMemberPassword = (userId: string): Promise<ResetPasswordResultDto> =>
  postApi(resetPasswordRoute(userId), {});

/** Remove a member from every project and deactivate them (frees their seat). */
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

/** Host: move a tenant to another plan. */
export const changeTenantPlan = async (tenantId: string, planId: string): Promise<void> => {
  await postApi(changeTenantPlanRoute(tenantId, planId), {});
};

/** Host: mark a tenant's subscription active (e.g. after payment). */
export const activateSubscription = async (tenantId: string): Promise<void> => {
  await postApi(activateSubscriptionRoute(tenantId), {});
};

/** Host: suspend a tenant's subscription (it can't record time until activated again). */
export const suspendSubscription = async (tenantId: string): Promise<void> => {
  await postApi(suspendSubscriptionRoute(tenantId), {});
};

/**
 * Host: every workspace's plan, status and seats; null when the server has no
 * such endpoint (an older one answers 404), so the console can say "—".
 */
export const getTenantSubscriptions = async (): Promise<TenantSubscriptionDto[] | null> => {
  try {
    const items = await getApi(TENANT_SUBSCRIPTIONS_ROUTE);
    return Array.isArray(items) ? (items as TenantSubscriptionDto[]) : [];
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
