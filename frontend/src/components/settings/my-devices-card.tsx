"use client";

import { useCallback, useEffect, useState } from "react";
import Link from "next/link";
import { Laptop, ShieldOff } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { toast } from "@/components/toast";
import { requestErrorText } from "@/components/layout/request-error";
import { RevokeDeviceDialog } from "@/components/dashboard/revoke-device-dialog";
import { getWorkspaceDevicesApi, revokeMyDeviceApi } from "@/hooks/useApi";
import { platformLabel, type WorkspaceDeviceDto } from "@/lib/api-contract";
import { agoLabel } from "@/lib/live-session";
import { normalizeUtcIso } from "@/lib/live-dataset";

/**
 * The signed-in person's own desktop agent installs in this workspace
 * (GET /api/app/device, filtered to them), with Revoke for a lost or retired
 * computer (E2E-09). Only an owner or manager restores a revoked one.
 */
export function MyDevicesCard({ live, userId, manager }: { live: boolean; userId: string; manager: boolean }) {
  const [devices, setDevices] = useState<WorkspaceDeviceDto[] | null | "failed" | "loading">(live ? "loading" : null);
  const [revoking, setRevoking] = useState<WorkspaceDeviceDto | null>(null);

  const load = useCallback(async () => {
    try {
      const all = await getWorkspaceDevicesApi();
      setDevices(all === null ? null : all.filter((d) => d.userId === userId));
    } catch {
      setDevices("failed");
    }
  }, [userId]);

  useEffect(() => {
    if (!live) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    void load();
  }, [live, load]);

  const nameOf = (d: WorkspaceDeviceDto) => d.deviceName?.trim() || "Unnamed device";

  async function revoke(d: WorkspaceDeviceDto, reason: string | null) {
    try {
      await revokeMyDeviceApi(d.deviceId, reason);
    } catch (err) {
      throw new Error(requestErrorText(err, "Couldn't revoke the device. Try again."));
    }
    setRevoking(null);
    toast({ tone: "success", title: `${nameOf(d)} revoked`, description: "Its agent was signed out of this workspace." });
    void load();
  }

  return (
    <Card>
      <CardHeader>
        <div>
          <CardTitle>Your devices</CardTitle>
          <p className="mt-0.5 text-xs text-muted-foreground">
            Computers where you signed in to the desktop agent for this workspace. Lost one? Revoke it: it is signed out and can&apos;t
            track here any more.
          </p>
        </div>
      </CardHeader>
      <CardContent className="space-y-3">
        {!live ? (
          <p className="rounded-xl bg-muted/60 px-3 py-3 text-sm text-muted-foreground">
            The demo workspace has no devices of yours. Sign in to your own workspace to manage them.
          </p>
        ) : devices === "loading" ? (
          <p role="status" className="text-sm text-muted-foreground">
            Loading your devices…
          </p>
        ) : devices === "failed" ? (
          <div role="alert" className="flex flex-wrap items-center gap-3 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-xs text-warning-on-tint">
            <span className="min-w-0 flex-1">Couldn&apos;t load your devices.</span>
            <Button size="sm" variant="outline" onClick={() => void load()}>
              Try again
            </Button>
          </div>
        ) : devices === null ? (
          <p className="text-sm text-muted-foreground">This server can&apos;t list devices yet.</p>
        ) : devices.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            None yet. A computer appears here once you sign in to the{" "}
            <Link href="/download" className="font-medium text-primary-strong underline underline-offset-2">
              desktop agent
            </Link>{" "}
            on it.
          </p>
        ) : (
          <ul className="divide-y divide-border rounded-xl border border-border" aria-label="Your devices">
            {devices.map((d) => {
              const seen = normalizeUtcIso(d.lastSeenAt);
              const system = d.osVersion?.trim() || platformLabel(d.platform) || "Unknown system";
              return (
                <li key={d.deviceId} className="flex flex-wrap items-center gap-3 px-3 py-3">
                  <Laptop aria-hidden className="h-4 w-4 shrink-0 text-muted-foreground" />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-medium">{nameOf(d)}</span>
                    <span className="block truncate text-xs text-muted-foreground">
                      {system}
                      {d.agentVersion ? ` · agent ${d.agentVersion}` : ""}
                      {seen ? ` · last seen ${agoLabel(seen)}` : ""}
                    </span>
                  </span>
                  {d.isRevoked ? (
                    <Badge tone="danger" title={d.reason ? `Reason: ${d.reason}` : undefined}>
                      Revoked
                    </Badge>
                  ) : (
                    <Button size="sm" variant="ghost" className="text-danger-on-tint hover:bg-danger/10" onClick={() => setRevoking(d)} aria-label={`Revoke ${nameOf(d)}`}>
                      <ShieldOff aria-hidden className="h-3.5 w-3.5" /> Revoke
                    </Button>
                  )}
                </li>
              );
            })}
          </ul>
        )}
        {live && manager && (
          <p className="text-xs text-muted-foreground">
            Everyone&apos;s devices, and restoring revoked ones:{" "}
            <Link href="/team" className="font-medium text-primary-strong underline underline-offset-2">
              Team → Devices
            </Link>
            .
          </p>
        )}
      </CardContent>
      {revoking && (
        <RevokeDeviceDialog device={{ name: nameOf(revoking), memberName: "" }} own onClose={() => setRevoking(null)} onConfirm={(reason) => revoke(revoking, reason)} />
      )}
    </Card>
  );
}
