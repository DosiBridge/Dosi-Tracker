"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { Users as UsersIcon } from "lucide-react";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Avatar } from "@/components/ui/avatar";
import { Select } from "@/components/ui/input";
import { PageHeader, PageStack } from "@/components/ui/page-header";
import { SearchField, Toolbar } from "@/components/ui/toolbar";
import { DataTable, type Column } from "@/components/ui/data-table";
import { useSession } from "@/components/session-provider";
import { globalUsers } from "@/lib/host-data";
import { getApi } from "@/hooks/useApi";

/** Unified row shape for both the live (API) and demo (fallback) views. */
interface Row {
  id: string;
  name: string;
  email: string;
  tenant: string;
}

/** Shape of GET /api/app/platform/users items. */
interface ApiUser {
  id: string;
  userName: string;
  email?: string | null;
  name?: string | null;
  tenantId?: string | null;
  tenantName?: string | null;
}

export default function HostUsersPage() {
  const { workspaces } = useSession();
  const [query, setQuery] = useState("");
  const [tenant, setTenant] = useState<"all" | string>("all");

  const [live, setLive] = useState<Row[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);

  const loadLive = useCallback(async () => {
    setLoading(true);
    setError(false);
    try {
      const res = (await getApi("/api/app/platform/users?MaxResultCount=500")) as { items?: ApiUser[] };
      const items = Array.isArray(res?.items) ? res.items : [];
      setLive(
        items.map((u) => ({
          id: u.id,
          name: (u.name && u.name.trim()) || u.userName,
          email: u.email ?? "",
          tenant: u.tenantName || "Host",
        }))
      );
    } catch {
      setLive(null); // fall back to the demo dataset
      setError(true);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (typeof window === "undefined" || !localStorage.getItem("dosi-token")) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- data fetch defers its own setState; see docs/QUALITY.md §10
    loadLive();
  }, [loadLive]);

  const mockRows = useMemo<Row[]>(
    () =>
      globalUsers(workspaces).map((g) => ({
        id: g.user.id,
        name: g.user.name,
        email: g.user.email,
        tenant: g.workspaceName,
      })),
    [workspaces]
  );

  const all = live ?? mockRows;
  const tenants = useMemo(() => [...new Set(all.map((r) => r.tenant))].sort(), [all]);

  const rows = useMemo(
    () =>
      all
        .filter((r) => (tenant === "all" ? true : r.tenant === tenant))
        .filter((r) => {
          if (!query.trim()) return true;
          const q = query.toLowerCase();
          return r.name.toLowerCase().includes(q) || r.email.toLowerCase().includes(q) || r.tenant.toLowerCase().includes(q);
        }),
    [all, tenant, query]
  );

  const columns: Column<Row>[] = [
    {
      key: "name",
      header: "User",
      sortValue: (r) => r.name.toLowerCase(),
      render: (r) => (
        <div className="flex items-center gap-2.5">
          <Avatar name={r.name} size="sm" />
          <div className="min-w-0">
            <div className="truncate font-medium">{r.name}</div>
            <div className="truncate text-xs text-muted-foreground">{r.email || "—"}</div>
          </div>
        </div>
      ),
    },
    {
      key: "tenant",
      header: "Tenant",
      sortValue: (r) => r.tenant.toLowerCase(),
      render: (r) => <span className="text-muted-foreground">{r.tenant}</span>,
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

      {loading && live === null && (
        <div className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">Loading users…</div>
      )}
      {error && live === null && (
        <div className="rounded-lg border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">
          Live users unavailable right now — showing demo data.
        </div>
      )}

      <Toolbar>
        <SearchField value={query} onChange={setQuery} placeholder="Search name, email or tenant…" />
        <Select value={tenant} onChange={(e) => setTenant(e.target.value)} className="w-full sm:w-auto sm:min-w-40">
          <option value="all">All tenants</option>
          {tenants.map((t) => (
            <option key={t} value={t}>{t}</option>
          ))}
        </Select>
      </Toolbar>

      <Card className="p-2">
        <DataTable columns={columns} rows={rows} initialSort={{ key: "name", dir: "asc" }} emptyText="No users match your filters." />
      </Card>
    </PageStack>
  );
}
