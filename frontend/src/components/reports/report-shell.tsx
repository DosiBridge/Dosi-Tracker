"use client";

import { useEffect, useRef, useState } from "react";
import Link from "next/link";
import type { LucideIcon } from "lucide-react";
import { ArrowLeft, Download, FileText, Printer, ChevronDown } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Select } from "@/components/ui/input";
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
      <Link href="/reports" className="inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground">
        <ArrowLeft className="h-4 w-4" /> All reports
      </Link>
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">{title}</h1>
          {description && <p className="mt-1 text-sm text-muted-foreground">{description}</p>}
        </div>
        {actions && <div className="flex items-center gap-2">{actions}</div>}
      </div>
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
    <Card className="p-3">
      <div className="flex flex-wrap items-center gap-3">
        <DateRangePicker value={rangeKey} onChange={onRange} />
        {onProject && (
          <Select value={projectId} onChange={(e) => onProject(e.target.value)} className="w-auto min-w-40">
            <option value="all">All projects</option>
            {projects.map((p) => (
              <option key={p.id} value={p.id}>{p.title}</option>
            ))}
          </Select>
        )}
        {onMember && (
          <Select value={memberId} onChange={(e) => onMember(e.target.value)} className="w-auto min-w-40">
            <option value="all">All members</option>
            {users.filter((u) => u.role !== "client").map((u) => (
              <option key={u.id} value={u.id}>{u.name}</option>
            ))}
          </Select>
        )}
        {extra}
        <div className="ml-auto flex items-center gap-2">{actions}</div>
      </div>
    </Card>
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
        className="inline-flex h-10 items-center gap-2 rounded-lg bg-primary px-4 text-sm font-medium text-primary-foreground hover:opacity-90"
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
  return <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">{children}</div>;
}

export function Kpi({
  label,
  value,
  icon: Icon,
  tone = "#6d5efc",
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
          {sub && <div className="truncate text-[11px] text-muted-foreground/70">{sub}</div>}
        </div>
      </div>
    </Card>
  );
}
