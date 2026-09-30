"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";
import { AlertTriangle, Loader2, RotateCw } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { useSession } from "@/components/session-provider";

/**
 * Holds the app shell until the session is known, so a signed-in owner never
 * sees a frame of demo people while their real identity loads, and shows an
 * honest error (with retry) when the backend can't be reached, instead of
 * quietly falling back to demo data.
 *
 * `area` picks the home a mismatched role is sent to: a live platform admin
 * who lands on the tenant app goes to /host, and vice versa.
 */
export function SessionGate({ area, children }: { area: "tenant" | "host"; children: React.ReactNode }) {
  const router = useRouter();
  const { status, liveError, refreshSession, logout, user, isLive } = useSession();

  const wrongArea = isLive && status === "ready" && (area === "tenant" ? user.role === "host" : user.role !== "host");
  useEffect(() => {
    if (wrongArea) router.replace(area === "tenant" ? "/host" : "/dashboard");
  }, [wrongArea, area, router]);

  if (status === "unknown" || status === "loading" || wrongArea) {
    return (
      <div className="flex min-h-screen items-center justify-center p-4" role="status" aria-live="polite">
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" /> Loading your workspace…
        </div>
      </div>
    );
  }

  if (status === "error") {
    return (
      <div className="flex min-h-screen items-center justify-center p-4">
        <Card className="flex max-w-md flex-col items-center gap-4 py-12 text-center">
          <div className="flex h-14 w-14 items-center justify-center rounded-full bg-danger/10 text-danger">
            <AlertTriangle className="h-7 w-7" />
          </div>
          <div className="px-6">
            <h2 className="text-lg font-semibold">Can&apos;t reach Dosi-Tracker</h2>
            <p className="mt-1 text-sm text-muted-foreground">{liveError}</p>
          </div>
          <div className="flex gap-2">
            <Button onClick={() => void refreshSession()}>
              <RotateCw className="h-4 w-4" /> Try again
            </Button>
            <Button
              variant="outline"
              onClick={() => {
                logout();
                router.push("/login");
              }}
            >
              Sign out
            </Button>
          </div>
        </Card>
      </div>
    );
  }

  return <>{children}</>;
}

/** Slim, dismiss-free notice when some (not all) live data failed to load. */
export function LiveWarningBanner() {
  const { liveWarning, refreshSession } = useSession();
  if (!liveWarning) return null;
  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-warning/30 bg-warning/10 px-4 py-2 text-sm lg:px-6" role="status">
      <AlertTriangle className="h-4 w-4 shrink-0 text-warning" />
      <span className="min-w-0 flex-1">{liveWarning}</span>
      <button onClick={() => void refreshSession({ silent: true })} className="text-xs font-medium text-primary hover:underline">
        Retry
      </button>
    </div>
  );
}
