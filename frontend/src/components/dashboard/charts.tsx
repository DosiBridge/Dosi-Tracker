"use client";

import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  Cell,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { useIsMdUp } from "@/hooks/use-media-query";
import { cn } from "@/lib/utils";

const axisColor = "#94a3b8";

const tooltipStyle = {
  borderRadius: 12,
  border: "1px solid var(--border)",
  background: "var(--card)",
  color: "var(--card-foreground)",
  fontSize: 12,
  boxShadow: "var(--elev-lg)",
} as const;

function ChartFrame({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("w-full min-w-0", className)}>
      <ResponsiveContainer width="100%" height="100%">
        {children as React.ReactElement}
      </ResponsiveContainer>
    </div>
  );
}

export function ActivityTrendChart({
  data,
}: {
  data: { day: string; tracked: number; productive: number }[];
}) {
  const md = useIsMdUp();
  return (
    <ChartFrame className="h-[200px] sm:h-[240px] lg:h-[260px]">
      <AreaChart data={data} margin={{ top: 10, right: 8, left: md ? -8 : -20, bottom: 0 }}>
        <defs>
          <linearGradient id="gTracked" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#0d9488" stopOpacity={0.4} />
            <stop offset="100%" stopColor="#0d9488" stopOpacity={0} />
          </linearGradient>
          <linearGradient id="gProd" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#16a34a" stopOpacity={0.35} />
            <stop offset="100%" stopColor="#16a34a" stopOpacity={0} />
          </linearGradient>
        </defs>
        <XAxis dataKey="day" tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: md ? 12 : 10 }} />
        <YAxis
          tickLine={false}
          axisLine={false}
          tick={{ fill: axisColor, fontSize: md ? 12 : 10 }}
          width={md ? 40 : 28}
        />
        <Tooltip contentStyle={tooltipStyle} formatter={(v) => `${v} min`} />
        <Area
          type="monotone"
          dataKey="tracked"
          stroke="#0d9488"
          strokeWidth={2.5}
          fill="url(#gTracked)"
          name="Tracked"
        />
        <Area
          type="monotone"
          dataKey="productive"
          stroke="#16a34a"
          strokeWidth={2.5}
          fill="url(#gProd)"
          name="Productive"
        />
      </AreaChart>
    </ChartFrame>
  );
}

export function ProjectDonut({ data }: { data: { name: string; value: number; color: string }[] }) {
  const md = useIsMdUp();
  const total = data.reduce((s, d) => s + d.value, 0);
  const inner = md ? 62 : 44;
  const outer = md ? 92 : 68;
  return (
    <ChartFrame className="mx-auto h-[180px] max-w-[280px] sm:h-[220px] sm:max-w-none">
      <PieChart>
        <Pie
          data={data}
          dataKey="value"
          nameKey="name"
          innerRadius={inner}
          outerRadius={outer}
          paddingAngle={2}
          strokeWidth={0}
        >
          {data.map((d) => (
            <Cell key={d.name} fill={d.color} />
          ))}
        </Pie>
        <Tooltip contentStyle={tooltipStyle} formatter={(v) => `${Math.round((Number(v) / total) * 100)}%`} />
      </PieChart>
    </ChartFrame>
  );
}

function shortLabel(name: string, max = 12) {
  return name.length > max ? `${name.slice(0, max - 1)}…` : name;
}

export function TopAppsChart({ data }: { data: { app: string; minutes: number; color: string }[] }) {
  const md = useIsMdUp();
  return (
    <ChartFrame className="h-[200px] sm:h-[240px]">
      <BarChart
        data={data}
        layout="vertical"
        margin={{ top: 0, right: 8, left: 4, bottom: 0 }}
      >
        <XAxis type="number" hide />
        <YAxis
          type="category"
          dataKey="app"
          tickLine={false}
          axisLine={false}
          width={md ? 100 : 72}
          tick={{ fill: axisColor, fontSize: md ? 12 : 10 }}
          tickFormatter={(v) => shortLabel(String(v), md ? 14 : 10)}
        />
        <Tooltip contentStyle={tooltipStyle} cursor={{ fill: "var(--muted)" }} formatter={(v) => `${v} min`} />
        <Bar dataKey="minutes" radius={[0, 6, 6, 0]} barSize={md ? 16 : 12}>
          {data.map((d) => (
            <Cell key={d.app} fill={d.color} />
          ))}
        </Bar>
      </BarChart>
    </ChartFrame>
  );
}

export function HourlyChart({ data }: { data: { hour: string; minutes: number }[] }) {
  const md = useIsMdUp();
  return (
    <ChartFrame className="h-[160px] sm:h-[180px]">
      <BarChart data={data} margin={{ top: 4, right: 4, left: md ? -8 : -16, bottom: 0 }}>
        <XAxis
          dataKey="hour"
          tickLine={false}
          axisLine={false}
          tick={{ fill: axisColor, fontSize: 10 }}
          interval={md ? 1 : 2}
        />
        <YAxis tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: 10 }} width={md ? 36 : 28} />
        <Tooltip contentStyle={tooltipStyle} cursor={{ fill: "var(--muted)" }} formatter={(v) => `${v} min`} />
        <Bar dataKey="minutes" radius={[4, 4, 0, 0]} fill="#0d9488" barSize={md ? 14 : 10} />
      </BarChart>
    </ChartFrame>
  );
}
