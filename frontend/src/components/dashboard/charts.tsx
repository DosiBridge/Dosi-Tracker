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

const axisColor = "#94a3b8";

const tooltipStyle = {
  borderRadius: 12,
  border: "1px solid var(--border)",
  background: "var(--card)",
  color: "var(--card-foreground)",
  fontSize: 12,
  boxShadow: "var(--elev-lg)",
} as const;

export function ActivityTrendChart({
  data,
}: {
  data: { day: string; tracked: number; productive: number }[];
}) {
  return (
    <ResponsiveContainer width="100%" height={260}>
      <AreaChart data={data} margin={{ top: 10, right: 8, left: -18, bottom: 0 }}>
        <defs>
          <linearGradient id="gTracked" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#6d5efc" stopOpacity={0.4} />
            <stop offset="100%" stopColor="#6d5efc" stopOpacity={0} />
          </linearGradient>
          <linearGradient id="gProd" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#22c55e" stopOpacity={0.35} />
            <stop offset="100%" stopColor="#22c55e" stopOpacity={0} />
          </linearGradient>
        </defs>
        <XAxis dataKey="day" tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: 12 }} />
        <YAxis tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: 12 }} />
        <Tooltip contentStyle={tooltipStyle} formatter={(v) => `${v} min`} />
        <Area
          type="monotone"
          dataKey="tracked"
          stroke="#6d5efc"
          strokeWidth={2.5}
          fill="url(#gTracked)"
          name="Tracked"
        />
        <Area
          type="monotone"
          dataKey="productive"
          stroke="#22c55e"
          strokeWidth={2.5}
          fill="url(#gProd)"
          name="Productive"
        />
      </AreaChart>
    </ResponsiveContainer>
  );
}

export function ProjectDonut({ data }: { data: { name: string; value: number; color: string }[] }) {
  const total = data.reduce((s, d) => s + d.value, 0);
  return (
    <ResponsiveContainer width="100%" height={220}>
      <PieChart>
        <Pie
          data={data}
          dataKey="value"
          nameKey="name"
          innerRadius={62}
          outerRadius={92}
          paddingAngle={2}
          strokeWidth={0}
        >
          {data.map((d) => (
            <Cell key={d.name} fill={d.color} />
          ))}
        </Pie>
        <Tooltip contentStyle={tooltipStyle} formatter={(v) => `${Math.round((Number(v) / total) * 100)}%`} />
      </PieChart>
    </ResponsiveContainer>
  );
}

export function TopAppsChart({ data }: { data: { app: string; minutes: number; color: string }[] }) {
  return (
    <ResponsiveContainer width="100%" height={240}>
      <BarChart data={data} layout="vertical" margin={{ top: 0, right: 12, left: 8, bottom: 0 }}>
        <XAxis type="number" hide />
        <YAxis
          type="category"
          dataKey="app"
          tickLine={false}
          axisLine={false}
          width={110}
          tick={{ fill: axisColor, fontSize: 12 }}
        />
        <Tooltip contentStyle={tooltipStyle} cursor={{ fill: "var(--muted)" }} formatter={(v) => `${v} min`} />
        <Bar dataKey="minutes" radius={[0, 6, 6, 0]} barSize={16}>
          {data.map((d) => (
            <Cell key={d.app} fill={d.color} />
          ))}
        </Bar>
      </BarChart>
    </ResponsiveContainer>
  );
}

export function HourlyChart({ data }: { data: { hour: string; minutes: number }[] }) {
  return (
    <ResponsiveContainer width="100%" height={180}>
      <BarChart data={data} margin={{ top: 4, right: 4, left: -22, bottom: 0 }}>
        <XAxis dataKey="hour" tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: 10 }} interval={1} />
        <YAxis tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: 10 }} />
        <Tooltip contentStyle={tooltipStyle} cursor={{ fill: "var(--muted)" }} formatter={(v) => `${v} min`} />
        <Bar dataKey="minutes" radius={[4, 4, 0, 0]} fill="#6d5efc" barSize={14} />
      </BarChart>
    </ResponsiveContainer>
  );
}
