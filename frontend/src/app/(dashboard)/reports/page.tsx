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
  CalendarClock,
  ArrowRight,
  Mail,
  FileText,
  Sparkles,
  Check,
  MonitorSmartphone,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { PageHeader, PageStack, SectionLabel } from "@/components/ui/page-header";
import { useSession } from "@/components/session-provider";
import { useReportAccess } from "@/components/reports/report-access";
import { reportCatalog, type ReportMeta } from "@/lib/reports-data";
import { canAccess } from "@/lib/roles";
import { cn } from "@/lib/utils";

const icons: Record<string, LucideIcon> = {
  Clock, Gauge, AppWindow, FolderKanban, CalendarCheck, CalendarDays, DollarSign, CalendarClock, Sparkles, MonitorSmartphone,
};

/** Decorative bar heights (%) for the hero's document preview — a shape, never data. */
const previewDays = [
  { day: "M", height: 74, weekend: false },
  { day: "T", height: 90, weekend: false },
  { day: "W", height: 62, weekend: false },
  { day: "T", height: 96, weekend: false },
  { day: "F", height: 80, weekend: false },
  { day: "S", height: 22, weekend: true },
  { day: "S", height: 8, weekend: true },
];

const heroPoints = ["Hours per day, at a glance", "Team timesheet", "Print, PDF & CSV"];

/** The hub's featured report: a client-ready document, shown as a hero card above the catalog. */
function FeaturedReport({ report }: { report: ReportMeta }) {
  return (
    <Link
      href={`/reports/${report.slug}`}
      className="group block rounded-2xl focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
    >
      <Card variant="interactive" className="overflow-hidden">
        <div aria-hidden className="h-1 bg-primary" />
        <div className="grid gap-6 p-5 sm:p-7 md:grid-cols-[minmax(0,1fr)_minmax(0,20rem)] md:items-center lg:grid-cols-[minmax(0,1fr)_minmax(0,24rem)]">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <Badge tone="primary" className="gap-1">
                <Sparkles aria-hidden className="h-3 w-3" /> Client-ready
              </Badge>
              <Badge tone="muted">{report.category}</Badge>
            </div>
            <h2 className="mt-4 font-display text-xl font-bold tracking-tight text-ink transition-colors group-hover:text-primary sm:text-2xl">
              {report.title}
            </h2>
            <p className="mt-1.5 max-w-xl text-sm leading-relaxed text-muted-foreground">{report.description}</p>
            <ul className="mt-4 flex flex-col gap-1.5 text-sm text-muted-foreground sm:flex-row sm:flex-wrap sm:gap-x-5">
              {heroPoints.map((p) => (
                <li key={p} className="flex items-center gap-1.5">
                  <Check aria-hidden className="h-4 w-4 shrink-0 text-success" /> {p}
                </li>
              ))}
            </ul>
            <span className="mt-5 inline-flex h-10 items-center gap-2 rounded-xl bg-primary px-4 text-sm font-medium text-primary-foreground transition-[filter] group-hover:brightness-105">
              Open report <ArrowRight aria-hidden className="h-4 w-4 transition-transform group-hover:translate-x-0.5" />
            </span>
          </div>

          {/* A miniature of the document: letterhead, KPI tiles and day bars. Purely decorative (not on phones). */}
          <div aria-hidden className="hidden rounded-xl border border-border bg-background/70 p-4 md:block">
            <div className="flex items-center gap-2">
              <span className="h-6 w-6 rounded-md bg-primary" />
              <span className="flex flex-col gap-1">
                <span className="h-2 w-24 rounded-full bg-foreground/15" />
                <span className="h-1.5 w-14 rounded-full bg-primary/40" />
              </span>
              <span className="ml-auto h-1.5 w-12 rounded-full bg-muted-foreground/20" />
            </div>
            <div className="mt-3 grid grid-cols-3 gap-2">
              {[0, 1, 2].map((i) => (
                <span key={i} className="flex flex-col gap-1.5 rounded-lg border border-border/70 bg-card p-2">
                  <span className="h-1.5 w-8 rounded-full bg-muted-foreground/20" />
                  <span className="h-2.5 w-11 rounded-full bg-foreground/25" />
                </span>
              ))}
            </div>
            <div className="mt-3 rounded-lg border border-border/70 bg-card px-3 pb-2 pt-3">
              <div className="relative flex h-20 items-end gap-1.5">
                <span className="absolute inset-x-0 top-[12%] border-t border-dashed border-muted-foreground/40" />
                {previewDays.map((d, i) => (
                  <span
                    key={i}
                    className={cn("flex-1 rounded-t-md", d.weekend ? "bg-primary/30" : "bg-primary")}
                    style={{ height: `${d.height}%` }}
                  />
                ))}
              </div>
              <div className="mt-1.5 flex gap-1.5">
                {previewDays.map((d, i) => (
                  <span key={i} className="flex-1 text-center text-[9px] font-medium text-muted-foreground">
                    {d.day}
                  </span>
                ))}
              </div>
            </div>
          </div>
        </div>
      </Card>
    </Link>
  );
}

const spotlightPoints = ["Work rhythm heat map", "Focus & deep work", "Who works on what", "Team comparisons"];

/** Decorative intensities (0–1) for the spotlight's heat-map preview — a shape, never data. */
const previewRhythm = Array.from({ length: 5 }, (_, row) =>
  Array.from({ length: 14 }, (_, col) => {
    const hour = col + 7;
    const shape = Math.max(0, 1 - Math.abs(hour - (row === 4 ? 11 : 12.5)) / 5.5);
    return Math.round(Math.min(1, shape * (0.65 + ((row * 7 + col * 3) % 5) * 0.09)) * 100) / 100;
  }),
);

/** The hub's analytics spotlight: a dark, chart-rich card for the deep-dive report. */
function SpotlightReport({ report }: { report: ReportMeta }) {
  return (
    <Link
      href={`/reports/${report.slug}`}
      className="group block rounded-2xl focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
    >
      <div className="relative overflow-hidden rounded-2xl border border-white/10 bg-[#0b1530] text-white shadow-lg shadow-primary/10 transition-[box-shadow,transform] duration-300 group-hover:shadow-xl group-hover:shadow-primary/20">
        <div
          aria-hidden
          className="pointer-events-none absolute inset-0 bg-[radial-gradient(60%_90%_at_0%_0%,rgba(0,107,255,0.45)_0%,transparent_60%),radial-gradient(55%_90%_at_100%_100%,rgba(124,58,237,0.45)_0%,transparent_60%)]"
        />
        <div className="relative grid gap-6 p-5 sm:p-7 md:grid-cols-[minmax(0,1fr)_minmax(0,22rem)] md:items-center lg:grid-cols-[minmax(0,1fr)_minmax(0,26rem)]">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <span className="inline-flex items-center gap-1 rounded-full bg-white/12 px-2.5 py-0.5 text-xs font-medium text-white">
                <Sparkles aria-hidden className="h-3 w-3" /> New · Interactive
              </span>
              <span className="rounded-full bg-white/10 px-2.5 py-0.5 text-xs font-medium text-white/85">{report.category}</span>
            </div>
            <h2 className="mt-4 font-display text-xl font-bold tracking-tight sm:text-2xl">{report.title}</h2>
            <p className="mt-1.5 max-w-xl text-sm leading-relaxed text-white/80">{report.description}</p>
            <ul className="mt-4 grid grid-cols-1 gap-1.5 text-sm text-white/85 sm:grid-cols-2">
              {spotlightPoints.map((p) => (
                <li key={p} className="flex items-center gap-1.5">
                  <Check aria-hidden className="h-4 w-4 shrink-0 text-[#7ee8c4]" /> {p}
                </li>
              ))}
            </ul>
            <span className="mt-5 inline-flex h-10 items-center gap-2 rounded-xl bg-white px-4 text-sm font-semibold text-[#0b1530] transition-[filter] group-hover:brightness-95">
              Explore analytics <ArrowRight aria-hidden className="h-4 w-4 transition-transform group-hover:translate-x-0.5" />
            </span>
          </div>

          {/* A miniature work-rhythm heat map and trend line. Purely decorative (not on phones). */}
          <div aria-hidden className="hidden rounded-xl border border-white/10 bg-white/[0.06] p-4 backdrop-blur md:block">
            <div className="grid gap-[3px]" style={{ gridTemplateColumns: "repeat(14, minmax(0, 1fr))" }}>
              {previewRhythm.flatMap((row, r) =>
                row.map((v, c) => (
                  <span
                    key={`${r}-${c}`}
                    className="chart-cell-in aspect-square rounded-[4px]"
                    style={{
                      background: v > 0.05 ? `rgba(96, 165, 250, ${0.18 + v * 0.82})` : "rgba(255,255,255,0.07)",
                      animationDelay: `${c * 30 + r * 60}ms`,
                    }}
                  />
                )),
              )}
            </div>
            <svg viewBox="0 0 200 48" preserveAspectRatio="none" className="mt-3 h-12 w-full">
              <path
                d="M0,40 C20,36 30,18 50,22 S80,38 100,26 S140,6 160,14 S190,30 200,10"
                fill="none"
                stroke="#a78bfa"
                strokeWidth={2.5}
                strokeLinecap="round"
                pathLength={1}
                className="chart-draw"
              />
            </svg>
          </div>
        </div>
      </div>
    </Link>
  );
}

const categoryTone: Record<ReportMeta["category"], "primary" | "success" | "info" | "warning"> = {
  Time: "primary",
  Productivity: "success",
  People: "info",
  Finance: "warning",
};

/** One report of the catalog: the whole card is the link. */
function ReportTile({ report }: { report: ReportMeta }) {
  const Icon = icons[report.icon] ?? FileText;
  return (
    <li>
      <Link
        href={`/reports/${report.slug}`}
        className="group block h-full rounded-2xl focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
      >
        <Card variant="interactive" className="flex h-full flex-col p-5">
          <div className="flex items-start justify-between gap-3">
            <div aria-hidden className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-accent text-accent-foreground">
              <Icon className="h-5 w-5" />
            </div>
            <Badge tone={categoryTone[report.category]}>{report.category}</Badge>
          </div>
          <h3 className="mt-4 font-semibold transition-colors group-hover:text-primary-strong">{report.title}</h3>
          <p className="mt-1 flex-1 text-sm leading-relaxed text-muted-foreground">{report.description}</p>
          <span className="mt-4 inline-flex items-center gap-1 text-xs font-medium text-primary-strong">
            Open report <ArrowRight aria-hidden className="h-3.5 w-3.5 transition-transform group-hover:translate-x-0.5 motion-reduce:transition-none" />
          </span>
        </Card>
      </Link>
    </li>
  );
}

export default function ReportsHubPage() {
  const { user } = useSession();
  const access = useReportAccess();
  // Only what the viewer can open: their role's routes (a client sees the reports shared with clients)
  // and, for pay, the permission to see hourly rates.
  const available = reportCatalog.filter(
    (r) => canAccess(user.role, `/reports/${r.slug}`) && (r.requires !== "manageTeam" || access.manageTeam),
  );
  const featured = available.find((r) => r.featured);
  const spotlight = available.find((r) => r.spotlight);
  // The hero and the spotlight are shown above, not repeated as tiles.
  const catalog = available.filter((r) => !r.featured && !r.spotlight);

  return (
    <PageStack>
      <PageHeader
        eyebrow="Analytics"
        title="Reports"
        description="Hours, activity, attendance, apps, devices and pay across your team — on screen, as CSV or as a PDF."
        actions={
          featured && (
            <Link
              href={`/reports/${featured.slug}`}
              className="inline-flex h-10 items-center gap-2 rounded-xl bg-primary px-4 text-sm font-medium text-primary-foreground transition-[filter] hover:brightness-105 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
            >
              Open {featured.title} <ArrowRight aria-hidden className="h-4 w-4" />
            </Link>
          )
        }
      />

      {featured && <FeaturedReport report={featured} />}
      {spotlight && <SpotlightReport report={spotlight} />}

      {catalog.length > 0 && (
        <section aria-labelledby="all-reports" className="space-y-3">
          <SectionLabel>
            <span id="all-reports">All reports</span>
          </SectionLabel>
          <ul className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {catalog.map((r) => (
              <ReportTile key={r.slug} report={r} />
            ))}
          </ul>
        </section>
      )}

      <Card variant="quiet" className="flex flex-col gap-3 p-5 sm:flex-row sm:items-center">
        <div aria-hidden className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-muted text-muted-foreground">
          <Mail className="h-5 w-5" />
        </div>
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-center gap-2 text-sm font-semibold">
            Scheduled delivery <Badge tone="muted">Coming soon</Badge>
          </p>
          <p className="mt-0.5 text-sm text-muted-foreground">
            Reports can&apos;t be emailed on a schedule yet. Open any report and use Export to download it as a CSV or save it as a PDF.
          </p>
        </div>
      </Card>
    </PageStack>
  );
}
