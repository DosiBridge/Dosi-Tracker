"use client";

import {
  Sparkles,
  TrendingUp,
  AlertTriangle,
  Info,
  ShieldAlert,
  Clock,
  Trophy,
  ArrowRight,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import Link from "next/link";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Ring } from "@/components/ui/ring";
import { ActivityTrendChart } from "@/components/dashboard/charts";
import { attendance, insights, users, weeklyTrend } from "@/lib/tenant-data";
import { type Insight } from "@/lib/reports-data";
import { formatDuration } from "@/lib/utils";

const toneIcon: Record<Insight["tone"], LucideIcon> = {
  success: TrendingUp,
  warning: AlertTriangle,
  danger: ShieldAlert,
  info: Info,
};
const toneColor: Record<Insight["tone"], string> = {
  success: "#22c55e",
  warning: "#f59e0b",
  danger: "#ef4444",
  info: "#0ea5e9",
};

export default function InsightsPage() {
  const cards = insights();
  const active = users.filter((u) => u.productivity > 0);
  const top = [...active].sort((a, b) => b.productivity - a.productivity).slice(0, 3);
  const attention = [...active].sort((a, b) => a.productivity - b.productivity).slice(0, 3);

  const lateCount = attendance.filter((a) => a.status === "late").length;
  const absentCount = attendance.filter((a) => a.status === "absent").length;

  const anomalies = [
    { icon: Clock, tone: "#f59e0b", title: `${lateCount} late arrivals this week`, detail: "Above the usual baseline of 3–4." },
    { icon: ShieldAlert, tone: "#ef4444", title: `${absentCount} unplanned absences`, detail: "Consider reviewing shift coverage." },
    { icon: TrendingUp, tone: "#22c55e", title: "Friday was the most productive day", detail: `${formatDuration(weeklyTrend[4].productive)} of focused time across the team.` },
  ];

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <div className="flex h-11 w-11 items-center justify-center rounded-xl brand-gradient text-white">
          <Sparkles className="h-5 w-5" />
        </div>
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Insights</h1>
          <p className="text-sm text-muted-foreground">Automated highlights and anomalies from your team&apos;s activity.</p>
        </div>
      </div>

      {/* Insight cards */}
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {cards.map((c) => {
          const Icon = toneIcon[c.tone];
          return (
            <Card key={c.id} className="p-4">
              <div className="flex items-start gap-3">
                <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg" style={{ background: `${toneColor[c.tone]}1a`, color: toneColor[c.tone] }}>
                  <Icon className="h-5 w-5" />
                </div>
                <div className="min-w-0">
                  <div className="text-sm font-semibold">{c.title}</div>
                  <div className="mt-0.5 text-xs text-muted-foreground">{c.detail}</div>
                </div>
              </div>
            </Card>
          );
        })}
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        {/* Top performers */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2"><Trophy className="h-4 w-4 text-warning" /> Top performers</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            {top.map((u, i) => (
              <div key={u.id} className="flex items-center gap-3">
                <span className="w-4 text-sm font-bold text-muted-foreground">{i + 1}</span>
                <Avatar name={u.name} size="sm" status={u.status} />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium">{u.name}</div>
                  <div className="truncate text-xs text-muted-foreground">{u.designation}</div>
                </div>
                <Ring value={u.productivity} size={40} />
              </div>
            ))}
          </CardContent>
        </Card>

        {/* Needs attention */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2"><AlertTriangle className="h-4 w-4 text-danger" /> Needs attention</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            {attention.map((u) => (
              <div key={u.id} className="flex items-center gap-3">
                <Avatar name={u.name} size="sm" status={u.status} />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium">{u.name}</div>
                  <div className="truncate text-xs text-muted-foreground">{formatDuration(u.trackedToday)} tracked today</div>
                </div>
                <Badge tone={u.productivity >= 70 ? "success" : u.productivity >= 55 ? "warning" : "danger"}>{u.productivity}%</Badge>
              </div>
            ))}
          </CardContent>
        </Card>

        {/* Anomalies */}
        <Card>
          <CardHeader><CardTitle>Anomalies</CardTitle></CardHeader>
          <CardContent className="space-y-3">
            {anomalies.map((a, i) => (
              <div key={i} className="flex items-start gap-3">
                <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg" style={{ background: `${a.tone}1a`, color: a.tone }}>
                  <a.icon className="h-4 w-4" />
                </div>
                <div className="min-w-0">
                  <div className="text-sm font-medium">{a.title}</div>
                  <div className="text-xs text-muted-foreground">{a.detail}</div>
                </div>
              </div>
            ))}
          </CardContent>
        </Card>
      </div>

      {/* Focus trend */}
      <Card>
        <CardHeader>
          <CardTitle>Team focus trend</CardTitle>
          <Link href="/reports/weekly" className="inline-flex items-center gap-1 text-sm text-primary hover:underline">
            Weekly report <ArrowRight className="h-4 w-4" />
          </Link>
        </CardHeader>
        <CardContent>
          <ActivityTrendChart data={weeklyTrend} />
        </CardContent>
      </Card>
    </div>
  );
}
