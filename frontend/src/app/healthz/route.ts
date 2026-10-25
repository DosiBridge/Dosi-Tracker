import { healthReport, resolveRuntimeConfig } from "@/lib/runtime-config";

// Answer every probe from the live environment, never from a build-time copy.
export const dynamic = "force-dynamic";

/**
 * Container/orchestrator health probe. 200 when the web app can serve
 * sign-ins; 503 (with the reasons) when its configuration can't work, e.g. no
 * DOSI_API_URL in production. It never calls the API itself: an API outage
 * must not restart the web container.
 */
export function GET(): Response {
  const report = healthReport(resolveRuntimeConfig(process.env));
  return Response.json(report, {
    status: report.status === "ok" ? 200 : 503,
    headers: { "Cache-Control": "no-store" },
  });
}
