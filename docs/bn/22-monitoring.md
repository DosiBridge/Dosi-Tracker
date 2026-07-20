# অধ্যায় ২২: Monitoring & Observability

> **Status:** Partial + target.  
> **As-built:** Serilog + `/health-status` (+ HealthChecks UI). No Grafana/Prometheus/Hangfire dashboard in repo — [`../AS_BUILT.md`](../AS_BUILT.md).

> **Canonical:** Serilog, health, metrics, alerts।  
> Backup config → [১১](./11-platform-settings.md) · Deploy → [২১](./21-deployment.md)

---

## Monitoring ছাড়া Production?

তখন আপনি উড়োজাহাজ উড়াচ্ছেন যন্ত্রপাতি বন্ধ করে।

চাই:

1. **Logs** — ঘটনার কাহিনি  
2. **Metrics** — সংখ্যা ও ট্রেন্ড  
3. **Traces** — ধীর request-এর মানচিত্র  
4. **Alerts** — ঘুম থেকে তোলা (শুধু সত্যি জরুরি)  

---

## Serilog (Structured Logging)

আনস্ট্রাকচার্ড টেক্সট লগ খুঁজতে কষ্ট। Structured:

```csharp
Log.Information(
  "Activity ingested {ClientActivityId} Tenant={TenantId} User={UserId} Ms={ElapsedMs}",
  clientId, tenantId, userId, elapsed);
```

### Levels
| Level | কখন |
|-------|-----|
| Debug | dev only |
| Information | business events |
| Warning | recoverable (retry) |
| Error | failed request/job |
| Fatal | process cannot continue |

### Sinks
- Console  
- Rolling file  
- Seq / Elasticsearch / Loki / CloudWatch  

### নিয়ম
- Token/password কখনো লগ নয়  
- Screenshot URL সাইন মাস্ক  
- প্রতি request `CorrelationId` / `TraceId`  
- TenantId সব business লগে  

---

## Prometheus Metrics

স্ক্র্যাপ উদাহরণ:

```
http_requests_total{code,method,endpoint}
http_request_duration_seconds_bucket
activities_ingested_total{tenant}
activities_ingest_errors_total
hangfire_jobs_failed
db_pool_busy
redis_connected
agent_devices_online_estimate
```

.NET-এ `prometheus-net` বা OpenTelemetry metrics exporter।

---

## Grafana Dashboards

কমপক্ষে ৪টা ড্যাশবোর্ড:

### 1. API Overview
- RPS, p50/p95/p99 latency  
- 4xx/5xx rate  
- Instance count  

### 2. Ingest Pipeline
- Activities/min  
- Batch size  
- Error ratio  
- Queue lag (যদি proxy queue থাকে)  

### 3. Dependencies
- PostgreSQL CPU/connections  
- Redis memory/hit rate  
- Storage error rate  
- SMTP success/fail  

### 4. Business (optional)
- Active tenants  
- Seats used  
- Trial ending soon  

---

## OpenTelemetry Tracing

একটি Agent POST:

```
agent → api.middleware → auth → ActivityAppService → EF INSERT → S3 PUT
```

প্রতিটি span timing দেখায়। ধীর INSERT নাকি S3 — তাৎক্ষণিক বোঝা যায়।

W3C Trace Context header forward করুন। Frontend থেকেও (ঐচ্ছিক) RUM।

---

## Health Checks

```csharp
// ধারণা
live  → always 200 if process up
ready → DB ping + Redis ping + (optional) storage head bucket
```

| Check | Fail হলে |
|-------|----------|
| live | restart container |
| ready | LB traffic বন্ধ |

Hangfire আলাদা `ready` dependency হতে পারে।

---

## Alerting — কীতে ঘুম ভাঙাবেন

| Alert | Severity | Condition উদাহরণ |
|-------|----------|------------------|
| API 5xx | critical | >2% for 5m |
| Ingest stalled | critical | rate == 0 while devices > 0 for 10m |
| DB connections | high | >90% pool |
| Disk | high | <15% |
| Backup failed | high | job != success last 26h |
| Cert expiry | high | <21 days |
| Hangfire failed spike | medium | >N failed/15m |

### Alert hygiene
- Runbook লিংক প্রতি alert-এ  
- Night noise কমান (business hours vs 24/7)  
- Auto-resolve  

Slack/Email/PagerDuty।

---

## Performance Monitoring

Metrics ছাড়াও:

- Slow query log (>500ms)  
- APM (App Insights / Jaeger UI)  
- Frontend: Web Vitals (LCP, INP, CLS)  
- Agent: sync duration histogram  

মাসিক “perf review”: top 10 slow endpoints ঠিক করুন।

---

## Backup Monitoring

Backup job “scheduled” ≠ “restorable”।

চেক:
1. Last success timestamp Grafana-এ  
2. Object size > 0  
3. Quarterly **restore to staging** drill  
4. Offsite / another region copy  

Restore drill ছাড়া DR মিথ্যা নিরাপত্তা।

---

## Log Analysis — Incident খেলার ধাপ

1. Alert দেখুন  
2. CorrelationId / TraceId ধরুন  
3. একই সময় metrics spike?  
4. Recent deploy? (yes → rollback বিবেচনা)  
5. Tenant-specific নাকি global?  
6. Fix → verify → postmortem (ব্লেমলেস)  

Template:

```
Summary:
Impact:
Detection:
Timeline:
Root cause:
Action items:
```

---

## Maintenance Routines

| কখন | কাজ |
|-----|-----|
| প্রতিদিন | alert inbox, failed jobs, disk |
| প্রতি সপ্তাহ | patch staging, dependency PRs |
| প্রতি মাস | capacity (DB size, storage $), slow query review |
| প্রতি কোয়ার্টার | restore drill, access rights review, secret rotation |
| প্রতি বছর | DR tabletop exercise |

Maintenance Mode (অধ্যায় ১১) বড় upgrade-এ ব্যবহার — Agent offline queue কাজে লাগে।

---

## On-call ও Runbooks

প্রতিটি critical alert-এর জন্য এক পৃষ্ঠার runbook:

```
Alert: API 5xx spike
1. Check Grafana API dashboard
2. Check last deploy
3. Check dependency health (DB/Redis)
4. If bad deploy → rollback
5. If DB → failover/scale
6. Notify status channel
```

নতুন engineer প্রথম সপ্তাহে runbook পড়ে staging-এ rehearsal করুক।

---

## এই অধ্যায়ের সারসংক্ষেপ

1. Logs (Serilog) + Metrics (Prometheus/Grafana) + Traces (OTel)।  
2. Health live/ready আলাদা।  
3. Alert actionable + runbook।  
4. Backup verify = restore drill।  
5. নিয়মিত maintenance ছাড়া সিস্টেম আস্তে আস্তে ক্ষয় হয়।  

→ পরবর্তী: [অধ্যায় ২৩ — Roadmap](./23-roadmap.md)
