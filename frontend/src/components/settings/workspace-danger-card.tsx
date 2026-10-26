"use client";

import { useState } from "react";
import { Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { toast } from "@/components/toast";
import { DeleteWorkspaceDialog } from "@/components/workspaces/delete-workspace-dialog";
import { clearCurrentWorkspace, currentWorkspaceId, redirectToWorkspaceChooser } from "@/lib/live-session";
import { dropLiveDatasets } from "@/lib/tenant-data";
import { purgeDateLabel } from "@/lib/erasure";

/** Settings → Workspace's last card (owners): delete the workspace for good (E2E-07). */
export function WorkspaceDangerCard({ live, workspaceName, demoNote }: { live: boolean; workspaceName: string; demoNote: string }) {
  const [open, setOpen] = useState(false);
  const workspaceId = live ? currentWorkspaceId() : null;

  return (
    <Card className="border-danger/30">
      <CardHeader>
        <div>
          <CardTitle>Delete this workspace</CardTitle>
          <p className="mt-0.5 text-xs text-muted-foreground">
            Removes {workspaceName} for everyone and erases its tracked data after a short grace period. To keep the data, archive it from
            the workspace list instead.
          </p>
        </div>
      </CardHeader>
      <CardContent>
        <Button
          variant="outline"
          className="border-danger/40 text-danger-on-tint hover:bg-danger/10"
          onClick={() => setOpen(true)}
          disabled={live && !workspaceId}
        >
          <Trash2 aria-hidden className="h-4 w-4" /> Delete workspace…
        </Button>
      </CardContent>
      {open && (
        <DeleteWorkspaceDialog
          workspace={{ id: workspaceId ?? "", name: workspaceName }}
          live={live}
          onClose={() => setOpen(false)}
          onDeleted={(result) => {
            setOpen(false);
            if (!live) {
              toast({ tone: "info", title: "Demo workspace", description: demoNote });
              return;
            }
            const purge = purgeDateLabel(result?.purgeAfter);
            toast({ tone: "info", title: "Workspace deleted", description: purge ? `Its tracked data is erased for good on ${purge}.` : undefined });
            // It is gone for everyone, this tab included: forget it and choose another.
            clearCurrentWorkspace();
            dropLiveDatasets();
            redirectToWorkspaceChooser();
          }}
        />
      )}
    </Card>
  );
}
