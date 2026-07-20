"use client";

import { useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import { LogIn, Users as UsersIcon } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Select } from "@/components/ui/input";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { SearchField, Toolbar } from "@/components/ui/toolbar";
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
          className="inline-flex h-8 w-8 items-center justify-center rounded-xl border border-border text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <LogIn className="h-4 w-4" />
        </button>
      ),
    },
  ];

  return (
    <PageStack>
      <PageHeader
        eyebrow="Host"
        title="Global Users"
        description={`${rows.length} of ${all.length} users across every tenant.`}
        actions={<Badge tone="muted" className="gap-1.5"><UsersIcon className="h-3.5 w-3.5" /> {all.length} total</Badge>}
      />

      <Toolbar>
        <SearchField value={query} onChange={setQuery} placeholder="Search name, email or tenant…" />
        <Select value={wsId} onChange={(e) => setWsId(e.target.value)} className="w-full sm:w-auto sm:min-w-40">
          <option value="all">All tenants</option>
          {workspaces.map((w) => (
            <option key={w.id} value={w.id}>{w.name}</option>
          ))}
        </Select>
        <Select value={role} onChange={(e) => setRole(e.target.value as typeof role)} className="w-full sm:w-auto sm:min-w-36">
          <option value="all">All roles</option>
          <option value="owner">Owner</option>
          <option value="admin">Administrator</option>
          <option value="worker">Member</option>
          <option value="client">Client</option>
        </Select>
      </Toolbar>

      <Card className="p-2">
        <DataTable columns={columns} rows={rows} initialSort={{ key: "tracked", dir: "desc" }} emptyText="No users match your filters." />
      </Card>
    </PageStack>
  );
}
