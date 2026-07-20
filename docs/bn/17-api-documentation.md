# অধ্যায় ১৭: API Documentation

> **Status:** Target design (product REST).  
> **As-built:** Agents expect `/api/app/account/login`, `/api/app/project/my-projects`, `/api/app/activity` — **not implemented**. ABP Identity/OpenIddict only — [`../AS_BUILT.md`](../AS_BUILT.md).

> **Canonical:** REST contracts, versioning, rate limits, Swagger।  
> Schema → [১৬](./16-database-design.md) · Security → [১৮](./18-security-architecture.md) · Agent sync → [১৫](./15-desktop-agent.md)

---

## Design নীতি

1. `/api/v1/...` versioning  
2. JSON  
3. `Authorization: Bearer <access>`  
4. Tenant **server-side** (client body-র tenantId বিশ্বাস নয়)  
5. একরূপ error envelope  
6. Activity ingest idempotent  
7. Pagination: `skip`/`maxResultCount` বা cursor  

---

## Authentication

| Method | Path | নোট |
|--------|------|-----|
| POST | `/auth/login` | email/password → access + refresh |
| POST | `/auth/refresh` | rotate refresh |
| POST | `/auth/logout` | revoke |
| POST | `/auth/forgot-password` | email |
| POST | `/auth/reset-password` | token |
| POST | `/auth/device-login` | Agent device bind |

Response (login):

```json
{
  "accessToken": "…",
  "expiresIn": 3600,
  "refreshToken": "…",
  "user": { "id": "…", "name": "…", "role": "owner", "tenantId": "…" }
}
```

---

## Users / Employees

| Method | Path | Permission |
|--------|------|------------|
| GET | `/users` | Team.View |
| GET | `/users/{id}` | Team.View বা Self |
| PUT | `/users/{id}` | Team.Manage / Self |
| POST | `/users/invite` | Team.Invite |

```json
{ "emails": ["jane@acme.com"], "role": "worker", "projectId": "optional" }
```

Admin যেন Owner invite করতে না পারে — policy [১৮](./18-security-architecture.md) + product rules।

---

## Activities

### Ingest (Agent)

`POST /activities` · `POST /activities/batch`

```json
{
  "clientActivityId": "device-local-uuid",
  "projectId": "…",
  "startedAt": "2026-07-16T09:00:00Z",
  "endedAt": "2026-07-16T09:10:00Z",
  "mouseClicks": 170,
  "keyboardHits": 350,
  "productivity": 82,
  "description": "Implementing filters",
  "activeWindows": [
    { "appName": "Visual Studio Code", "windowTitle": "…", "seconds": 480 }
  ],
  "runningPrograms": [{ "appName": "Docker Desktop" }],
  "hasWebcam": false
}
```

Server sets `userId`/`tenantId` from token। Duplicate `clientActivityId` → same resource (200)।

### Query (Dashboard)

`GET /activities?from&to&userId&projectId&app&minProductivity&skip&maxResultCount`

| Role | Scope |
|------|-------|
| owner/admin | tenant-wide (filters optional) |
| worker | force userId=self |
| client | force projectId ∈ membership |

---

## Screenshots

Preferred scale path:

1. `POST /screenshots/presign` → URL + key  
2. Agent PUT to storage  
3. `POST /screenshots/complete` { activityId, key, size, blurred }

`GET /screenshots` → metadata + **short-lived presigned GET** (কখনো public ACL নয়)।

---

## Projects

```
GET/POST /projects
GET/PUT  /projects/{id}
POST     /projects/{id}/members
DELETE   /projects/{id}/members/{userId}
POST     /projects/{id}/archive
```

Create body: title, intervalMinutes, permissions flags, memberIds।

---

## Reports & Dashboard

```
GET /dashboard                          # role-aware summary
GET /reports/time-activity?from&to&…
GET /reports/productivity
GET /reports/apps-urls
GET /reports/projects
GET /reports/attendance
GET /reports/payroll
GET /reports/weekly
GET /reports/{slug}/export.csv
```

Reports: Owner/Admin (+ permission)। Export rate-limited।

---

## Billing (Owner)

```
GET  /billing/plan
GET  /billing/usage
GET  /billing/invoices
POST /billing/change-plan
POST /billing/payment-method
```

Host platform billing আলাদা prefix নিচে।

---

## Host / Admin

```
GET/POST /host/tenants
PUT      /host/tenants/{id}
POST     /host/tenants/{id}/suspend
POST     /host/tenants/{id}/impersonate
GET/PUT  /host/plans/{id}
GET      /host/users
GET      /host/overview
GET/PUT  /host/settings/{section}
```

শুধু `host`। Impersonation audit বাধ্যতামূলক ([১৮](./18-security-architecture.md))।  
Settings sections = [১১](./11-platform-settings.md) মডিউলগুলো।

---

## Error envelope

```json
{
  "error": {
    "code": "Dosi:ActivityValidationFailed",
    "message": "EndedAt must be after StartedAt.",
    "details": { "field": "endedAt" },
    "correlationId": "…"
  }
}
```

| Status | অর্থ |
|--------|------|
| 400 | validation |
| 401 | auth |
| 403 | permission/tenant |
| 404 | missing |
| 409 | conflict (optional) |
| 429 | rate limit |
| 500 | unexpected |

---

## Swagger / OpenAPI

- Staging: `/swagger`  
- Production: বন্ধ বা VPN-only  
- Generate frontend/agent clients from OpenAPI  

প্রতিটি operation: summary, auth, examples।

---

## Rate limiting (উদাহরণ)

| Client | Limit |
|--------|-------|
| Device ingest | 120/min |
| Interactive user | 60/min |
| Export | 10/min |
| Login | strict (anti brute-force) |

Headers: `X-RateLimit-Remaining`, `Retry-After`। Global defaults [১১](./11-platform-settings.md)।

---

## সারসংক্ষেপ

Versioned REST, tenant-safe, idempotent ingest, presigned files, এক error shape, Swagger contract।  
এটি API-এর একমাত্র বিস্তারিত অধ্যায়।

→ [অধ্যায় ১৮ — Security](./18-security-architecture.md)
