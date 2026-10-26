import { useEffect, useState } from "react";
import { getBillingModeApi } from "@/hooks/useApi";
import { isLiveSession } from "@/lib/live-session";
import { runtimeConfig } from "@/lib/runtime-config";

/**
 * Whether this deployment bills workspaces, for pages that have no workspace
 * loaded yet (sign-up, the workspace chooser's create form). Inside a
 * workspace, read it from the session instead (saas-data billingEnabledFor).
 *
 *  - `null` while asking, and when the server can't say (an older backend,
 *    which always billed) — callers keep their plan pickers then;
 *  - the demo (no session, demo mode on) bills: its plans and trials show,
 *    and nothing is fetched.
 *
 * One request per page view, shared by every caller.
 */
let asked: Promise<boolean | null> | null = null;

function ask(): Promise<boolean | null> {
  asked ??= getBillingModeApi().then(
    (mode) => (mode ? mode.enabled : null),
    () => {
      asked = null; // a failed request is asked again next time
      return null;
    },
  );
  return asked;
}

/** Forget the shared answer (tests). */
export function resetBillingModeForTests(): void {
  asked = null;
}

export function useBillingMode(): { enabled: boolean | null; ready: boolean } {
  const demo = runtimeConfig().demoMode && !isLiveSession();
  const [state, setState] = useState<{ enabled: boolean | null; ready: boolean }>(() => (demo ? { enabled: true, ready: true } : { enabled: null, ready: false }));

  useEffect(() => {
    if (demo) return;
    let alive = true;
    void ask().then((enabled) => {
      if (alive) setState({ enabled, ready: true });
    });
    return () => {
      alive = false;
    };
  }, [demo]);

  return state;
}
