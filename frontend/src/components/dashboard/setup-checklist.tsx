"use client";

import Link from "next/link";
import { CheckCircle2, Circle, Download, FolderPlus, UserPlus, ArrowRight } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { projects, users, activities } from "@/lib/tenant-data";
import { countSeats } from "@/lib/roles";

/**
 * Shown when a workspace has no real product data yet.
 * Guides owners/admins toward value before billing.
 */
export function SetupChecklist({ isOwner }: { isOwner: boolean }) {
  const hasTeam = countSeats(users) > 1;
  const hasProject = projects.some((p) => !p.archived);
  const hasActivity = activities.length > 0;
  const steps = [
    {
      id: "invite",
      done: hasTeam,
      title: "Invite your team",
      body: "Add members so seats and tracking have someone to cover.",
      href: "/team",
      cta: "Open Team",
      icon: UserPlus,
    },
    {
      id: "project",
      done: hasProject,
      title: "Create a project",
      body: "Projects carry capture permissions and time attribution.",
      href: "/projects",
      cta: "Open Projects",
      icon: FolderPlus,
    },
    {
      id: "agent",
      done: hasActivity,
      title: "Install a desktop agent",
      body: "Windows (Rust) or macOS (Swift) — offline queue until sync is live.",
      href: "/settings",
      cta: "Privacy & tracking",
      icon: Download,
    },
  ];

  const remaining = steps.filter((s) => !s.done).length;
  if (remaining === 0) return null;

  return (
    <Card className="border-primary/25 bg-primary/5">
      <CardHeader>
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <CardTitle>Get your workspace live</CardTitle>
            <p className="mt-1 text-sm text-muted-foreground">
              {remaining} step{remaining === 1 ? "" : "s"} left — start with people and a project, then capture.
            </p>
          </div>
          {isOwner && (
            <Link href="/billing" className="text-xs text-muted-foreground hover:text-foreground hover:underline">
              Plan & billing later →
            </Link>
          )}
        </div>
      </CardHeader>
      <CardContent className="space-y-3">
        {steps.map((s) => {
          const Icon = s.icon;
          return (
            <div
              key={s.id}
              className="flex flex-wrap items-center gap-3 rounded-xl border border-border bg-card p-3"
            >
              {s.done ? (
                <CheckCircle2 className="h-5 w-5 shrink-0 text-success" />
              ) : (
                <Circle className="h-5 w-5 shrink-0 text-muted-foreground" />
              )}
              <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
                <Icon className="h-4 w-4" />
              </div>
              <div className="min-w-0 flex-1">
                <div className={`text-sm font-medium ${s.done ? "text-muted-foreground line-through" : ""}`}>{s.title}</div>
                <div className="text-xs text-muted-foreground">{s.body}</div>
              </div>
              {!s.done && (
                <Link href={s.href}>
                  <Button size="sm" variant="outline">
                    {s.cta} <ArrowRight className="h-3.5 w-3.5" />
                  </Button>
                </Link>
              )}
            </div>
          );
        })}
      </CardContent>
    </Card>
  );
}
