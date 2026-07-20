# অধ্যায় ১৩: Multi-Tenant Data Isolation (Frontend)

> **Status:** Implemented (demo — `tenant-data.ts`).  
> **As-built:** Frontend-only; backend product tables + filter later — [`../AS_BUILT.md`](../AS_BUILT.md).

> **Canonical:** `datasetFor`, live bindings, remount, user lookup।  
> SaaS → [০৩](./03-saas-multi-tenancy.md) · DB TenantId → [১৬](./16-database-design.md) · Impersonation → [১২](./12-impersonation.md)

---

## লক্ষ্য

Backend ছাড়াই demo **যেন real SaaS**: workspace বদলালে **সম্পূর্ণ আলাদা world** — users, projects, activities, invoices।

---

## পাঁচ ধাপ (`tenant-data.ts`)

### 1. Dataset
এক workspace-এর full bundle: users, projects, activities, charts, attendance, billing, notifications।

### 2. `datasetFor(workspaceId)`

| ID | Source |
|----|--------|
| `w1` | Exact seed `mock-data.ts` |
| `w2`, `w3` | Deterministic generated rosters |
| নতুন create | Empty starter workspace |

Results cached।

### 3. Live bindings
`export let users, projects, activities, …` = **active** tenant।  
`setActiveWorkspace(id)` সব binding swap করে।

### 4. Remount
`SessionProvider` → `setActiveWorkspace(workspace.id)`।  
`(dashboard)/layout.tsx`: `key={workspace.id}` on `<main>` → subtree **full remount** — stale hooks এড়ানো।

### 5. Cross-tenant user resolution
Lookup order:

```
[hostUser, primaryUsers, datasetFor(activeWorkspace).users]
```

Impersonation-এ tenant owner এখান থেকে resolve — [১২](./12-impersonation.md)।

---

## Role scoping (`scope.ts`)

Tenant isolation ≠ member privacy।

| Layer | কাজ |
|-------|-----|
| Tenant switch | অন্য কোম্পানির ডেটা দেখাবে না |
| `scope.ts` | Worker শুধু নিজের rows |
| `RoleGuard` | Route-এ ঢোকা |

দুটোই লাগবে। Known gaps → [`../PRODUCTION_READINESS.md`](../PRODUCTION_READINESS.md)।

---

## Demo persistence

| Key pattern | অর্থ |
|-------------|------|
| `dosi-user` | Current user |
| `dosi-workspace` | Active workspace |
| `dosi-workspaces-*` | Created / patches / deleted |
| `dosi-impersonating` | Host impersonation flag |

Production: API source of truth; `localStorage` শুধু UX prefs।

---

## API wire-এ একই নীতি

| Demo | Production |
|------|------------|
| `setActiveWorkspace` | JWT `tenantid` + server filter |
| Client dataset | Never trust body `tenantId` |
| Remount on switch | Invalidate React Query / SWR caches |

Tenant leak tests → [২০](./20-testing.md)।
