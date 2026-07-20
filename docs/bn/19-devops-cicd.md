# অধ্যায় ১৯: DevOps, Docker & CI/CD

> **Status:** Target design.  
> **As-built:** Dockerfiles exist; **no** docker-compose, **no** `.github/workflows` — [`../AS_BUILT.md`](../AS_BUILT.md) | [`../RUNBOOK.md`](../RUNBOOK.md).

> **Canonical:** containerize, CI pipelines, secrets, zero-downtime, rollback imaging।  
> IIS/DNS/SSL go-live ধাপ → [২১](./21-deployment.md) (এখানে পুনরাবৃত্তি নেই)।  
> Backup schedule config → [১১](./11-platform-settings.md) ·  
> Backup verify/alerts → [২২](./22-monitoring.md)

---

## DevOps কেন

Commit → একই automated path → staging → prod। হাতে RDP copy = drift + অজানা version।

---

## Docker images

| Image | Role |
|-------|------|
| `dosi-api` | .NET Host |
| `dosi-web` | Next.js |
| `dosi-migrator` | one-shot DbMigrator |
| `postgres` / `redis` | official |

### Dockerfile নীতি

- Multi-stage (SDK ≠ runtime)  
- Non-root user  
- `HEALTHCHECK` → `/health/live`  
- No secrets in layers  

---

## Docker Compose (local/staging এক বক্স)

সার্ভিস: `db`, `redis`, `migrator` (completed successfully), `api`, `web`।  
API `depends_on` migrator। Env থেকে `POSTGRES_PASSWORD`, `JWT_SIGNING_KEY`।

```bash
docker compose up --build
docker compose logs -f api
```

পুরো YAML উদাহরণ রেপোতে `docker-compose.yml` রাখার সুপারিশ — এখানে নীতিই যথেষ্ট যাতে
deploy অধ্যায়ের infra checklist-এর সাথে ডুপ্লিকেট না হয়।

---

## GitHub Actions

### PR
`restore → build → test → lint → (optional) docker build → vuln/secret scan`

### Main
`build → test → push images:sha → deploy staging → smoke → approval → prod → smoke`

Image tag = git SHA। শুধু `latest` নয়।

### TeamCity
একই stages; টুল আলাদা।

---

## Secrets

Git-এ নয়: DB password, JWT key, SMTP, cloud keys, signing cert।  
Stores: GitHub Environments, Azure Key Vault, AWS Secrets Manager।  
Leak → rotate immediately।

Env vars নামকরণ উদাহরণ (মান deploy অধ্যায়ে bind):

`ConnectionStrings__Default`, `Redis__Configuration`, `Auth__Jwt__SigningKey`,
`Storage__*`, `SMTP__*`।

---

## Zero-downtime

1. Rolling বা blue-green  
2. DB **expand** migration আগে → code → পরে **contract**  
3. Health fail → traffic পুরনোতে  

Migrator আলাদা job — multi-pod startup race নয়।

---

## Rollback

1. Previous image digest redeploy  
2. Irreversible migration হলে snapshot restore (planned only)  
3. Frontend previous build  
4. Postmortem  

Staging-এ rollback drill রাখুন।

---

## Pre-deploy backup hook

Prod pipeline deploy-এর আগে DB snapshot trigger (cloud API)।  
Retention/policy বিস্তারিত [১১](./11-platform-settings.md) — এখানে শুধু “deploy আগে snapshot”।

---

## সাধারণ ভুল

| ভুল | সমাধান |
|-----|--------|
| Secret in git | vault |
| Migrate on every pod start | migrator job |
| No smoke | health + login + one ingest |
| Friday 6pm only deploys | weekday + on-call |

---

## সারসংক্ষেপ

Docker + CI + secrets + safe migrate + rollback images।  
সার্ভারে IIS bind / DNS / cert — পরের অধ্যায়।

→ [অধ্যায় ২০ — Testing](./20-testing.md) · [২১ — Deployment](./21-deployment.md)
