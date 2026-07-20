# Runbook — Local & Future Production

> **Today:** local demo + ABP Host scaffold.  
> **Not yet:** production IIS/K8s cutover (see target [`bn/21-deployment.md`](./bn/21-deployment.md)).  
> As-built: [`AS_BUILT.md`](./AS_BUILT.md).

---

## 1. Prerequisites

| Component | Need |
|-----------|------|
| Frontend | Node 20+ |
| Backend | .NET 10 SDK, PostgreSQL 16+ |
| Windows agent | Rust (stable) |
| macOS agent | macOS 14+, Xcode 15+ |

---

## 2. Start PostgreSQL + migrate

1. Create DB matching `ConnectionStrings:Default` in:
   - `backend/src/Dosi.Tracker.HttpApi.Host/appsettings.json`
   - `backend/src/Dosi.Tracker.DbMigrator/appsettings.json`
2. Run migrator:

```powershell
cd "D:\Github Projects\DosiBridge\Open Source\Dosi-Tracker\backend"
dotnet run --project src/Dosi.Tracker.DbMigrator
```

Or ABP Studio / `backend/etc/scripts/migrate-database.ps1` if you use Studio.

**Expect:** `Abp*` + OpenIddict tables only — no `activities` table yet.

---

## 3. Start API Host

```powershell
cd "D:\Github Projects\DosiBridge\Open Source\Dosi-Tracker\backend"
dotnet run --project src/Dosi.Tracker.HttpApi.Host
```

| Check | URL / note |
|-------|------------|
| Self URL | `https://localhost:44342` (default) |
| Health | `https://localhost:44342/health-status` |
| Swagger | Host Swagger UI (when enabled in env) |
| Logs | `Logs/logs.txt` under Host output |

**Dev certs:** trust ASP.NET HTTPS certificate if browser/agent TLS fails.

---

## 4. Start frontend (demo, no API required)

```powershell
cd "D:\Github Projects\DosiBridge\Open Source\Dosi-Tracker\frontend"
npm install
npm run dev
```

Login with demo accounts listed in [`DOCUMENTATION.md`](../DOCUMENTATION.md) §19.  
Session is **localStorage** — Host can be down.

---

## 5. Desktop agents (offline / sync will fail until APIs exist)

### Windows

```powershell
cd "D:\Github Projects\DosiBridge\Open Source\Dosi-Tracker\clients\windows"
copy config.example.toml config.toml
# set api_base_url to https://localhost:44342 when Host is up
cargo run --release
```

Env overrides: `DOSI__API_BASE_URL`, `DOSI_USERNAME`, `DOSI_PASSWORD`.

### macOS

See `clients/macos/README.md` — align `apiBaseURL` with Host.

**Known failure mode:** `POST /api/app/account/login` (and related) are **not implemented**. Agent should queue locally; sync errors are expected.

---

## 6. Health & logs (current)

| Signal | Where |
|--------|--------|
| Process up | Host listening on `SelfUrl` |
| DB up | Migrator success; Host starts without connection errors |
| App health | `GET /health-status` |
| App logs | Serilog → console + `Logs/logs.txt` |
| Frontend | Browser console; Next terminal |

No Grafana / alert manager in repo yet.

---

## 7. Common failures

| Symptom | Likely cause | Action |
|---------|--------------|--------|
| Host won’t start | Postgres down / wrong connection string | Fix `ConnectionStrings:Default`, retry |
| Migration fail | Permissions / DB missing | Create DB, re-run DbMigrator |
| Agent login 404 | Product login route missing | Expected today — use offline queue only |
| Agent TLS / connection refused | Wrong port (`44300` vs `44342`) or untrusted cert | Match `SelfUrl`; trust cert |
| Frontend “wrong” data | Mock seed | Expected — not API-backed |
| CORS errors (once FE wired) | Origin not in Host CORS | Add Next origin to Host CORS config |

---

## 8. Rollback (when you have deployments)

Until CI/CD exists, local rollback =:

1. Stop Host process.  
2. Revert git to last known-good commit.  
3. Re-run DbMigrator only if migrations are backward-compatible; otherwise restore Postgres dump from before migrate.  
4. Restart Host + frontend.

**Production target** (zero-downtime, blue-green): course ch. 19/21 — **not as-built**.

---

## 9. Backup / restore (minimum for any shared env)

| Step | Action |
|------|--------|
| Backup | `pg_dump` of Tracker DB on a schedule |
| Secrets | Do not commit production connection strings |
| Restore drill | Restore dump to a scratch DB; point a Host instance at it; hit `/health-status` |
| Screenshots | N/A until object storage exists |

Retention / Host backup UI in course ch. 11 is **target**, not implemented.

---

## 10. Incident stub (future)

1. Confirm blast radius: one tenant vs platform.  
2. Check Host health + Serilog errors.  
3. Pause agent sync (feature flag / disable ingest) if data corruption risk.  
4. Preserve logs + DB snapshot before schema fixes.  
5. Postmortem → update this runbook.
