"use client";

import Link from "next/link";
import {
  Clock,
  Gauge,
  AppWindow,
  FolderKanban,
  CalendarCheck,
  CalendarDays,
  DollarSign,
  ArrowRight,
  Plus,
  Repeat,
  Mail,
  FileText,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { reportCatalog } from "@/lib/reports-data";

const icons: Record<string, LucideIcon> = {
  Clock, Gauge, AppWindow, FolderKanban, CalendarCheck, CalendarDays, DollarSign,
};

const categoryTone: Record<string, "primary" | "success" | "info" | "warning"> = {
  Time: "primary",
  Productivity: "success",
  People: "info",
  Finance: "warning",
};

const scheduled = [
  { name: "Weekly team summary", cadence: "Every Monday · 9:00 AM", recipients: 3, report: "Weekly Summary" },
  { name: "Payroll export", cadence: "1st of month", recipients: 1, report: "Payroll & Billing" },
  { name: "Daily productivity digest", cadence: "Every day · 6:00 PM", recipients: 5, report: "Productivity" },
];

export default function ReportsHubPage() {
  const categories = ["Time", "Productivity", "People", "Finance"] as const;

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Reports</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Generate, schedule, and export detailed reports across your team.
          </p>
        </div>
        <Button>
          <Plus className="h-4 w-4" /> Schedule report
        </Button>
      </div>

      {/* Catalog by category */}
      {categories.map((cat) => {
        const list = reportCatalog.filter((r) => r.category === cat);
        if (!list.length) return null;
        return (
          <div key={cat} className="space-y-3">
            <div className="flex items-center gap-2">
              <h2 className="text-sm font-semibold text-muted-foreground">{cat}</h2>
              <span className="h-px flex-1 bg-border" />
            </div>
            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {list.map((r) => {
                const Icon = icons[r.icon] ?? FileText;
                return (
                  <Link key={r.slug} href={`/reports/${r.slug}`}>
                    <Card className="group h-full p-5 transition-all hover:-translate-y-0.5 hover:card-elev-lg">
                      <div className="flex items-start justify-between">
                        <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-accent text-accent-foreground">
                          <Icon className="h-5 w-5" />
                        </div>
                        <Badge tone={categoryTone[r.category]}>{r.category}</Badge>
                      </div>
                      <h3 className="mt-4 font-semibold group-hover:text-primary">{r.title}</h3>
                      <p className="mt-1 text-sm text-muted-foreground">{r.description}</p>
                      <div className="mt-3 inline-flex items-center gap-1 text-sm font-medium text-primary opacity-0 transition-opacity group-hover:opacity-100">
                        Open report <ArrowRight className="h-4 w-4" />
                      </div>
                    </Card>
                  </Link>
                );
              })}
            </div>
          </div>
        );
      })}

      {/* Scheduled reports */}
      <Card>
        <CardHeader>
          <CardTitle>Scheduled reports</CardTitle>
          <Badge tone="muted">{scheduled.length} active</Badge>
        </CardHeader>
        <CardContent className="space-y-1">
          {scheduled.map((s) => (
            <div key={s.name} className="flex items-center gap-3 rounded-lg p-2 hover:bg-muted/60">
              <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-muted text-muted-foreground">
                <Repeat className="h-5 w-5" />
              </div>
              <div className="min-w-0 flex-1">
                <div className="truncate text-sm font-medium">{s.name}</div>
                <div className="truncate text-xs text-muted-foreground">{s.cadence} · {s.report}</div>
              </div>
              <span className="hidden items-center gap-1 text-xs text-muted-foreground sm:flex">
                <Mail className="h-3.5 w-3.5" /> {s.recipients}
              </span>
              <Button variant="ghost" size="sm">Edit</Button>
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  );
}
