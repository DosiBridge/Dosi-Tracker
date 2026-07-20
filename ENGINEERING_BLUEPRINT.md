# Dosi-Tracker — Engineering Blueprint (English index)

> **Target engineering course (Bengali, chapters 1–23):**  
> [`docs/bn/README.md`](./docs/bn/README.md)
>
> **As-built / production pack (English):**  
> [`docs/AS_BUILT.md`](./docs/AS_BUILT.md) · [`docs/PRODUCTION_READINESS.md`](./docs/PRODUCTION_READINESS.md) · [`docs/RUNBOOK.md`](./docs/RUNBOOK.md) · [`docs/SECURITY.md`](./docs/SECURITY.md)
>
> **Product / UI / roles / demo:** [`DOCUMENTATION.md`](./DOCUMENTATION.md)
>
> **Bengali entry shortcut:** [`ENGINEERING_BLUEPRINT.bn.md`](./ENGINEERING_BLUEPRINT.bn.md)

This file is an **index only** — teaching text lives under `docs/bn/`; production truth under `docs/AS_BUILT.md`.

**Note:** Bengali ch. **11** = Host platform **settings** (SMTP, storage). `DOCUMENTATION.md` §11 = Host Console **UI** (covered in bn ch. **10** + English doc).

---

## Chapter map (full course 1–23)

### Part A — Product & frontend (1–14)

| Ch | Topic | File |
|---:|-------|------|
| 1 | Product Vision & Idea | [`docs/bn/01-product-vision.md`](./docs/bn/01-product-vision.md) |
| 2 | Personas & Roles | [`docs/bn/02-personas-roles.md`](./docs/bn/02-personas-roles.md) |
| 3 | SaaS & Multi-Tenancy | [`docs/bn/03-saas-multi-tenancy.md`](./docs/bn/03-saas-multi-tenancy.md) |
| 4 | System Architecture | [`docs/bn/04-system-architecture.md`](./docs/bn/04-system-architecture.md) |
| 5 | Technology Stack | [`docs/bn/05-technology-stack.md`](./docs/bn/05-technology-stack.md) |
| 6 | Repository Layout | [`docs/bn/06-repository-layout.md`](./docs/bn/06-repository-layout.md) |
| 7 | Frontend Architecture | [`docs/bn/07-frontend-architecture.md`](./docs/bn/07-frontend-architecture.md) |
| 8 | Domain & Data Model | [`docs/bn/08-domain-data-model.md`](./docs/bn/08-domain-data-model.md) |
| 9 | Productivity Model | [`docs/bn/09-productivity-model.md`](./docs/bn/09-productivity-model.md) |
| 10 | Tenant App Features | [`docs/bn/10-tenant-features.md`](./docs/bn/10-tenant-features.md) |
| 11 | Platform Settings (Host config) | [`docs/bn/11-platform-settings.md`](./docs/bn/11-platform-settings.md) |
| 12 | Impersonation Flow | [`docs/bn/12-impersonation.md`](./docs/bn/12-impersonation.md) |
| 13 | Multi-Tenant Isolation (FE) | [`docs/bn/13-multi-tenant-isolation.md`](./docs/bn/13-multi-tenant-isolation.md) |
| 14 | Design System & UX | [`docs/bn/14-design-system-ux.md`](./docs/bn/14-design-system-ux.md) |

### Part B — Engineering & ops (15–23)

| Ch | Topic | File |
|---:|-------|------|
| 15 | Desktop Agent | [`docs/bn/15-desktop-agent.md`](./docs/bn/15-desktop-agent.md) |
| 16 | Database Design | [`docs/bn/16-database-design.md`](./docs/bn/16-database-design.md) |
| 17 | API Documentation | [`docs/bn/17-api-documentation.md`](./docs/bn/17-api-documentation.md) |
| 18 | Security Architecture | [`docs/bn/18-security-architecture.md`](./docs/bn/18-security-architecture.md) |
| 19 | DevOps, Docker & CI/CD | [`docs/bn/19-devops-cicd.md`](./docs/bn/19-devops-cicd.md) |
| 20 | Testing | [`docs/bn/20-testing.md`](./docs/bn/20-testing.md) |
| 21 | Deployment | [`docs/bn/21-deployment.md`](./docs/bn/21-deployment.md) |
| 22 | Monitoring | [`docs/bn/22-monitoring.md`](./docs/bn/22-monitoring.md) |
| 23 | Roadmap | [`docs/bn/23-roadmap.md`](./docs/bn/23-roadmap.md) |
| — | Glossary (appendix) | [`docs/bn/24-glossary.md`](./docs/bn/24-glossary.md) |

**Ownership rule:** see the table in [`docs/bn/README.md`](./docs/bn/README.md). Each chapter has a **Status** banner.

---

## One-line architecture (target)

`Desktop Agent → HTTPS API (.NET 10 + ABP) → PostgreSQL / Object Storage ← Web Dashboard (Tenant + Host)`

**Today:** agents and dashboard are **not** fully wired to product APIs — see [`docs/AS_BUILT.md`](./docs/AS_BUILT.md).

---

## Repository

```
backend/     .NET 10 + ABP (scaffold + Identity)
frontend/    Next.js (mock demo)
clients/     windows (Rust), macos (Swift) — partial
docs/bn/     full course ch. 1–23 (Bengali)
docs/        AS_BUILT, PRODUCTION_READINESS, RUNBOOK, SECURITY
```

---

*Teaching text → `docs/bn/`. Shipping truth → `docs/AS_BUILT.md`.*
