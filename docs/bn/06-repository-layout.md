# অধ্যায় ৬: Repository Layout

> **Status:** Implemented (monorepo structure).  
> **As-built:** [`../AS_BUILT.md`](../AS_BUILT.md) · Run → [`../RUNBOOK.md`](../RUNBOOK.md).

> **Canonical:** ফোল্ডার মানে কী, কোথায় কাজ করবেন।  
> Frontend → [০৭](./07-frontend-architecture.md) · Backend layers → [১৭](./17-api-documentation.md) · Agent → [১৫](./15-desktop-agent.md)

---

## Monorepo overview

```
Dosi-Tracker/
├── backend/              # .NET 10 + ABP, PostgreSQL
├── frontend/             # Next.js — Tenant App + Host Console
├── clients/
│   ├── windows/          # Rust agent
│   └── macos/            # Swift agent
├── docs/
│   ├── bn/               # এই কোর্স (বাংলা) 01–24
│   ├── AS_BUILT.md
│   ├── PRODUCTION_READINESS.md
│   ├── RUNBOOK.md
│   └── SECURITY.md
├── DOCUMENTATION.md      # Product SSOT (English)
├── ENGINEERING_BLUEPRINT.md
├── ENGINEERING_BLUEPRINT.bn.md
└── README.md
```

প্রতিটি component-এর নিজস্ব README: `backend/`, `frontend/`, `clients/windows/`, `clients/macos/`।

---

## কোথায় কী করবেন

| কাজ | ফোল্ডার |
|-----|---------|
| UI page / component | `frontend/src/app/`, `components/` |
| Demo data / types / roles | `frontend/src/lib/` |
| Domain entity | `backend/src/Dosi.Tracker.Domain/` |
| DTO + IAppService | `Application.Contracts/` |
| App service impl | `Application/` |
| EF + migrations | `EntityFrameworkCore/` |
| Host / OpenIddict / Serilog | `HttpApi.Host/` |
| One-shot migrate + seed | `DbMigrator/` |
| Windows capture/sync | `clients/windows/src/` |
| macOS capture/sync | `clients/macos/Sources/` |

---

## Backend ABP layers

```
Domain.Shared → Domain → Application.Contracts
                      → Application
                      → EntityFrameworkCore
                      → HttpApi.Host
```

নতুন feature ক্রম: Entity → Consts → Migration → DTO → AppService → (frontend wire)।  
Template rules → `backend/.cursor/rules/`।

---

## Clients layout (সংক্ষেপ)

| | Windows | macOS |
|-|---------|-------|
| Language | Rust | Swift |
| Config | `config.example.toml` | `config.example.json` |
| Modules | tracking/, storage, api, sync | Tracking/, Storage, ApiClient |

বিস্তারিত → [১৫](./15-desktop-agent.md)।

---

## Docs ownership (ডুপ্লিকেট এড়াতে)

| প্রশ্ন | ফাইল |
|--------|------|
| Product / UI কী? | `DOCUMENTATION.md` |
| Engineering course? | `docs/bn/` |
| Shipped vs planned? | `docs/AS_BUILT.md` |
| Production gaps? | `docs/PRODUCTION_READINESS.md` |
| Glossary | [২৪](./24-glossary.md) |
