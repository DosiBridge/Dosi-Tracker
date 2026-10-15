"use client";

import { useEffect, useState } from "react";
import { ChevronRight, Loader2, Radar } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ThemeToggle } from "@/components/ui/theme-toggle";
import { CreateWorkspaceForm } from "@/components/layout/workspace-switcher";
import { useSession } from "@/components/session-provider";
import { getMyWorkspacesApi } from "@/hooks/useApi";
import { switchToWorkspace } from "@/lib/live-session";
import { workspaceInitial, workspaceRoleLabel, type WorkspaceMembership } from "@/lib/sign-in";

/** Pick (or create) the workspace to work in — Notion-style. */
export default function WorkspacesPage() {
  const { logout } = useSession();
  const [mine, setMine] = useState<WorkspaceMembership[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [creating, setCreating] = useState(false);

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
            <ul className="mt-6 space-y-2" aria-label="Your workspaces">
              {mine.map((w) => (
                <li key={w.id}>
                  <button
                    type="button"
                    onClick={() => switchToWorkspace(w.id)}
                    className="flex w-full items-center gap-3 rounded-xl border border-border bg-card px-3 py-3 text-left transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    <span aria-hidden="true" className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg brand-gradient text-sm font-bold text-white">
                      {workspaceInitial(w)}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-semibold">{w.name}</span>
                      {w.planName && <span className="block truncate text-xs text-muted-foreground">{w.planName}</span>}
                    </span>
                    <Badge tone={w.role === "owner" ? "info" : "muted"}>{workspaceRoleLabel(w.role)}</Badge>
                    <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                  </button>
                </li>
              ))}
            </ul>
            <Button variant="outline" className="mt-4 w-full" onClick={() => setCreating(true)}>
              Create a new workspace
            </Button>
          </>
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
