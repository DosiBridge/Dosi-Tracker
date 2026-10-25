/* ============================================================================
 * CLIENT ERROR REPORTS — what a render crash leaves behind for whoever runs
 * the site.
 *
 * The error boundaries (app/** /error.tsx, app/global-error.tsx) describe
 * every error they catch with `describeClientError` and log it once. The
 * record is deliberately small and free of personal data: the error's kind,
 * Next.js's digest (which matches the server log line for server errors), the
 * route PATTERN (ids replaced, never the query string) and the time. Messages
 * from server errors are already generic in production; client messages are
 * kept to 200 characters and are never shown to the user.
 * ========================================================================== */

export interface ClientErrorReport {
  /** "Error", "TypeError", "ChunkLoadError", or "non-error" for a thrown non-Error value. */
  kind: string;
  /** Next.js's digest, present for errors forwarded from the server. */
  digest: string | null;
  message: string | null;
  /** The route with ids and tokens replaced (e.g. "/projects/:id"). */
  route: string;
  /** Which boundary caught it ("dashboard", "host", "global", …). */
  boundary: string;
  at: string;
}

const MAX_MESSAGE = 200;

/** UUIDs, long hex, numbers and long opaque tokens become ":id" so no identifier leaves the page. */
export function routePattern(pathname: string): string {
  return (
    pathname
      .split("?")[0]
      .split("#")[0]
      .split("/")
      .map((segment) =>
        /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(segment) ||
        /^\d+$/.test(segment) ||
        /^[0-9a-f]{16,}$/i.test(segment) ||
        /^[A-Za-z0-9_-]{24,}$/.test(segment)
          ? ":id"
          : segment,
      )
      .join("/") || "/"
  );
}

/** The report for one caught error. */
export function describeClientError(error: unknown, context: { pathname: string; boundary: string; now?: Date }): ClientErrorReport {
  const isError = error instanceof Error;
  const digest = isError && typeof (error as Error & { digest?: unknown }).digest === "string" ? (error as Error & { digest: string }).digest : null;
  const message = isError && error.message ? error.message.slice(0, MAX_MESSAGE) : null;
  return {
    kind: isError ? error.name || "Error" : "non-error",
    digest,
    message,
    route: routePattern(context.pathname),
    boundary: context.boundary,
    at: (context.now ?? new Date()).toISOString(),
  };
}

/**
 * A short reference a person can quote to support: the digest when the
 * server knows the error, else nothing (a client-only error has no server
 * log to match).
 */
export function errorReference(error: unknown): string | null {
  return describeClientError(error, { pathname: "/", boundary: "" }).digest;
}

/** Log one caught error (the browser console is where operators and reports pick it up). */
export function reportClientError(report: ClientErrorReport, log: (...args: unknown[]) => void = console.error): void {
  log("[dosi-tracker] render error", report);
}
