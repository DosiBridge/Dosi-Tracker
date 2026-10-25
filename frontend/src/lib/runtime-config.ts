/* ============================================================================
 * RUNTIME CONFIGURATION — what differs per deployment, read when a request is
 * served, not when the app is built.
 *
 * `NEXT_PUBLIC_*` values are frozen into the bundle at `next build`, so one
 * image could never move from staging to production, and a build that forgot
 * the variable silently talked to https://localhost:6006. Instead the server
 * reads plain environment variables per request:
 *
 *   DOSI_API_URL              the Dosi-Tracker API this web app (and every
 *                             desktop agent) signs in to. Required in
 *                             production. https only, except loopback hosts
 *                             or with DOSI_ALLOW_INSECURE_HTTP=true.
 *   DOSI_DOWNLOAD_BASE_URL    where the desktop agent release assets live
 *                             (default: GitHub Releases, latest).
 *   DOSI_DEMO_MODE            "1"/"true" serves the built-in demo dataset to
 *                             visitors without a session. Off by default and
 *                             meant for demos and the E2E suite only.
 *   DOSI_ALLOW_INSECURE_HTTP  "true" accepts a plain-http API URL that is not
 *                             loopback (an explicit admin opt-in).
 *   NEXT_PUBLIC_API_URL       legacy name for DOSI_API_URL, still honoured.
 *
 * The root layout writes the resolved values into the page as a JSON data
 * block (never executed, so it needs no CSP allowance); browser code reads it
 * through `runtimeConfig()`. Server code (the proxy, server components, SSR of
 * client components) resolves the same values from `process.env` directly.
 * ========================================================================== */

/** The API a development checkout talks to when nothing is configured (the backend's dev port). */
export const DEV_API_BASE_URL = "https://localhost:6006";

/** Where agent release assets are published unless the deployment says otherwise. */
export const DEFAULT_DOWNLOAD_BASE_URL = "https://github.com/DosiBridge/Dosi-Tracker/releases/latest/download";

/** The id of the JSON data block the root layout renders. */
export const RUNTIME_CONFIG_ELEMENT_ID = "dosi-runtime-config";

export interface RuntimeConfig {
  /** The API base URL without a trailing slash; null when none (valid) is configured. */
  apiBaseUrl: string | null;
  /** The release download base URL without a trailing slash. */
  downloadBaseUrl: string;
  /** Visitors without a session get the demo dataset instead of the sign-in page. */
  demoMode: boolean;
  /** What an operator must fix, in plain words (empty when the configuration is sound). */
  problems: string[];
}

/** The variables `resolveRuntimeConfig` reads (a subset of `process.env`). */
export interface RuntimeEnv {
  DOSI_API_URL?: string;
  NEXT_PUBLIC_API_URL?: string;
  DOSI_DOWNLOAD_BASE_URL?: string;
  DOSI_DEMO_MODE?: string;
  DOSI_ALLOW_INSECURE_HTTP?: string;
  NODE_ENV?: string;
}

const TRUE = new Set(["1", "true", "yes", "on"]);

/** "1", "true", "yes", "on" (any case) — anything else is off. */
export function envFlag(value: string | undefined): boolean {
  return TRUE.has((value ?? "").trim().toLowerCase());
}

const LOOPBACK = new Set(["localhost", "127.0.0.1", "[::1]"]);

/** Whether `hostname` is this machine (plain http is acceptable there). */
export function isLoopbackHost(hostname: string): boolean {
  const host = hostname.toLowerCase();
  return LOOPBACK.has(host) || host.endsWith(".localhost") || /^127(\.\d{1,3}){3}$/.test(host);
}

export type BaseUrlResult = { url: string; problem: null } | { url: null; problem: string };

/**
 * A configured base URL, normalised: trimmed, http(s) only, no credentials,
 * query or fragment, and no trailing slash (so `${base}/connect/token` never
 * becomes `//connect/token`). Plain http is refused unless the host is
 * loopback or `allowInsecureHttp` is set.
 */
export function normalizeBaseUrl(
  raw: string,
  name: string,
  options: { allowInsecureHttp?: boolean } = {},
): BaseUrlResult {
  const value = raw.trim();
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return { url: null, problem: `${name} is not a valid URL ("${value}").` };
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") {
    return { url: null, problem: `${name} must be an http(s) URL, not ${url.protocol}//.` };
  }
  if (url.username || url.password) {
    return { url: null, problem: `${name} must not embed a user name or other credentials.` };
  }
  if (url.search || url.hash) {
    return { url: null, problem: `${name} must not contain a query string or fragment.` };
  }
  if (url.protocol === "http:" && !isLoopbackHost(url.hostname) && !options.allowInsecureHttp) {
    return {
      url: null,
      problem: `${name} uses plain http:// for ${url.hostname}. Use https://, or set DOSI_ALLOW_INSECURE_HTTP=true to accept it on purpose.`,
    };
  }
  const path = url.pathname.replace(/\/+$/, "");
  return { url: `${url.origin}${path}`, problem: null };
}

/** Resolve the deployment's configuration from environment variables. Pure: pass `process.env`. */
export function resolveRuntimeConfig(env: RuntimeEnv): RuntimeConfig {
  const production = env.NODE_ENV === "production";
  const demoMode = envFlag(env.DOSI_DEMO_MODE);
  const allowInsecureHttp = envFlag(env.DOSI_ALLOW_INSECURE_HTTP);
  const problems: string[] = [];

  let apiBaseUrl: string | null = null;
  const rawApi = env.DOSI_API_URL?.trim() || env.NEXT_PUBLIC_API_URL?.trim() || "";
  if (rawApi) {
    const result = normalizeBaseUrl(rawApi, env.DOSI_API_URL?.trim() ? "DOSI_API_URL" : "NEXT_PUBLIC_API_URL", { allowInsecureHttp });
    apiBaseUrl = result.url;
    if (result.problem) problems.push(result.problem);
  } else if (!production || demoMode) {
    // A development checkout (or a demo, which never calls the API) gets the
    // backend's dev address. A production deployment gets nothing: it must say
    // where its API lives rather than quietly calling localhost.
    apiBaseUrl = DEV_API_BASE_URL;
  } else {
    problems.push("DOSI_API_URL is not set, so nobody can sign in. Set it to the address of your Dosi-Tracker API.");
  }

  let downloadBaseUrl = DEFAULT_DOWNLOAD_BASE_URL;
  const rawDownload = env.DOSI_DOWNLOAD_BASE_URL?.trim();
  if (rawDownload) {
    const result = normalizeBaseUrl(rawDownload, "DOSI_DOWNLOAD_BASE_URL", { allowInsecureHttp });
    if (result.url) downloadBaseUrl = result.url;
    else problems.push(`${result.problem} Using the default download location instead.`);
  }

  return { apiBaseUrl, downloadBaseUrl, demoMode, problems };
}

function isRuntimeConfig(value: unknown): value is RuntimeConfig {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  return (
    (v.apiBaseUrl === null || typeof v.apiBaseUrl === "string") &&
    typeof v.downloadBaseUrl === "string" &&
    typeof v.demoMode === "boolean" &&
    Array.isArray(v.problems)
  );
}

/** The configuration the server rendered into this page, or null when there is none. */
function fromDocument(): RuntimeConfig | null {
  if (typeof document === "undefined") return null;
  const text = document.getElementById(RUNTIME_CONFIG_ELEMENT_ID)?.textContent;
  if (!text) return null;
  try {
    const parsed: unknown = JSON.parse(text);
    return isRuntimeConfig(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function processEnv(): RuntimeEnv {
  return typeof process !== "undefined" && process.env ? (process.env as RuntimeEnv) : {};
}

let rendered: RuntimeConfig | null = null;

/**
 * This deployment's configuration. In the browser it is what the server
 * rendered into the page (read once, then kept); on the server, and in a
 * browser page that carries no data block (unit tests), it is resolved from
 * the environment on every call.
 */
export function runtimeConfig(): RuntimeConfig {
  if (typeof window !== "undefined") {
    rendered ??= fromDocument();
    if (rendered) return rendered;
  }
  return resolveRuntimeConfig(processEnv());
}

/**
 * The JSON for the page's data block. `<` is escaped so no value can close
 * the surrounding <script> element.
 */
export function serializeRuntimeConfig(config: RuntimeConfig): string {
  return JSON.stringify(config).replace(/</g, "\\u003c");
}

/** The deployment names no (valid) API server, so nothing can sign in or load. */
export class ApiNotConfiguredError extends Error {
  constructor() {
    super("This site has no Dosi-Tracker API server configured.");
    this.name = "ApiNotConfiguredError";
  }
}

/** The API base URL; throws ApiNotConfiguredError when the deployment has none. */
export function requireApiBaseUrl(): string {
  const base = runtimeConfig().apiBaseUrl;
  if (!base) throw new ApiNotConfiguredError();
  return base;
}

/** The full URL of an API endpoint (`/api/...`, `/connect/token`). */
export function apiUrl(endpoint: string): string {
  return `${requireApiBaseUrl()}${endpoint.startsWith("/") ? endpoint : `/${endpoint}`}`;
}

/** The API's origin (for the CSP's connect-src), or null when none is configured. */
export function apiOrigin(config: RuntimeConfig = runtimeConfig()): string | null {
  return config.apiBaseUrl ? new URL(config.apiBaseUrl).origin : null;
}

export interface HealthReport {
  /** "ok", or "misconfigured" when nobody could sign in (no valid API address). */
  status: "ok" | "misconfigured";
  /** Everything an operator should fix, fatal or not. */
  problems: string[];
  demoMode: boolean;
}

/**
 * What /healthz answers. Only a missing or invalid API address is fatal: the
 * server is up, but the app it serves can't work. A bad download location
 * falls back to the default, so it is reported without failing the probe.
 */
export function healthReport(config: RuntimeConfig): HealthReport {
  return {
    status: config.apiBaseUrl ? "ok" : "misconfigured",
    problems: config.problems,
    demoMode: config.demoMode,
  };
}

/** Forget the configuration read from the page (tests that swap the data block). */
export function resetRuntimeConfigCache(): void {
  rendered = null;
}
