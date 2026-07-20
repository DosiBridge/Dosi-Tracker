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
import { PageHeader, PageStack, SectionLabel } from "@/components/ui/page-header";
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
    <PageStack>
      <PageHeader
        eyebrow="Analytics"
        title="Reports"
        description="Generate, schedule, and export detailed reports across your team."
        actions={
          <Button>
            <Plus className="h-4 w-4" /> Schedule report
          </Button>
        }
      />

      {categories.map((cat) => {
        const list = reportCatalog.filter((r) => r.category === cat);
        if (!list.length) return null;
        return (
          <div key={cat} className="space-y-3">
            <SectionLabel>{cat}</SectionLabel>
            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {list.map((r) => {
                const Icon = icons[r.icon] ?? FileText;
                return (
                  <Link key={r.slug} href={`/reports/${r.slug}`}>
                    <Card variant="interactive" className="group h-full p-5">
                      <div className="flex items-start justify-between">
                        <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-accent text-accent-foreground">
                          <Icon className="h-5 w-5" />
                        </div>
                        <Badge tone={categoryTone[r.category]}>{r.category}</Badge>
                      </div>
                      <h3 className="mt-4 font-semibold group-hover:text-primary">{r.title}</h3>
                      <p className="mt-1 line-clamp-2 text-sm text-muted-foreground">{r.description}</p>
                      <span className="mt-4 inline-flex items-center gap-1 text-xs font-medium text-primary">
                        Open report <ArrowRight className="h-3.5 w-3.5" />
                      </span>
                    </Card>
                  </Link>
                );
              })}
            </div>
          </div>
        );
      })}

      <div className="space-y-3">
        <SectionLabel>Scheduled</SectionLabel>
        <Card>
          <CardHeader>
            <CardTitle>Upcoming deliveries</CardTitle>
            <Badge tone="muted" className="gap-1"><Repeat className="h-3 w-3" /> Auto</Badge>
          </CardHeader>
          <CardContent className="space-y-2">
            {scheduled.map((s) => (
              <div key={s.name} className="flex flex-wrap items-center gap-3 rounded-xl border border-border/70 bg-muted/30 px-3 py-2.5">
                <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-accent text-accent-foreground">
                  <Mail className="h-4 w-4" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium">{s.name}</div>
                  <div className="truncate text-xs text-muted-foreground">{s.cadence} · {s.report}</div>
                </div>
                <span className="text-xs text-muted-foreground">{s.recipients} recipients</span>
              </div>
            ))}
          </CardContent>
        </Card>
      </div>
    </PageStack>
  );
}
