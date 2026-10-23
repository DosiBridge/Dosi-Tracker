"use client";

import { useEffect, useState } from "react";
import { ChevronRight, Loader2, Radar } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { ThemeToggle } from "@/components/ui/theme-toggle";
import { CreateWorkspaceForm } from "@/components/layout/workspace-switcher";
import { useSession } from "@/components/session-provider";
import { toast } from "@/components/toast";
import { archiveWorkspaceApi, getMyWorkspacesApi, restoreWorkspaceApi } from "@/hooks/useApi";
import { clearCurrentWorkspace, currentWorkspaceId, switchToWorkspace } from "@/lib/live-session";
import { dropLiveDatasets } from "@/lib/tenant-data";
import { isWorkspaceOwner, workspaceInitial, workspaceRoleLabel, type WorkspaceMembership } from "@/lib/sign-in";

interface ErrorShape {
  serverMessage?: string | null;
  message?: string | null;
}

function errorFields(err: unknown): ErrorShape {
  return (err ?? {}) as ErrorShape;
}

/** Pick (or create) the workspace to work in — Notion-style. */
export default function WorkspacesPage() {
  const { logout } = useSession();
  const [mine, setMine] = useState<WorkspaceMembership[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [creating, setCreating] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [archivingWorkspace, setArchivingWorkspace] = useState<WorkspaceMembership | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [refreshError, setRefreshError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => {
    let alive = true;
    getMyWorkspacesApi().then(
      (list) => alive && setMine(list),
      () => alive && setFailed(true),
    );
    return () => {
      alive = false;
    };
  }, []);

  const empty = mine !== null && mine.length === 0;

  async function reloadMine() {
    if (refreshing || busyId) return;
    setRefreshing(true);
    try {
      const refreshed = await getMyWorkspacesApi();
      setMine(refreshed);
      setRefreshError(null);
    } catch {
      setRefreshError("Couldn't refresh your workspaces. Check your connection and try again.");
    } finally {
      setRefreshing(false);
    }
  }

  async function handleRestore(w: WorkspaceMembership) {
    if (busyId) return;
    setBusyId(w.id);
    setActionError(null);
    setRefreshError(null);
    try {
      await restoreWorkspaceApi(w.id);
    } catch (err) {
      const e = errorFields(err);
      const msg = e.serverMessage || e.message || "Couldn't restore the workspace. Try again.";
      setActionError(msg);
      toast({ title: "Couldn't restore workspace", description: msg, tone: "danger" });
      setBusyId(null);
      return;
    }
    // Mutation succeeded: reconcile target locally immediately so repeated restore is prevented
    setMine((prev) =>
      prev ? prev.map((item) => (item.id.toLowerCase() === w.id.toLowerCase() ? { ...item, isArchived: false } : item)) : prev
    );
    toast({ title: "Workspace restored", description: `${w.name} is now active.`, tone: "success" });

    try {
      const refreshed = await getMyWorkspacesApi();
      setMine(refreshed);
    } catch {
      setRefreshError("Workspace was restored, but we couldn't refresh the list. Retry to update.");
    } finally {
      setBusyId(null);
    }
  }

  async function handleArchive(w: WorkspaceMembership) {
    if (busyId) return;
    setBusyId(w.id);
    setActionError(null);
    setRefreshError(null);
    try {
      await archiveWorkspaceApi(w.id);
    } catch (err) {
      const e = errorFields(err);
      const msg = e.serverMessage || e.message || "Couldn't archive the workspace. Try again.";
      setActionError(msg);
      toast({ title: "Couldn't archive workspace", description: msg, tone: "danger" });
      setBusyId(null);
      return;
    }
    // Mutation succeeded: reconcile target locally immediately so stale ordinary selection is impossible
    setMine((prev) =>
      prev ? prev.map((item) => (item.id.toLowerCase() === w.id.toLowerCase() ? { ...item, isArchived: true } : item)) : prev
    );
    if (currentWorkspaceId()?.toLowerCase() === w.id.toLowerCase()) {
      clearCurrentWorkspace();
      dropLiveDatasets();
    }
    toast({ title: "Workspace archived", description: `${w.name} has been archived.`, tone: "info" });
    setArchivingWorkspace(null);

    try {
      const refreshed = await getMyWorkspacesApi();
      setMine(refreshed);
    } catch {
      setRefreshError("Workspace was archived, but we couldn't refresh the list. Retry to update.");
    } finally {
      setBusyId(null);
    }
  }

  return (
    <div className="flex min-h-screen items-start justify-center p-6 sm:items-center">
      <div className="absolute right-4 top-4">
        <ThemeToggle />
      </div>
      <div className="page-enter w-full max-w-md py-10">
        <div className="mb-8 flex items-center gap-3">
          <div className="flex h-10 w-10 items-center justify-center rounded-xl brand-gradient text-white">
            <Radar className="h-6 w-6" />
          </div>
          <span className="font-display text-lg font-bold tracking-tight">Dosi-Tracker</span>
        </div>

        {failed ? (
          <p role="alert" className="rounded-lg bg-danger/10 px-3 py-2 text-sm font-medium text-danger-on-tint">
            We couldn&apos;t load your workspaces. Check your connection and reload the page.
          </p>
        ) : mine === null ? (
          <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin motion-reduce:animate-none" aria-hidden="true" /> Loading your workspaces…
          </p>
        ) : empty || creating ? (
          <>
            <h1 className="font-display text-2xl font-bold tracking-tight">
              {empty ? "Create your first workspace" : "Create a workspace"}
            </h1>
            <p className="mt-1 mb-6 text-sm text-muted-foreground">
              {empty
                ? "You're not in any workspace yet. Create one to start tracking — or ask a workspace owner for an invitation."
                : "You'll be its owner."}
            </p>
            <CreateWorkspaceForm onCancel={empty ? undefined : () => setCreating(false)} />
          </>
        ) : (
          <>
            <h1 className="font-display text-2xl font-bold tracking-tight">Choose a workspace</h1>
            <p className="mt-1 text-sm text-muted-foreground">You can switch any time from the sidebar.</p>

            {actionError && (
              <p role="alert" className="mt-3 rounded-lg bg-danger/10 px-3 py-2 text-sm font-medium text-danger-on-tint">
                {actionError}
              </p>
            )}

            {refreshError && (
              <div role="alert" className="mt-3 flex items-center justify-between gap-3 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-sm text-warning-on-tint">
                <span className="min-w-0 flex-1 font-medium">{refreshError}</span>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  disabled={refreshing || !!busyId}
                  onClick={() => void reloadMine()}
                  className="shrink-0"
                >
                  {refreshing && <Loader2 className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none mr-1" aria-hidden="true" />}
                  {refreshing ? "Retrying…" : "Retry"}
                </Button>
              </div>
            )}

            <ul className="mt-6 space-y-2" aria-label="Your workspaces">
              {mine.map((w) => {
                const owner = isWorkspaceOwner(w);
                const isArchived = !!w.isArchived;

                return (
                  <li key={w.id}>
                    {isArchived ? (
                      <div className="flex w-full items-center justify-between gap-3 rounded-xl border border-dashed border-border bg-card/60 px-3 py-3 text-left">
                        <span aria-hidden="true" className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-muted text-sm font-bold text-muted-foreground">
                          {workspaceInitial(w)}
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="block truncate text-sm font-semibold text-muted-foreground">{w.name}</span>
                          {w.planName && <span className="block truncate text-xs text-muted-foreground">{w.planName}</span>}
                        </span>
                        <div className="flex items-center gap-2">
                          <Badge tone="muted">Archived</Badge>
                          {owner && (
                            <Button
                              type="button"
                              variant="outline"
                              size="sm"
                              disabled={busyId === w.id}
                              onClick={() => void handleRestore(w)}
                              title={`Restore ${w.name}`}
                            >
                              {busyId === w.id && <Loader2 className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none mr-1" aria-hidden="true" />}
                              {busyId === w.id ? "Restoring…" : "Restore"}
                            </Button>
                          )}
                        </div>
                      </div>
                    ) : (
                      <div className="flex w-full items-center gap-2 rounded-xl border border-border bg-card p-1.5 transition-colors hover:bg-muted/50">
                        <button
                          type="button"
                          onClick={() => switchToWorkspace(w.id)}
                          className="flex min-w-0 flex-1 items-center gap-3 p-1.5 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-lg"
                        >
                          <span aria-hidden="true" className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg brand-gradient text-sm font-bold text-white">
                            {workspaceInitial(w)}
                          </span>
                          <span className="min-w-0 flex-1">
                            <span className="block truncate text-sm font-semibold">{w.name}</span>
                            {w.planName && <span className="block truncate text-xs text-muted-foreground">{w.planName}</span>}
                          </span>
                          <Badge tone={owner ? "info" : "muted"}>{workspaceRoleLabel(w.role)}</Badge>
                          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                        </button>
                        {owner && (
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            disabled={busyId === w.id}
                            onClick={() => setArchivingWorkspace(w)}
                            className="text-xs text-muted-foreground hover:text-danger-on-tint shrink-0"
                            title={`Archive ${w.name}`}
                          >
                            Archive
                          </Button>
                        )}
                      </div>
                    )}
                  </li>
                );
              })}
            </ul>
            <Button variant="outline" className="mt-4 w-full" onClick={() => setCreating(true)}>
              Create a new workspace
            </Button>
          </>
        )}

        {archivingWorkspace && (
          <ConfirmDialog
            open={!!archivingWorkspace}
            onCancel={() => setArchivingWorkspace(null)}
            onConfirm={() => void handleArchive(archivingWorkspace)}
            busy={busyId === archivingWorkspace.id}
            title={`Archive ${archivingWorkspace.name}?`}
            description="This workspace will be archived and hidden from normal switching. You can restore it at any time, and its projects, activities, and settings will be preserved. Subscription billing is not cancelled."
            confirmLabel="Archive workspace"
            tone="danger"
          />
        )}

        <button
          type="button"
          onClick={logout}
          className="mt-8 rounded text-sm text-muted-foreground hover:text-foreground hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          Sign out
        </button>
      </div>
    </div>
  );
}
