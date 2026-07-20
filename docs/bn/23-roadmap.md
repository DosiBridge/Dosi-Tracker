# অধ্যায় ২৩: Roadmap & Scaling

> **Status:** Target roadmap.  
> **Prioritized gaps:** [`../PRODUCTION_READINESS.md`](../PRODUCTION_READINESS.md) | Product roadmap also in [`../../DOCUMENTATION.md`](../../DOCUMENTATION.md) section 22.

> **Canonical:** ভবিষ্যৎ scale/roadmap। আজকের arch → [০৪](./04-system-architecture.md)।

---

## আজকের সঠিক ভিত্তি

Modular monolith (ABP layered) + Next.js + native agents + এক primary region।  
দিন-১ microservices নয়।

---

## Microservices — কখন

| প্রার্থী service | কেন আলাদা |
|------------------|----------|
| Ingest | Agent traffic আলাদা scale |
| Reporting | heavy CPU/IO |
| Billing | compliance boundary |
| Notification | burst email/push |

Migrate যখন bottleneck/টিম/blast-radius স্পষ্ট — তাড়াতাড়ি ভাঙলে ops খরচ বাড়ে।

---

## Kubernetes

HPA, rolling update, ConfigMaps/Secrets, Ingress TLS — Compose-এর পরবর্তী ধাপ ([১৯](./19-devops-cicd.md))।

---

## AI Analytics (ভবিষ্যৎ)

Anomaly productivity, smarter insights, unknown-app category, NL reports।  
Training/inference **tenant-isolated**; opt-in।

---

## Mobile App

Manager companion — একই API ([১৭](./17-api-documentation.md))। নতুন Backend নয়।

---

## Offline sync উন্নতি

Conflict UI, resume large uploads, metered-network mode — ভিত্তি ইতিমধ্যে [১৫](./15-desktop-agent.md)।

---

## Multi-region / CDN / SaaS scale

| Goal | Approach |
|------|----------|
| Latency | regional API + agent pin |
| Residency | tenant↔region affinity |
| DR | async replica + runbook |
| CDN | static assets; private shots stay presigned |
| Noisy neighbor | per-tenant rate limits |
| Extreme | shard by tenant hash |

---

## HA ও Disaster Recovery

- Multi-AZ DB, ≥2 API instances, Redis HA  
- Document RPO/RTO (উদাহরণ: RPO 1h, RTO 4h)  
- Restore drill ([২২](./22-monitoring.md))  
- Status page templates  

---

## Product roadmap (ক্রম)

1. Frontend mock → real ABP APIs  
2. Real auth + subdomain tenants  
3. Payments (Stripe etc.)  
4. Finish Rust/Swift capture pipelines  
5. SignalR live feed  
6. Enforce retention/storage caps  
7. Public API + webhooks  
8. Mobile  
9. AI insights  
10. Integrations marketplace  

---

## চিরকালীন engineering নীতি (সংক্ষেপ — বিস্তারিত ছড়িয়ে অধ্যায়ে)

| নীতি | Canonical অধ্যায় |
|------|-------------------|
| API-mediated writes | ০৪ |
| Tenant filter | ১৬ + ১৮ |
| Tests in CI | ২০ + ১৯ |
| Observability | ২২ |
| Secrets outside git | ১৯ |
| Incremental change | ১৯ zero-downtime |

---

## সারসংক্ষেপ

এই অধ্যায় **ভবিষ্যৎ**। আজকের বিস্তারিত বাস্তবায়ন ০৪–২২-এ একবার করে আছে —
এখানে আর কপি নয়।

← [`README.md`](./README.md) সূচি
