# অধ্যায় ৭: Frontend Architecture

> **Status:** Implemented (Next.js demo — no API wire).  
> **As-built:** Mock session + `tenant-data.ts` — [`../AS_BUILT.md`](../AS_BUILT.md).

> **Canonical:** App Router structure, providers, data layers, wire plan।  
> Features → [১০](./10-tenant-features.md) · Isolation → [১৩](./13-multi-tenant-isolation.md) · Design → [১৪](./14-design-system-ux.md)

---

## Route groups

```
frontend/src/app/
├── layout.tsx                 # Root: fonts, ThemeProvider, SessionProvider
├── page.tsx                   # Entry redirect
├── login/page.tsx             # Sign in, demo accounts, Host login
├── (dashboard)/               # Tenant App — role-scoped
│   ├── layout.tsx             # Sidebar, Topbar, RoleGuard, impersonation banner
│   └── dashboard, projects, team, monitor, activities,
│       screenshots, timesheet, reports, insights, billing, settings
└── (host)/                    # Host Console — host role only
    ├── layout.tsx             # HostSidebar, HostTopbar, host guard
    └── host, tenants, plans, users, billing
```

Login এক পেজে: tenant demo users + create workspace + Host login।

---

## Cross-cutting providers

| Component | ফাইল | দায়িত্ব |
|-----------|------|----------|
| **SessionProvider** | `session-provider.tsx` | user, workspace, CRUD workspace, host login, impersonation; `localStorage` |
| **RoleGuard** | `role-guard.tsx` | `canAccess(role, path)` — [০২](./02-personas-roles.md) |
| **ThemeProvider** | `theme-provider.tsx` | light/dark via `.dark` + CSS variables |
| **Command Palette** | `command-palette.tsx` | ⌘K/Ctrl+K, role-filtered |
| **Toasts** | `toast.tsx` | `toast({...})` event-driven |
| **UI kit** | `components/ui/*` | Button, Card, Modal, Drawer, DataTable, … |

### SessionProvider API (মনে রাখার মতো)

`setUserById`, `setWorkspaceById`, `createWorkspace`, `updateWorkspace`, `deleteWorkspace`,  
`isHost`, `isImpersonating`, `impersonate`, `stopImpersonating`, `loginAsHost`।

Active workspace বদলালে `setActiveWorkspace` + layout `key={workspace.id}` remount — [১৩](./13-multi-tenant-isolation.md)।

---

## Data layers (`frontend/src/lib`)

| File | দায়িত্ব |
|------|----------|
| `types.ts` | `Role`, `User`, `Project`, `Activity`, … |
| `mock-data.ts` | Primary tenant `w1` seed + fixed `NOW` |
| `tenant-data.ts` | Per-tenant dataset + live bindings |
| `saas-data.ts` | Plans, workspaces, usage, invoices |
| `host-data.ts` | Platform roll-ups (MRR, tenants, …) |
| `monitor-data.ts` | Day timeline, browser tabs |
| `reports-data.ts` | Report catalog + helpers |
| `roles.ts` | Labels, nav access, landing |
| `scope.ts` | Role-based project/activity filter |
| `activity-filters.ts` | Activities page filters |
| `export.ts` / `utils.ts` | CSV, formatting |

Domain shapes → [০৮](./08-domain-data-model.md)।

---

## Demo runtime choices

| Choice | কেন |
|--------|-----|
| Fixed `NOW` clock | SSR/client hydration match |
| Minutes-from-midnight (monitor) | Timezone bug কমায় |
| Synthetic `screen-mock` | Binary screenshot asset ship করা যায় না |
| Backend-optional | Product explore without API |

---

## API wire plan (লক্ষ্য)

Mock shape ≈ intended API — incremental swap:

1. Auth → OpenIddict / ABP Account ([১৮](./18-security-architecture.md))  
2. One read path (e.g. activities list) off mock  
3. Keep DTO shapes → [১৭](./17-api-documentation.md)  
4. Feature flag: `USE_MOCK` until vertical slice green  

**রাখবেন:** `key={workspace.id}` remount; never trust client `tenantId` in body।
