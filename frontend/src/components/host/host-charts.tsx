"use client";

import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  Cell,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type { GrowthPoint } from "@/lib/host-data";

const axisColor = "#94a3b8";
const tooltipStyle = {
  borderRadius: 12,
  border: "1px solid var(--border)",
  background: "var(--card)",
  color: "var(--card-foreground)",
  fontSize: 12,
  boxShadow: "var(--elev-lg)",
} as const;

export function MrrChart({ data }: { data: GrowthPoint[] }) {
  return (
    <ResponsiveContainer width="100%" height={260}>
      <AreaChart data={data} margin={{ top: 10, right: 8, left: -8, bottom: 0 }}>
        <defs>
          <linearGradient id="gMrr" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#6d5efc" stopOpacity={0.4} />
            <stop offset="100%" stopColor="#6d5efc" stopOpacity={0} />
          </linearGradient>
        </defs>
        <XAxis dataKey="month" tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: 12 }} />
        <YAxis tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: 12 }} tickFormatter={(v) => `$${Number(v) >= 1000 ? `${Math.round(Number(v) / 1000)}k` : v}`} />
        <Tooltip contentStyle={tooltipStyle} formatter={(v) => [`$${Number(v).toLocaleString()}`, "MRR"]} />
        <Area type="monotone" dataKey="mrr" stroke="#6d5efc" strokeWidth={2.5} fill="url(#gMrr)" name="MRR" />
      </AreaChart>
    </ResponsiveContainer>
  );
}

export function TenantsChart({ data }: { data: GrowthPoint[] }) {
  return (
    <ResponsiveContainer width="100%" height={220}>
      <BarChart data={data} margin={{ top: 4, right: 4, left: -22, bottom: 0 }}>
        <XAxis dataKey="month" tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: 11 }} />
        <YAxis tickLine={false} axisLine={false} tick={{ fill: axisColor, fontSize: 11 }} allowDecimals={false} />
        <Tooltip contentStyle={tooltipStyle} cursor={{ fill: "var(--muted)" }} formatter={(v) => [`${v}`, "Tenants"]} />
        <Bar dataKey="tenants" radius={[4, 4, 0, 0]} fill="#0ea5e9" barSize={16}>
          {data.map((d, i) => (
            <Cell key={i} fill={i === data.length - 1 ? "#6d5efc" : "#0ea5e9"} />
          ))}
        </Bar>
      </BarChart>
    </ResponsiveContainer>
  );
}
