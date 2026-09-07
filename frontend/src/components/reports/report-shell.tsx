"use client";

import { useEffect, useRef, useState } from "react";
import Link from "next/link";
import type { LucideIcon } from "lucide-react";
import { ArrowLeft, Download, FileText, Printer, ChevronDown } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Select } from "@/components/ui/input";
import { PageHeader } from "@/components/ui/page-header";
import { Toolbar } from "@/components/ui/toolbar";
import { cn } from "@/lib/utils";
import { projects, users } from "@/lib/tenant-data";
import { toast } from "@/components/toast";
import { DateRangePicker, type ResolvedRange } from "./date-range-picker";
import type { RangeKey } from "@/lib/reports-data";

export function ReportShell({
  title,
  description,
  actions,
  children,
}: {
  title: string;
  description?: string;
  actions?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-6">
      <Link href="/reports" className="inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground">
        <ArrowLeft className="h-4 w-4" /> All reports
      </Link>
      <PageHeader title={title} description={description} actions={actions} eyebrow="Report" />
      {children}
    </div>
  );
}

export function FilterBar({
  rangeKey,
  onRange,
  projectId,
  onProject,
  memberId,
  onMember,
  extra,
  actions,
}: {
  rangeKey: RangeKey;
  onRange: (r: ResolvedRange) => void;
  projectId?: string;
  onProject?: (v: string) => void;
  memberId?: string;
  onMember?: (v: string) => void;
  extra?: React.ReactNode;
  actions?: React.ReactNode;
}) {
  return (
    <Toolbar>
      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2 sm:gap-3">
        <DateRangePicker value={rangeKey} onChange={onRange} />
        {onProject && (
          <Select value={projectId} onChange={(e) => onProject(e.target.value)} className="w-full min-w-0 sm:w-auto sm:min-w-40">
            <option value="all">All projects</option>
            {projects.map((p) => (
              <option key={p.id} value={p.id}>{p.title}</option>
            ))}
          </Select>
        )}
        {onMember && (
          <Select value={memberId} onChange={(e) => onMember(e.target.value)} className="w-full min-w-0 sm:w-auto sm:min-w-40">
            <option value="all">All members</option>
            {users.filter((u) => u.role === "owner" || u.role === "admin" || u.role === "worker").map((u) => (
              <option key={u.id} value={u.id}>{u.name}</option>
            ))}
          </Select>
        )}
        {extra}
      </div>
      {actions && (
        <div className="flex w-full items-center gap-2 sm:ml-auto sm:w-auto">{actions}</div>
      )}
    </Toolbar>
  );
}

export function ExportMenu({ onExportCSV }: { onExportCSV: () => void }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, []);

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={() => setOpen((o) => !o)}
        className="inline-flex h-10 w-full items-center justify-center gap-2 rounded-lg bg-primary px-4 text-sm font-medium text-primary-foreground hover:opacity-90 sm:w-auto"
      >
        <Download className="h-4 w-4" /> Export <ChevronDown className="h-4 w-4" />
      </button>
      {open && (
        <div className="absolute right-0 z-30 mt-2 w-44 overflow-hidden rounded-xl border border-border bg-card p-1 card-elev-lg animate-scale-in">
          <button
            onClick={() => { onExportCSV(); setOpen(false); toast({ title: "Export ready", description: "Your CSV has been downloaded.", tone: "success" }); }}
            className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm hover:bg-muted"
          >
            <FileText className="h-4 w-4 text-muted-foreground" /> Download CSV
          </button>
          <button
            onClick={() => { setOpen(false); toast({ title: "Preparing print view", tone: "info" }); setTimeout(() => window.print(), 200); }}
            className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm hover:bg-muted"
          >
            <Printer className="h-4 w-4 text-muted-foreground" /> Print / PDF
          </button>
        </div>
      )}
    </div>
  );
}

export function KpiGrid({ children }: { children: React.ReactNode }) {
  return <div className="grid grid-cols-2 gap-3 sm:gap-4 lg:grid-cols-4">{children}</div>;
}

export function Kpi({
  label,
  value,
  icon: Icon,
  tone = "#0d9488",
  sub,
}: {
  label: string;
  value: string;
  icon?: LucideIcon;
  tone?: string;
  sub?: string;
}) {
  return (
    <Card className="p-4">
      <div className="flex items-center gap-3">
        {Icon && (
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg" style={{ background: `${tone}1a`, color: tone }}>
            <Icon className="h-5 w-5" />
          </div>
        )}
        <div className="min-w-0">
          <div className={cn("truncate text-xl font-bold")}>{value}</div>
          <div className="truncate text-xs text-muted-foreground">{label}</div>
          {sub && <div className="truncate text-[11px] text-muted-foreground">{sub}</div>}
        </div>
      </div>
    </Card>
  );
}
