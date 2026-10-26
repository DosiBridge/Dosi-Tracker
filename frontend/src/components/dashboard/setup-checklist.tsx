"use client";

import Link from "next/link";
import { CheckCircle2, Circle, Download, FolderPlus, UserPlus, ArrowRight } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { AiSetupHelpButton } from "@/components/ai/ai-setup-help-button";
import { projects, users, activities } from "@/lib/tenant-data";
import { isTrackedMember } from "@/lib/roles";
import { billingEnabledFor } from "@/lib/saas-data";
import { useSession } from "@/components/session-provider";
import { cn } from "@/lib/utils";
import { linkButtonClass } from "./link-button";

/**
 * Shown while a workspace has no real product data yet. The steps follow the
 * order they can actually be done in: people are invited TO a project, and an
 * agent tracks time ON one — so the project comes first.
 */
export function SetupChecklist({ isOwner }: { isOwner: boolean }) {
  const { workspace } = useSession();
  const billing = billingEnabledFor(workspace);
  const hasProject = projects.some((p) => !p.archived);
  // "Invite your team" is done only once someone besides the owner is on the
  // roster — a workspace of one has not invited anybody yet.
  const hasTeam = users.filter(isTrackedMember).length > 1;
  const hasActivity = activities.length > 0;
  const steps = [
    {
      id: "project",
      done: hasProject,
      title: "Create a project",
      body: "Projects hold what the agent captures, how often, and whose time counts where.",
      href: "/projects",
      cta: "Open Projects",
      icon: FolderPlus,
    },
    {
      id: "invite",
      done: hasTeam,
      title: "Invite your team",
      body: hasProject ? "Invite people to a project by email — each gets sign-in details to share." : "Invites add people to a project, so create one first.",
      href: "/team",
      cta: "Open Team",
      icon: UserPlus,
    },
    {
      id: "agent",
      done: hasActivity,
      title: "Install a desktop agent",
      body: "Install the Windows or macOS agent on each computer you want to track.",
      href: "/download",
      cta: "Get the agent",
      icon: Download,
    },
  ];

  const remaining = steps.filter((s) => !s.done).length;
  if (remaining === 0) return null;

  return (
    <Card className="border-primary/25 bg-primary/5">
      <CardHeader>
        <div className="flex w-full flex-wrap items-start justify-between gap-3">
          <div>
            <CardTitle>Get your workspace live</CardTitle>
            <p className="mt-1 text-sm text-muted-foreground">
              {remaining} {remaining === 1 ? "step" : "steps"} left — create a project, invite people to it, then install the agent.
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-3">
            <AiSetupHelpButton />
            {isOwner && billing && (
              <Link href="/billing" className="text-xs text-muted-foreground hover:text-foreground hover:underline">
                Plan & billing later →
              </Link>
            )}
          </div>
        </div>
      </CardHeader>
      <CardContent>
        <ol className="space-y-3">
          {steps.map((s, i) => {
            const Icon = s.icon;
            return (
              <li
                key={s.id}
                className="flex flex-col gap-3 rounded-xl border border-border bg-card p-3 sm:flex-row sm:items-center"
              >
                {/* Mobile: status + text on one row, the action full-width below it,
                    so the copy never gets squeezed into a sliver next to the button. */}
                <div className="flex min-w-0 flex-1 items-start gap-3 sm:items-center">
                  {s.done ? (
                    <CheckCircle2 aria-hidden className="mt-0.5 h-5 w-5 shrink-0 text-success sm:mt-0" />
                  ) : (
                    <Circle aria-hidden className="mt-0.5 h-5 w-5 shrink-0 text-muted-foreground sm:mt-0" />
                  )}
                  <div aria-hidden className="hidden h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground sm:flex">
                    <Icon className="h-4 w-4" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className={cn("text-sm font-medium", s.done && "text-muted-foreground line-through")}>{s.title}</div>
                    <div className="text-xs text-muted-foreground">
                      <span className="sr-only">
                        Step {i + 1}, {s.done ? "done" : "to do"}.{" "}
                      </span>
                      {s.body}
                    </div>
                  </div>
                </div>
                {!s.done && (
                  <Link href={s.href} className={cn(linkButtonClass("outline", "sm"), "w-full sm:w-auto")}>
                    {s.cta} <ArrowRight aria-hidden className="h-3.5 w-3.5" />
                  </Link>
                )}
              </li>
            );
          })}
        </ol>
      </CardContent>
    </Card>
  );
}
