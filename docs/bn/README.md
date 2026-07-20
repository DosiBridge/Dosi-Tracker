# Dosi-Tracker — বাংলা ইঞ্জিনিয়ারিং কোর্স

> **একমাত্র বিস্তারিত টার্গেট কোর্স** এই ফোল্ডার। প্রতিটি বিষয় **একবারই** লেখা —
> অন্য ফাইলে শুধু লিঙ্ক।
>
> প্রতিটি অধ্যায়ের উপরে **Status** ব্যানার আছে: Target / Partial / As-built লিঙ্ক।

| ডক | ভূমিকা |
|----|--------|
| প্রোডাক্ট/UI (English, বিস্তারিত UI) | [`../../DOCUMENTATION.md`](../../DOCUMENTATION.md) |
| **কোডে যা আছে** | [`../AS_BUILT.md`](../AS_BUILT.md) |
| **প্রোডাকশন গ্যাপ** | [`../PRODUCTION_READINESS.md`](../PRODUCTION_READINESS.md) |
| রানবুক | [`../RUNBOOK.md`](../RUNBOOK.md) |
| সিকিউরিটি | [`../SECURITY.md`](../SECURITY.md) |
| ইংরেজি সূচি | [`../../ENGINEERING_BLUEPRINT.md`](../../ENGINEERING_BLUEPRINT.md) |

**গুরুত্বপূর্ণ:** অধ্যায় **১–১৪** = প্রোডাক্ট + ফ্রন্টএন্ড কোর্স। **১৫–২৩** = ব্যাকএন্ড/অ্যাজেন্ট/অপস ইঞ্জিনিয়ারিং।  
অধ্যায় **১১** এখানে = Host **platform settings** (SMTP, storage) — `DOCUMENTATION.md` §11 Host Console **UI** → [১০](./10-tenant-features.md) + English doc।

---

## কীভাবে পড়বেন (ডুপ্লিকেট এড়াতে)

| বিষয় | শুধু এই অধ্যায়ে বিস্তারিত | অন্য জায়গায় |
|------|---------------------------|---------------|
| Vision, তিন অংশ, নীতি | **০১** | Arch → ০৪ |
| Roles, access matrix | **০২** | Security → ১৮ |
| Workspace, plan, billing math | **০৩** | DB tenant → ১৬ |
| সিস্টেমের ৪ অংশ, data flow | **০৪** | — |
| কোন টুল কেন | **০৫** | বাস্তবায়ন → ১৫+ |
| Monorepo ফোল্ডার | **০৬** | — |
| Next.js routes, providers, lib | **০৭** | Features → ১০ |
| Domain types, ER | **০৮** | SQL → ১৬ |
| Productivity score, categories | **০৯** | — |
| Tenant pages (dashboard…billing) | **১০** | Host UI table → DOCUMENTATION §11 |
| Host global config (SMTP, storage…) | **১১** | — |
| Impersonation | **১২** | Isolation → ১৩ |
| Frontend tenant isolation | **১৩** | DB filter → ১৬ |
| Design tokens, UX, responsive | **১৪** | — |
| Agent কোড/SQLite/sync | **১৫** | Arch → ০৪ |
| টেবিল, ER, index, TenantId | **১৬** | — |
| REST endpoints, Swagger | **১৭** | — |
| JWT, RBAC, OWASP | **১৮** | — |
| Docker, CI/CD | **১৯** | Go-live → ২১ |
| Tests | **২০** | — |
| IIS/Cloud/DNS/SSL | **২১** | Compose → ১৯ |
| Serilog, Grafana, alerts | **২২** | Backup config → ১১ |
| Scale / roadmap | **২৩** | Product roadmap → DOCUMENTATION §22 |

**নিয়ম:** একই উদাহরণ/টেবিল দুই অধ্যায়ে পুরো কপি নয় — এক জায়গায় লিখে বাকি জায়গায় “দেখুন অধ্যায় X”।

---

## সম্পূর্ণ সূচিপত্র (১–২৩)

### ভাগ ক — প্রোডাক্ট ও ফ্রন্টএন্ড (০১–১৪)

| অধ্যায় | ফাইল | As-built (সংক্ষেপ) |
|-------:|------|-------------------|
| ১ | [01-product-vision.md](./01-product-vision.md) | Vision + demo UI |
| ২ | [02-personas-roles.md](./02-personas-roles.md) | Client-side roles |
| ৩ | [03-saas-multi-tenancy.md](./03-saas-multi-tenancy.md) | Mock SaaS data |
| ৪ | [04-system-architecture.md](./04-system-architecture.md) | Target overview |
| ৫ | [05-technology-stack.md](./05-technology-stack.md) | Partial stack |
| ৬ | [06-repository-layout.md](./06-repository-layout.md) | Monorepo ✓ |
| ৭ | [07-frontend-architecture.md](./07-frontend-architecture.md) | Mock, no API |
| ৮ | [08-domain-data-model.md](./08-domain-data-model.md) | Types only |
| ৯ | [09-productivity-model.md](./09-productivity-model.md) | Demo logic |
| ১০ | [10-tenant-features.md](./10-tenant-features.md) | Full demo UI |
| ১১ | [11-platform-settings.md](./11-platform-settings.md) | Mock / target |
| ১২ | [12-impersonation.md](./12-impersonation.md) | Demo session |
| ১৩ | [13-multi-tenant-isolation.md](./13-multi-tenant-isolation.md) | Frontend only |
| ১৪ | [14-design-system-ux.md](./14-design-system-ux.md) | Implemented |

### ভাগ খ — ইঞ্জিনিয়ারিং ও অপস (১৫–২৩)

| অধ্যায় | ফাইল | As-built (সংক্ষেপ) |
|-------:|------|-------------------|
| ১৫ | [15-desktop-agent.md](./15-desktop-agent.md) | Partial agent |
| ১৬ | [16-database-design.md](./16-database-design.md) | Abp* only |
| ১৭ | [17-api-documentation.md](./17-api-documentation.md) | Product APIs missing |
| ১৮ | [18-security-architecture.md](./18-security-architecture.md) | OpenIddict + gaps |
| ১৯ | [19-devops-cicd.md](./19-devops-cicd.md) | No CI/compose |
| ২০ | [20-testing.md](./20-testing.md) | Target |
| ২১ | [21-deployment.md](./21-deployment.md) | Local only |
| ২২ | [22-monitoring.md](./22-monitoring.md) | Serilog + health |
| ২৩ | [23-roadmap.md](./23-roadmap.md) | Target |
| — | [24-glossary.md](./24-glossary.md) | পরিভাষা (পরিশিষ্ট) |

---

## পড়ার ক্রম (সুপারিশ)

1. **০১ → ০৩** — কেন, কে, SaaS  
2. **০৪ → ০৫** — architecture + stack  
3. **০৬ → ১৪** — repo, frontend, domain, features (demo explore)  
4. **১৫ → ২৩** — build, ship, secure, scale  
5. **২৪** — glossary যখন শব্দ অপরিচিত  

বাস্তব শিপ স্ট্যাটাস: [`../PRODUCTION_READINESS.md`](../PRODUCTION_READINESS.md)।
