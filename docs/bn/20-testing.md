# অধ্যায় ২০: Testing Strategy

> **Status:** Target design (test pyramid).  
> **As-built:** No product-domain test suite gate in CI (CI absent) — [`../PRODUCTION_READINESS.md`](../PRODUCTION_READINESS.md)

> **Canonical:** Unit → Integration → API → UI → Load → Security tests।

---

## কেন Testing আলাদা অধ্যায়?

Dosi-Tracker-এ সংবেদনশীল জিনিস আছে:

- Screenshot  
- অন্যের activity দেখা (Admin) vs না দেখা (Worker)  
- Multi-tenant isolation  
- Offline queue duplicate  

একটি বাগ মানে reputational + legal ঝুঁকি। তাই test কৌশল বাধ্যতামূলক।

---

## Test Pyramid বিস্তারিত

```
            /\
           /E2E\          খুব কম — login, create project, ingest visible
          /------\
         /  API   \       permission matrix, validation, idempotency
        /----------\
       / Integration \    EF + PostgreSQL Testcontainers, Hangfire hooks
      /--------------\
     /   Unit tests    \  domain, scoring, filters, pure functions — সবচেয়ে বেশি
    /--------------------\
```

নীচে যত বেশি test, CI তত দ্রুত feedback। উপরে flaky হলেও coverage আছে।

---

## Unit Testing

### Backend উদাহরণ

| Subject | Assert |
|---------|--------|
| `Activity` entity | EndedAt <= StartedAt → exception |
| Productivity helper | split productive/neutral/unproductive |
| `canAccess(role, path)` | client cannot `/billing` |
| Idempotency key parser | same key → same hash |

xUnit উদাহরণ ধারণা:

```csharp
[Fact]
public void Worker_Cannot_Access_Billing()
{
    canAccess("worker", "/billing").Should().BeFalse();
    canAccess("owner", "/billing").Should().BeTrue();
}
```

### Frontend unit

- `scopeActivities(worker, list)` শুধু নিজের id  
- `scopeActivities(client, list)` শুধু নিজের project  
- Report date range helpers  

Vitest/Jest + Testing Library।

### Agent unit

- Idle detector threshold  
- Queue retry backoff curve  
- Payload serializer  

---

## Integration Testing

বাস্তব (বা Testcontainers) PostgreSQL দিয়ে:

1. Create tenant A + user  
2. Insert activity as A  
3. Login as tenant B  
4. Query activities → **খালি** (isolation)  
5. Invitation creates user with role worker  
6. DbMigrator on empty DB succeeds  

ABP `AbpIntegratedTest` বা WebApplicationFactory + container।

---

## API Testing

### Auth flows
- Wrong password → 401  
- Refresh reuse after rotation → 401 family revoke  
- Expired access → 401  

### Authorization matrix (বাধ্যতামূলক টেবিল)

| Caller | GET /activities?userId=OTHER | Expected |
|--------|------------------------------|----------|
| owner | yes | 200 |
| admin | yes | 200 |
| worker | yes | 403 |
| client | other project | 403 |
| host (not impersonating) | tenant API | 403 |

### Activity ingest
- Valid batch → 200/201  
- Duplicate `clientActivityId` → 200 same id (idempotent)  
- Future StartedAt absurd → 400  
- Spoofed tenantId in body ignored  

### Error shape
সব error এক envelope — contract test।

Tools: `WebApplicationFactory`, RestSharp, Newman (Postman CI)।

---

## UI Testing

Playwright scenarios:

1. Login as Owner → Dashboard KPIs দেখা যায়  
2. Login as Admin → `/billing` RoleGuard  
3. Login as Worker → Activities শুধু নিজের; Timesheet scoped (fix পর)  
4. Login as Client → Projects nav নেই; dashboard project progress  
5. Host → Tenants → Impersonate → banner → Return to Host  
6. Create workspace from switcher → empty state  

Stable selectors (`data-testid`) ব্যবহার করুন। শুধু class name নয়।

---

## Load Testing

লক্ষ্য: অনেক Agent একসাথে।

k6 স্ক্রিপ্ট ধারণা:

```
1000 virtual agents
each: POST /api/v1/activities/batch every 30s
duration: 15m
thresholds:
  http_req_failed < 1%
  http_req_duration{p95} < 500ms
```

দেখুন:

- DB CPU  
- connection pool exhaustion  
- Redis memory  
- Hangfire backlog  

Bottleneck পেলে: batch size, index, horizontal API pods।

---

## Performance Testing

Load ≠ Performance। Performance মানে একক/কয়েক request-এর দক্ষতা।

চেকলিস্ট:

- Reports-এ N+1 (EF `Include` / projection)  
- `EXPLAIN ANALYZE` slow queries  
- Screenshot list pagination default limit  
- Dashboard cache hit ratio  
- Frontend bundle size / LCP  

---

## Security Testing

| Test | Tool/Method |
|------|-------------|
| Dependency CVE | `dotnet list package --vulnerable`, `npm audit` |
| Secrets in repo | gitleaks |
| DAST baseline | OWASP ZAP |
| Permission fuzz | scripted matrix above |
| Tenant spoof | body/header manipulation tests |
| Upload abuse | huge file, exe rename as jpg |

MFA/captcha enterprise পর্যায়ে যোগ।

---

## Automation Testing (CI-তে বাঁধা)

| Trigger | Suite |
|---------|-------|
| Every PR | unit + integration + api smoke + frontend unit |
| Main | + docker build + playwright smoke |
| Nightly | full load + zap baseline |
| Pre-release | full E2E on staging |

Fail হলে merge block। Flaky test quarantine করুন — ignore করে সবুজ দেখানো নিষেধ।

---

## Test Data Management

- Seed deterministic (fixed clock)  
- Factory: `ActivityFactory.Create(tenant, user)`  
- কোনো production DB-তে test নয়  
- PII fake (faker)  

Frontend demo `NOW = 2026-07-14…` — একই ধারণা backend test-এ `IClock` fake।

---

## Definition of Done (DoD) — ফিচার শেষ?

একটি ফিচার “শেষ” তখনই যখন:

1. Unit tests আছে  
2. Permission tests আছে  
3. Happy + failure path API test  
4. (UI হলে) একটা Playwright path  
5. Docs/changelog আপডেট  

---

## এই অধ্যায়ের সারসংক্ষেপ

1. Pyramid অনুসরণ — বেশি unit, মাঝারি integration/API, কম E2E।  
2. Multi-tenant ও RBAC tests সবচেয়ে গুরুত্বপূর্ণ।  
3. Load দিয়ে Agent scale যাচাই।  
4. Security scan CI-তে।  
5. Automation ছাড়া quality অনুমান মাত্র।  

→ পরবর্তী: [অধ্যায় ২১ — Deployment](./21-deployment.md)
