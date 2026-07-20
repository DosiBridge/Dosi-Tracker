# অধ্যায় ১৫: Desktop Agent Development

> **Status:** Partial (capture + SQLite + HTTP client). Sync targets **missing** backend APIs.  
> **As-built:** [`../AS_BUILT.md`](../AS_BUILT.md) | **Run:** [`../RUNBOOK.md`](../RUNBOOK.md) section 5 | **Security:** [`../SECURITY.md`](../SECURITY.md)

> **Canonical:** Agent বাস্তবায়ন — tracking, SQLite, sync, update, installer।  
> Architecture ভূমিকা → [০৪](./04-system-architecture.md) ·  
> কেন Rust/Swift → [০৫](./05-technology-stack.md) ·  
> JWT/OWASP গভীরতা → [১৮](./18-security-architecture.md) ·  
> Ingest API contract → [১৭](./17-api-documentation.md)

---

## Agent কী

Background native process যা OS থেকে সিগন্যাল নেয়, local queue-তে রাখে, Backend-এ
sync করে। Browser extension পুরো desktop অ্যাপ কভার করে না — তাই native দরকার।

দুটি সমান Architecture, আলাদা UI স্ট্যাক:

| পথ | কখন |
|----|-----|
| Rust (Win) + Swift (Mac) | Monorepo প্রাথমিক — সর্বনিম্ন resource |
| Avalonia + C# worker | এক .NET টিম, tray UI চাই |

দুটোই একই API ও offline নীতি মেনে চলে।

---

## মডিউল মানচিত্র (দুটো পথেই)

```
Trackers/     keyboard, mouse, active_window, apps, screenshot, idle
Storage/      SQLite queue + encryption
Sync/         batch upload, retry, backoff, idempotency
Api/          device login, refresh, submit, version check
Config/       server URL, intervals, policy cache
Update/       signed download + restart
App/          DI composition, tray/service host
```

### Avalonia হলে অতিরিক্ত

- **MVVM:** View ↔ ViewModel ↔ Services  
- **DI:** `IKeyboardTracker`, `IActivityQueue`, `ISyncService`, `IApiClient`, …  
- Tray: Online / Offline / Syncing / Pause (policy allow করলে)

---

## Keyboard Tracking (বিস্তারিত)

| সংগ্রহ | সংগ্রহ নয় |
|--------|-----------|
| Hit count প্রতি interval | কোন কী, password, chat টেক্সট |

Windows: low-level hook / raw input → counter++।  
macOS: event tap (Accessibility permission)।  

Interval শেষে payload ফিল্ড: `keyboardHits`। Privacy নীতির কেন্দ্র — লঙ্ঘন করবেন না।

---

## Mouse Tracking

- Click count (left/right/middle আলাদা রাখতে পারেন metrics হিসেবে)  
- ঐচ্ছিক: scroll ticks, movement distance  

স্ক্রিন কোঅর্ডিনেটের raw stream লগ করবেন না (পুনর্গঠন ঝুঁকি)।

---

## Active Window & Running Programs

Polling বা OS event:

```
appName:     Visual Studio Code
windowTitle: Hospital Management System — dashboard.tsx
pid:         …
```

Running list: open processes (name only)।  
পরে Backend/UI: productivity category, project heuristic, monitor timeline।

Idle: `idleTimeoutSeconds` (global policy থেকে আসতে পারে — সেটিংস [১১](./11-platform-settings.md))
পার হলে tracking pause / idle segment।

---

## Screenshot Capture

1. Project/tenant `screenshot` permission চেক (API থেকে sync করা policy)  
2. Timer = `intervalMinutes`  
3. Capture → optional blur → JPEG/WebP compress → size cap  
4. Multi-monitor: policy (primary only vs all)  
5. Queue-এ file path + metadata; upload sync loop-এ  

Webcam একই ধাপ, default প্রায়ই off (privacy-first)।

---

## SQLite Local Database (schema)

### `pending_activities`
| Column | অর্থ |
|--------|------|
| id | local UUID = `clientActivityId` |
| payload_json | API body |
| screenshot_path | nullable local file |
| created_at | queue time |
| attempts | retry |
| last_error | text |
| status | pending \| sending \| done \| dead |

### `device_state`
encrypted tokens, server_url, device_id, last_sync_at, policy_json_cache।

### `sync_meta`
last_success_cursor, backoff_until।

**Encryption:** Windows DPAPI / macOS Keychain / SQLCipher — laptop theft mitigation।

---

## Offline Mode ও Sync Loop (এখানেই পূর্ণ বিবরণ)

```
Capture → INSERT pending → Sync loop → POST batch → ACK → mark done / delete file
```

অ্যালগরিদম:

1. Connectivity probe  
2. Select oldest `pending` LIMIT N  
3. Attach screenshots (presign flow থাকলে আগে upload — [১৭](./17-api-documentation.md))  
4. `POST /api/v1/activities/batch` + `Idempotency-Key` / `clientActivityId`  
5. 401 → refresh → একবার retry  
6. 408/5xx → exponential backoff (cap), `attempts++`  
7. attempts > max → `dead` + tray warning  
8. Success → status done, delete local screenshot blob  

Duplicate POST নিরাপদ হতে হবে — Backend unique (tenant_id, client_activity_id)।

---

## Auto Update

```
GET /api/v1/agent/version?os=windows&current=1.2.3
→ download URL + signature
→ verify signature
→ install silent
→ restart service
→ health ping; fail হলে rollback previous build
```

Unsigned update production-এ নিষিদ্ধ।

---

## Installer

**Windows:** MSIX (clean update) বা signed MSI/EXE (GPO)। Startup = Service বা
user logon task। Outbound HTTPS only।

**macOS:** notarized `.app`/pkg; Screen Recording + Accessibility prompts first run।

First-run wizard: server URL (optional) → login → device register → project select → start।

---

## Configuration উদাহরণ

```json
{
  "serverUrl": "https://api.dositracker.com",
  "syncIntervalSeconds": 30,
  "idleTimeoutSeconds": 300,
  "captureDefaults": {
    "screenshot": true,
    "webcam": false,
    "keyboard": true,
    "mouse": true,
    "activeWindow": true,
    "runningPrograms": true
  }
}
```

Tenant/project policy API থেকে এসে defaults override করে (interval, blur, webcam)।

---

## Agent-side security (সংক্ষেপ — গভীরতা অধ্যায় ১৮)

| বিষয় | Agent করে |
|------|-----------|
| Auth | device login → store refresh encrypted |
| Transport | HTTPS only |
| DB password | **কখনো রাখে না** |
| Queue | encrypt at rest |
| Updates | signature verify |

Threat model, OWASP, RBAC — পুনরাবৃত্তি নয়: [১৮](./18-security-architecture.md)।

---

## Testing Agent (এই স্তরের)

- Unit: counters, idle threshold, backoff  
- Integration: SQLite + mock HTTP API  
- Manual: sleep/wake, VPN flap, multi-monitor, disk full  
- Privacy review: logs-এ key content আছে কি না  

পুরো QA strategy → [২০](./20-testing.md)।

---

## সারসংক্ষেপ

Agent = Track → Queue → Sync → API। Privacy counts-only। Offline হারায় না।  
Signed install/update। Security গভীরতা অন্য অধ্যায়ে — এখানে শুধু agent প্রয়োগ।

→ [অধ্যায় ১৬ — Database](./16-database-design.md)
