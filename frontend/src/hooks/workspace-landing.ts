import { getApi, getMyWorkspacesApi } from "@/hooks/useApi";
import { clearCurrentWorkspace, currentWorkspaceId, setCurrentWorkspace } from "@/lib/live-session";
import { isPlatformAdminConfig, type AppConfiguration } from "@/lib/live-identity";
import { LIVE_ENDPOINTS } from "@/lib/live-loader";
import { pickWorkspace } from "@/lib/sign-in";

/** Where a freshly signed-in person goes. */
export type Landing = "host" | "dashboard" | "chooser";

/**
 * Decide (and set) the current workspace after a sign-in: the platform operator
 * gets the console; otherwise none → chooser (create the first one), one → it,
 * several → the last used one while still a member, else the chooser.
 */
export async function resolveLanding(): Promise<Landing> {
  const mine = await getMyWorkspacesApi();
  const choice = pickWorkspace(mine, currentWorkspaceId());
  if (choice.kind === "select") setCurrentWorkspace(choice.id);
  else clearCurrentWorkspace();
  const config = (await getApi(LIVE_ENDPOINTS.configuration)) as AppConfiguration;
  if (isPlatformAdminConfig(config)) {
    clearCurrentWorkspace();
    return "host";
  }
  return choice.kind === "select" ? "dashboard" : "chooser";
}

/** A same-site path to return to after signing in (`?next=`), or null. */
export function safeNext(raw: string | null | undefined): string | null {
  if (!raw || !raw.startsWith("/") || raw.startsWith("//")) return null;
  return raw;
}
