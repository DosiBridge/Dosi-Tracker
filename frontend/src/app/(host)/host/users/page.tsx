"use client";

import { useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import { Search, LogIn, Users as UsersIcon } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Select } from "@/components/ui/input";
import { DataTable, type Column } from "@/components/ui/data-table";
import { toast } from "@/components/toast";
import { useSession } from "@/components/session-provider";
import { globalUsers, type GlobalUser } from "@/lib/host-data";
import { roleLabels } from "@/lib/roles";
import { formatDuration } from "@/lib/utils";
import type { Role } from "@/lib/types";

const roleTone: Record<string, "primary" | "warning" | "info" | "muted"> = {
  owner: "primary",
  admin: "warning",
  worker: "info",
  client: "muted",
};

export default function HostUsersPage() {
  const router = useRouter();
  const { workspaces, impersonate } = useSession();
  const [query, setQuery] = useState("");
  const [role, setRole] = useState<"all" | Role>("all");
  const [wsId, setWsId] = useState<"all" | string>("all");

  const all = useMemo(() => globalUsers(workspaces), [workspaces]);

  const rows = useMemo(() => {
    return all
      .filter((g) => (role === "all" ? true : g.user.role === role))
      .filter((g) => (wsId === "all" ? true : g.workspaceId === wsId))
      .filter((g) => {
        if (!query.trim()) return true;
        const q = query.toLowerCase();
        return g.user.name.toLowerCase().includes(q) || g.user.email.toLowerCase().includes(q) || g.workspaceName.toLowerCase().includes(q);
      });
  }, [all, role, wsId, query]);

  function enter(g: GlobalUser) {
    impersonate(g.workspaceId);
    toast({ title: `Entering ${g.workspaceName}`, tone: "info" });
    setTimeout(() => router.push("/dashboard"), 300);
  }

  const columns: Column<GlobalUser>[] = [
    {
      key: "name",
      header: "User",
      sortValue: (g) => g.user.name.toLowerCase(),
      render: (g) => (
        <div className="flex items-center gap-2.5">
          <Avatar name={g.user.name} size="sm" status={g.user.status} />
          <div className="min-w-0">
            <div className="truncate font-medium">{g.user.name}</div>
            <div className="truncate text-xs text-muted-foreground">{g.user.email}</div>
          </div>
        </div>
      ),
    },
    {
      key: "workspace",
      header: "Tenant",
      sortValue: (g) => g.workspaceName.toLowerCase(),
      render: (g) => (
        <span className="inline-flex items-center gap-1.5">
          <span className="h-2.5 w-2.5 rounded-full" style={{ background: g.workspaceColor }} />
          {g.workspaceName}
        </span>
      ),
    },
    { key: "role", header: "Role", sortValue: (g) => g.user.role, render: (g) => <Badge tone={roleTone[g.user.role] ?? "muted"}>{roleLabels[g.user.role]}</Badge> },
    { key: "designation", header: "Designation", render: (g) => <span className="text-muted-foreground">{g.user.designation}</span> },
    { key: "productivity", header: "Productivity", align: "right", sortValue: (g) => g.user.productivity, render: (g) => (g.user.role === "client" ? "—" : `${g.user.productivity}%`) },
    { key: "tracked", header: "Tracked today", align: "right", sortValue: (g) => g.user.trackedToday, render: (g) => (g.user.trackedToday ? formatDuration(g.user.trackedToday) : "—") },
    {
      key: "actions",
      header: "",
      align: "right",
      render: (g) => (
        <button
          onClick={() => enter(g)}
          title="Open tenant"
          className="inline-flex h-8 w-8 items-center justify-center rounded-lg border border-border text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <LogIn className="h-4 w-4" />
        </button>
      ),
    },
  ];

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Global Users</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {rows.length} of {all.length} users across every tenant.
          </p>
        </div>
        <Badge tone="muted" className="gap-1.5"><UsersIcon className="h-3.5 w-3.5" /> {all.length} total</Badge>
      </div>

      <Card className="p-3">
        <div className="flex flex-wrap items-center gap-3">
          <div className="relative min-w-0 flex-1">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search name, email or tenant…"
              className="h-10 w-full rounded-lg border border-border bg-background pl-9 pr-3 text-sm focus:outline-none focus:ring-2 focus:ring-ring"
            />
          </div>
          <Select value={wsId} onChange={(e) => setWsId(e.target.value)} className="w-auto min-w-40">
            <option value="all">All tenants</option>
            {workspaces.map((w) => (
              <option key={w.id} value={w.id}>{w.name}</option>
            ))}
          </Select>
          <Select value={role} onChange={(e) => setRole(e.target.value as typeof role)} className="w-auto min-w-36">
            <option value="all">All roles</option>
            <option value="owner">Owner</option>
            <option value="admin">Administrator</option>
            <option value="worker">Member</option>
            <option value="client">Client</option>
          </Select>
        </div>
      </Card>

      <Card className="p-2">
        <DataTable columns={columns} rows={rows} initialSort={{ key: "tracked", dir: "desc" }} emptyText="No users match your filters." />
      </Card>
    </div>
  );
}
