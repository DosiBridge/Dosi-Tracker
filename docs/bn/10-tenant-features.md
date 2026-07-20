# অধ্যায় ১০: Tenant App — Features

> **Status:** Implemented (demo UI under `(dashboard)/`).  
> **As-built:** Mock data; some role scoping bugs — [`../PRODUCTION_READINESS.md`](../PRODUCTION_READINESS.md).  
> **English detail:** [`../../DOCUMENTATION.md`](../../DOCUMENTATION.md) section 10.

> **Canonical:** প্রতিটি tenant page কী করে (feature map)।  
> Roles → [০২](./02-personas-roles.md) · Architecture → [০৭](./07-frontend-architecture.md) · Host settings → [১১](./11-platform-settings.md)

---

## Page map

| Route | Page | Role notes |
|-------|------|------------|
| `/dashboard` | Role-adaptive KPIs | Admin / Worker / Client variants |
| `/projects` | List + create + detail | Per-project tracking permissions |
| `/team` | Members, invite, seats | Invite off at seat limit |
| `/monitor` | Timeline + schedule + tabs | Deepest per-member view |
| `/activities` | Gallery + filters + drawer | |
| `/screenshots` | Gallery + lightbox | |
| `/timesheet` | Weekly heatmap | Scope gap: see readiness |
| `/reports/*` | 7 reports + CSV/PDF | Owner/Admin |
| `/insights` | Auto highlights | |
| `/settings` | Profile, tracking, theme | Tracking = Owner/Admin |
| `/billing` | Plan, usage, invoices | **Owner only** |

---

## Dashboard

- **AdminDashboard:** org KPIs (tracked, active members, avg productivity, screenshots), weekly trend, time-by-project donut, top apps, hourly focus, leaderboard, live feed  
- **WorkerDashboard:** personal stats, own projects, recent activity  
- **ClientDashboard:** assigned projects read-only  

---

## Projects & Team

**Projects:** searchable cards; Create modal (interval + TrackingPermissions); detail KPIs/members/activity।  

**Team:** role filters, status, productivity; Invite modal; seats meter → plan ([০৩](./03-saas-multi-tenancy.md))। Owner দেখে Upgrade path when full।

**Invite security:** Admin যেন Owner role pick না করে — [০২](./02-personas-roles.md), [১৮](./18-security-architecture.md)।

---

## Member Monitor

`monitor/page.tsx` + `monitor-data.ts`:

1. **Timeline** — KPIs, activity band, segments, hourly focus, apps, screenshot strip  
2. **Schedule** — day grid; block → modal (metrics + browser tabs)  
3. **Browser activity** — sites, minutes, category bar  

Productivity categories → [০৯](./09-productivity-model.md)।

---

## Activities & Screenshots

**Activities:** filter bar — search, member, project, sort, date presets, time-of-day, app, productivity slider, live/webcam toggles; chips; detail drawer।  

**Screenshots:** same filter philosophy + lightbox।

---

## Timesheet

Weekly heatmap (members × days), week nav, summary cards।  

**Gap:** Worker “My timesheet” এখনো team-wide হতে পারে — API wire-এর আগে `scope.ts` ঠিক করুন।

---

## Reports suite

| Report | দেখায় |
|--------|--------|
| Time & Activity | Time per member/project |
| Productivity | Split & ranking |
| Apps & Websites | App/site by category |
| Project Breakdown | Time & cost per project |
| Attendance & Shifts | Present/late/remote/absent |
| Weekly Summary | WoW team rollup |
| Payroll & Billing | Billable hours × rates |

Shared: `ReportShell`, filters, `ExportMenu`, print CSS → [১৪](./14-design-system-ux.md)।

---

## Insights & Settings

**Insights:** top performer, most tracked, needs-attention, dominant app, team focus, distraction — `insights()` in `tenant-data.ts`।  

**Settings tabs:** Profile · Tracking (privacy toggles) · Appearance · Notifications।  
Tracking: screenshots, webcam, keyboard/mouse counts, active window, running programs, idle, blur — Owner/Admin। Agent policy align → [১৫](./15-desktop-agent.md)।

---

## Billing (Owner)

Current plan, trial banner, usage meters, payment method, plan comparison, invoice CSV — SaaS math [০৩](./03-saas-multi-tenancy.md)।

---

## Global helpers

- Workspace Switcher — switch/create; remount subtree  
- Topbar — breadcrumb, ⌘K, plan badge → billing, theme, notifications  
- Command palette / toasts → [০৭](./07-frontend-architecture.md)

---

## Host Console (সংক্ষেপ — duplicate নয়)

`(host)/` routes: overview, tenants, plans, users, platform billing।  
পূর্ণ UI টেবিল → [`DOCUMENTATION.md`](../../DOCUMENTATION.md) §11।  
Impersonation → [১২](./12-impersonation.md)।  
Platform SMTP/storage config → [১১](./11-platform-settings.md)।
