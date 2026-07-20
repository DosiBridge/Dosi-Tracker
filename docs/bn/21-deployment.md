# অধ্যায় ২১: Deployment

> **Status:** Target design (IIS/cloud go-live).  
> **As-built:** Local Host + DbMigrator only — [`../RUNBOOK.md`](../RUNBOOK.md).

> **Canonical:** IIS/Cloud/DNS/SSL go-live।  
> Compose/CI → [১৯](./19-devops-cicd.md) · Monitoring after go-live → [২২](./22-monitoring.md)

---

## টার্গেট টপোলজি

```
Users/Agents → DNS+TLS → LB/IIS/Nginx → Web + API(+Hangfire)
                      → PostgreSQL, Redis, Object Storage, SMTP
```

`app.` = frontend · `api.` = backend।

---

## Pre-flight checklist

**DNS/TLS:** app + api hosts, cert auto-renew, optional `*.dositracker.app`  
**Data:** DB, Redis, private bucket+IAM, backup ON  
**Secrets:** JWT, SMTP verified, CORS = app origin only  
**Hardening:** Hangfire dashboard locked, Swagger off public prod  
**People:** on-call, rollback owner, status channel  

---

## অপশন A — Windows Server + IIS

1. .NET 10 Hosting Bundle + IIS + WebSocket/ARR as needed  
2. Publish API → site `api.` 443, app pool low-priv  
3. Env/vault bind  
4. **একবার** DbMigrator  
5. Frontend static বা Node+proxy  
6. Verify `/health/ready` + `/login`  

SQL Server চাইলে EF provider মেলান — schema নীতি [১৬](./16-database-design.md)।

---

## অপশন B — Cloud containers

CI image → registry → migrator job → API ≥2 replicas → Web → Ingress TLS → smoke।  
HPA: CPU + ingest lag।

Compose দিয়ে ছোট VPSও চলে — ফাইল [১৯](./19-devops-cicd.md)।

---

## Redis (runtime)

Cache, Data Protection keys, SignalR backplane, rate counters, optional Hangfire store।  
AUTH+TLS। Memory alert। Keys সবদা tenant-prefixed ([০৫](./05-technology-stack.md) সতর্কতা)।

---

## Hangfire (runtime)

Queues: `critical`, `default`, `email`। Dashboard host-only + IP allow-list।  
Failed → alert ([২২](./22-monitoring.md))। Job তালিকা/cron source of truth [১১](./11-platform-settings.md)।

---

## SignalR (runtime)

Multi-node: Redis backplane (সুপারিশ) বা sticky sessions।  
Nginx/IIS-এ WebSocket upgrade headers।

---

## SSL ও DNS

| Host | Target |
|------|--------|
| app.dositracker.com | Web |
| api.dositracker.com | API |
| *.dositracker.app | tenant apps (optional) |

HTTP→HTTPS redirect। SPF/DKIM/DMARC — নাহলে invite spam।  
Cert expiry alert [২২](./22-monitoring.md)।

---

## প্রথম go-live ক্রম

```
Infra → Secrets → Migrator → API → Web →
Smoke login → Smoke agent ingest → Smoke host overview →
Enable alerts → Announce
```

### পরবর্তী release

CI green → staging → smoke → approve → prod rolling → watch Grafana 30m।

---

## Rollback রানবুক

1. Incident declare  
2. Previous image ([১৯](./19-devops-cicd.md))  
3. DB restore শুধু যদি migration break করে (approved)  
4. Health verify  
5. Timeline/postmortem  

---

## Deploy শেষে monitoring সংযোগ

`/health/live` + `/health/ready` LB-এ; logs/metrics ship; test alert once।  
বিস্তারিত ড্যাশবোর্ড [২২](./22-monitoring.md) — এখানে ডুপ্লিকেট নয়।

---

## সারসংক্ষেপ

Checklist → migrator → API/Web → Redis/Hangfire/SignalR prod knobs → smoke।  
CI/Docker আগের অধ্যায়; observability পরের অধ্যায়।

→ [অধ্যায় ২২ — Monitoring](./22-monitoring.md)
