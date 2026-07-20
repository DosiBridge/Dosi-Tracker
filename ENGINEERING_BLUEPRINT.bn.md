# Dosi-Tracker — ইঞ্জিনিয়ারিং ব্লুপ্রিন্ট (বাংলা সূচি)

> **বিস্তারিত কোর্স (অধ্যায় ১–২৩):** [`docs/bn/README.md`](./docs/bn/README.md)
>
> **কোডে যা আছে / প্রোডাকশন:**  
> [`docs/AS_BUILT.md`](./docs/AS_BUILT.md) · [`docs/PRODUCTION_READINESS.md`](./docs/PRODUCTION_READINESS.md) · [`docs/RUNBOOK.md`](./docs/RUNBOOK.md) · [`docs/SECURITY.md`](./docs/SECURITY.md)
>
> **প্রোডাক্ট/UI (English):** [`DOCUMENTATION.md`](./DOCUMENTATION.md)  
> **ইংরেজি সূচি:** [`ENGINEERING_BLUEPRINT.md`](./ENGINEERING_BLUEPRINT.md)

এই ফাইল শুধু **সূচি** — পূর্ণ পাঠ `docs/bn/`-এ একবারই লেখা।

**নোট:** বাংলা অধ্যায় **১১** = Host **platform settings** (SMTP, storage)।  
`DOCUMENTATION.md` §11 = Host Console **UI** → বাংলা [১০](./docs/bn/10-tenant-features.md) + English doc।

---

## সূচিপত্র

### ভাগ ক — প্রোডাক্ট ও ফ্রন্টএন্ড (১–১৪)

| অধ্যায় | বিষয় | ফাইল |
|-------:|------|------|
| ১ | Product Vision | [`docs/bn/01-product-vision.md`](./docs/bn/01-product-vision.md) |
| ২ | Personas & Roles | [`docs/bn/02-personas-roles.md`](./docs/bn/02-personas-roles.md) |
| ৩ | SaaS & Multi-Tenancy | [`docs/bn/03-saas-multi-tenancy.md`](./docs/bn/03-saas-multi-tenancy.md) |
| ৪ | System Architecture | [`docs/bn/04-system-architecture.md`](./docs/bn/04-system-architecture.md) |
| ৫ | Technology Stack | [`docs/bn/05-technology-stack.md`](./docs/bn/05-technology-stack.md) |
| ৬ | Repository Layout | [`docs/bn/06-repository-layout.md`](./docs/bn/06-repository-layout.md) |
| ৭ | Frontend Architecture | [`docs/bn/07-frontend-architecture.md`](./docs/bn/07-frontend-architecture.md) |
| ৮ | Domain & Data Model | [`docs/bn/08-domain-data-model.md`](./docs/bn/08-domain-data-model.md) |
| ৯ | Productivity Model | [`docs/bn/09-productivity-model.md`](./docs/bn/09-productivity-model.md) |
| ১০ | Tenant Features | [`docs/bn/10-tenant-features.md`](./docs/bn/10-tenant-features.md) |
| ১১ | Platform Settings | [`docs/bn/11-platform-settings.md`](./docs/bn/11-platform-settings.md) |
| ১২ | Impersonation | [`docs/bn/12-impersonation.md`](./docs/bn/12-impersonation.md) |
| ১৩ | Multi-Tenant Isolation | [`docs/bn/13-multi-tenant-isolation.md`](./docs/bn/13-multi-tenant-isolation.md) |
| ১৪ | Design System & UX | [`docs/bn/14-design-system-ux.md`](./docs/bn/14-design-system-ux.md) |

### ভাগ খ — ইঞ্জিনিয়ারিং ও অপস (১৫–২৩)

| অধ্যায় | বিষয় | ফাইল |
|-------:|------|------|
| ১৫ | Desktop Agent | [`docs/bn/15-desktop-agent.md`](./docs/bn/15-desktop-agent.md) |
| ১৬ | Database Design | [`docs/bn/16-database-design.md`](./docs/bn/16-database-design.md) |
| ১৭ | API Documentation | [`docs/bn/17-api-documentation.md`](./docs/bn/17-api-documentation.md) |
| ১৮ | Security | [`docs/bn/18-security-architecture.md`](./docs/bn/18-security-architecture.md) |
| ১৯ | DevOps & CI/CD | [`docs/bn/19-devops-cicd.md`](./docs/bn/19-devops-cicd.md) |
| ২০ | Testing | [`docs/bn/20-testing.md`](./docs/bn/20-testing.md) |
| ২১ | Deployment | [`docs/bn/21-deployment.md`](./docs/bn/21-deployment.md) |
| ২২ | Monitoring | [`docs/bn/22-monitoring.md`](./docs/bn/22-monitoring.md) |
| ২৩ | Roadmap | [`docs/bn/23-roadmap.md`](./docs/bn/23-roadmap.md) |
| — | Glossary (পরিশিষ্ট) | [`docs/bn/24-glossary.md`](./docs/bn/24-glossary.md) |

---

## এক লাইনে architecture (লক্ষ্য)

`Desktop Agent → HTTPS API (.NET 10 + ABP) → PostgreSQL / Object Storage ← Web Dashboard (Tenant + Host)`

আজকের বাস্তবতা: [`docs/AS_BUILT.md`](./docs/AS_BUILT.md)।
