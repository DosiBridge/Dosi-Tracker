# অধ্যায় ১৪: Design System & UX

> **Status:** Implemented (`globals.css`, `components/ui/*`, shared page chrome).  
> **As-built:** Fully in frontend demo.

> **Canonical:** Tokens, brand, components, charts, print, responsive, page chrome.  
> Frontend structure → [০৭](./07-frontend-architecture.md) · Features → [১০](./10-tenant-features.md)

---

## Design tokens

`globals.css` — light + dark CSS variables:

| Group | Examples |
|-------|----------|
| Surface | `background`, `card`, `muted`, `border` |
| Brand | ink + teal primary (not purple SaaS) |
| Semantic | `success`, `warning`, `danger`, `info` |
| Elevation | soft shadow tokens |
| Rhythm | `--page-gap`, `--control-h`, `--radius-control` |

Typography: **Outfit** (UI) + **Syne** (display / `.page-title`).

Tailwind v4: `@theme inline` maps tokens to utilities।

---

## Shared page chrome

Use these instead of one-off headers/filters:

| Primitive | Path | Role |
|-----------|------|------|
| `PageHeader` / `PageStack` / `SectionLabel` | `components/ui/page-header.tsx` | Title, eyebrow, actions |
| `Toolbar` / `SearchField` / `SegmentedControl` | `components/ui/toolbar.tsx` | List filters |
| `Surface` + `Card variant` | `components/ui/surface.tsx`, `card.tsx` | `default` · `quiet` · `interactive` · `inset` |
| `EmptyState` | `components/ui/empty-state.tsx` | Empty lists |

Every tenant feature screen should share this language so the product feels one system.

---

## Charts

Recharts wrappers themed to tokens:

- `components/dashboard/charts.tsx` — tenant  
- `components/host/host-charts.tsx` — MRR, plan donut, growth  

---

## Feedback UX

| Tool | Use |
|------|-----|
| Toasts | create / invite / save |
| Command palette | ⌘K navigation |
| Notifications panel | topbar events |

---

## Print & export

Reports: **CSV** (`export.ts`) + **Print-to-PDF** (`@media print` hides chrome)।

---

## Responsiveness

| Area | Behavior |
|------|----------|
| Sidebar | Mobile off-canvas; desktop icon collapse |
| Grids | `grid-cols-1` → breakpoints |
| Wide tables / heatmap / monitor | Container `overflow-x-auto` only |
| Modals | Full-width on small screens |
| Global | `body { overflow-x: hidden }`, `overflow-wrap: break-word` |

Mobile-first; tenant + host verified।

---

## Dark mode

`ThemeProvider` + `ThemeToggle`: light / dark / system।  
Settings → Appearance; preference `dosi-theme` in `localStorage`।
