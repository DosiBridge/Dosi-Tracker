# Self-hosting Dosi-Tracker with Docker

This guide runs the whole server side of Dosi-Tracker on one machine with Docker Compose: PostgreSQL, the database
migrator, the API and the web dashboard, plus an optional TLS reverse proxy. The desktop agents (Windows, macOS,
Linux) then sign in to your API URL.

Everything here is driven by two tracked files: [`docker-compose.yml`](../docker-compose.yml) and
[`.env.example`](../.env.example). Your own copy, `.env`, is git-ignored and holds every secret.

## 1. What runs

| Service | Image (built from this repository) | Role | Listens on |
|---------|-------------------------------------|------|------------|
| `postgres` | `postgres:16-alpine` | Database (volume `tracker-pgdata`) | internal network only |
| `migrator` | `backend/src/Dosi.Tracker.DbMigrator/Dockerfile` | One-shot: applies migrations, seeds plans, OpenIddict clients and the first administrator | — (exits 0) |
| `api` | `backend/src/Dosi.Tracker.HttpApi.Host/Dockerfile` | REST API and OpenIddict token server | `127.0.0.1:6006` → container `8080` |
| `web` | `frontend/Dockerfile` | Next.js dashboard (standalone server) | `127.0.0.1:6005` → container `3000` |
| `proxy` (profile `proxy`) | `caddy:2-alpine` | TLS for both public URLs | `80`, `443` |
| `certgen` (profile `init`) | `alpine` | One-time creation of `./certs/openiddict.pfx` | — |

Properties of every image:

- Multi-stage builds from source: the images never copy anything a developer built or installed on the host
  (`backend/.dockerignore`, `frontend/.dockerignore`). The API image installs the ABP client libraries
  (`wwwroot/libs`) during the build and the build fails if they are missing.
- Final stages are runtime images (`mcr.microsoft.com/dotnet/aspnet:10.0`, `node:24-alpine`), pinned by tag and digest;
  Dependabot proposes base-image updates.
- They run as non-root users (`app`, uid/gid 1654, for .NET; `node`, uid 1000, for the dashboard). The application
  directory belongs to root and is read-only for that user.
- No secrets and no keys are baked in. `appsettings.Development.json`, `appsettings.secrets.json` and `*.pfx` never
  enter the build context, and the build fails if one appears in the publish output. Configuration comes from
  environment variables; the token-signing certificate is mounted at run time.
- `ASPNETCORE_ENVIRONMENT` / `DOTNET_ENVIRONMENT` are `Production`: no Swagger, no HealthChecks UI, no developer
  exception page. The API refuses to start with the repository's template secrets, and the migrator refuses to create
  the administrator with the template password.
- Logs go to stdout (`docker compose logs -f api`).

Networks: `data` is internal: only the migrator and the API can reach PostgreSQL. `edge` connects the API, the
dashboard and the proxy; its subnet (`TRACKER_EDGE_SUBNET`) is the only source, besides loopback, from which the API
accepts `X-Forwarded-For` / `X-Forwarded-Proto`.

## 2. Requirements

- Docker Engine with the Compose v2 plugin (`docker compose`). Verified with Docker Desktop (Compose v5.1).
- Two DNS names for this machine, one for the API and one for the dashboard (for example
  `tracker-api.example.com` and `tracker.example.com`).
- For publicly trusted certificates from the bundled proxy, ports 80 and 443 must be reachable from the internet.
- Network access to NuGet, npm and the container registries while the images are built.

## 3. First start

```bash
cp .env.example .env
# Edit .env: set TRACKER_API_URL, TRACKER_WEB_URL and every empty secret. Generate each secret with e.g.:
openssl rand -hex 32

docker compose --profile init run --rm certgen   # once: creates ./certs/openiddict.pfx
docker compose up -d --build --wait              # builds, migrates, starts api + web, waits until healthy
docker compose --profile proxy up -d             # optional: TLS on 80/443 for both URLs
```

What each step does:

1. If a required value in `.env` is missing, `docker compose` refuses to start and names it (`required variable
   TRACKER_DB_PASSWORD is missing a value`). There are no fallback defaults for secrets.
2. `certgen` creates an RSA 3072 certificate protected with `TRACKER_CERT_PASSPHRASE`. The API uses it to sign and
   encrypt tokens (section 5). It never overwrites an existing file.
3. `postgres` initialises its volume. `migrator` then migrates and seeds the database and exits with code 0. Only after
   that does `api` start; `web` follows. `--wait` returns once the API's `/health-status` and the dashboard's
   `/healthz` are healthy.
4. Sign in to the dashboard at `TRACKER_WEB_URL` as `admin` with `TRACKER_ADMIN_PASSWORD`, then change the password.
   Desktop agents use `TRACKER_API_URL` as their server address.

The administrator is created only on an empty database. Later migrator runs never create or change administrators,
so changing `TRACKER_ADMIN_*` afterwards has no effect.

## 4. HTTPS and the reverse proxy

The API issues tokens over HTTPS only. A plain-HTTP token request gets `400 This server only accepts HTTPS
requests`. The desktop agents also refuse plain `http://` servers except loopback. TLS therefore ends at a reverse
proxy in front of the containers.

**Bundled proxy (Caddy).** `docker compose --profile proxy up -d` serves `TRACKER_API_URL` → `api:8080` and
`TRACKER_WEB_URL` → `web:3000` ([`docs/self-hosting/Caddyfile`](./self-hosting/Caddyfile)). `TRACKER_PROXY_TLS`
chooses the certificates:

- your e-mail address: publicly trusted certificates from Let's Encrypt or ZeroSSL;
- `internal`: Caddy's local CA, for tests only (clients must trust it).

Certificates are kept in the `caddy-data` volume.

**Your own proxy** (nginx, Traefik, a cloud load balancer) must:

- forward to `127.0.0.1:6006` (API) and `127.0.0.1:6005` (dashboard), or join the `edge` network;
- send `X-Forwarded-Proto: https` and `X-Forwarded-For`;
- reach the API from loopback or from `TRACKER_EDGE_SUBNET`. A proxy on the same host qualifies: connections to the
  published ports arrive from the `edge` network's gateway address.

For a proxy on another machine, publish the ports on a private interface (`TRACKER_BIND_ADDRESS`) and add that
proxy's address to the API's `App__ForwardedHeaders__KnownProxies` (for example in a `docker-compose.override.yml`).

> Never publish the API or dashboard ports on a public interface. Whoever can reach them directly is treated as a
> trusted proxy and can claim any client address or scheme.

## 5. The token-signing certificate (`openiddict.pfx`)

Outside Development the API loads one certificate for signing and for encrypting the tokens it issues:
`/app/openiddict.pfx`, protected with `AuthServer__CertificatePassPhrase`. Compose mounts
`${TRACKER_OPENIDDICT_PFX:-./certs/openiddict.pfx}` read-only at that path. Without the file, the API container stops
with `FATAL: /app/openiddict.pfx is missing or unreadable`.

- **Permissions.** The file must be readable by uid or gid 1654. `certgen` sets `root:1654` and mode `0640`. If you
  bring your own file, run `chown root:1654 openiddict.pfx && chmod 0640 openiddict.pfx`.
- **Your own certificate.** Any RSA certificate whose key usage allows `digitalSignature` and `keyEncipherment`
  works. For example:

  ```bash
  openssl req -x509 -newkey rsa:3072 -sha256 -days 730 -nodes -subj "/CN=Dosi Tracker OpenIddict" \
    -addext "keyUsage=critical,digitalSignature,keyEncipherment" -keyout key.pem -out cert.pem
  openssl pkcs12 -export -inkey key.pem -in cert.pem -out openiddict.pfx   # prompts for the passphrase
  rm key.pem
  ```

  Do not use the HTTPS certificate of your domain.
- **Expiry.** Check the expiry date with
  `openssl pkcs12 -in certs/openiddict.pfx -nokeys | openssl x509 -noout -enddate`, and renew in time.
- **Rotation.** Rotation is disruptive. Delete the file, run `certgen` again and restart the API
  (`docker compose up -d --force-recreate api`). Every access and refresh token issued with the old key becomes
  invalid: users sign in to the dashboard again, and desktop agents show "sign-in expired" until the person signs in
  again.
- **Backups.** Back up the file and `.env` together. Without the certificate, every session is lost. Without
  `TRACKER_ENCRYPTION_PASSPHRASE`, values stored encrypted can no longer be read.

## 6. Configuration

[`.env.example`](../.env.example) lists and explains every variable the stack reads. Highlights:

| Variable | Purpose |
|----------|---------|
| `TRACKER_API_URL`, `TRACKER_WEB_URL` | Public URLs. The API's issuer, CORS and redirect origins, the dashboard's API address, and the OpenIddict client root URL are derived from them. |
| `TRACKER_DB_*` | PostgreSQL database, user and password. The password is applied only when the volume is first initialised. |
| `TRACKER_ADMIN_EMAIL`, `TRACKER_ADMIN_PASSWORD` | First platform administrator. |
| `TRACKER_CERT_PASSPHRASE`, `TRACKER_ENCRYPTION_PASSPHRASE` | Certificate passphrase and the key for encrypted settings. |
| `AI__*` | Optional AI assistant: off, with an empty key, by default. |
| `Billing__Enabled` | `false` for self-hosting. |
| `Settings__Abp.Mailing.*` | Optional SMTP for outgoing e-mail. |
| `S3__*` | Optional S3-compatible capture storage. Captures are stored in PostgreSQL by default. |

The optional settings use the API's own configuration names and are passed through only when `.env` defines them. A
name that is commented out stays unset, so the built-in default applies. The dashboard reads `DOSI_API_URL`,
`DOSI_DOWNLOAD_BASE_URL`, `DOSI_DEMO_MODE` and `DOSI_ALLOW_INSECURE_HTTP` when it serves each request. Compose
sets them from `.env`, so one dashboard image works for every environment.

## 7. Health checks and probes

| Endpoint | Healthy | Unhealthy |
|----------|---------|-----------|
| API `GET /health-status` | `200 {"status":"Healthy"}` (the host is up and its database is reachable) | `503` |
| Dashboard `GET /healthz` | `200 {"status":"ok",...}` | `503` with the configuration problems (for example a missing or plain-http `DOSI_API_URL`) |

Both images declare a Docker `HEALTHCHECK` on these endpoints, so `docker compose ps` shows the health and `up
--wait` waits for it.

On Kubernetes, use `/health-status` for the API's readiness probe and a TCP probe on 8080 for liveness, so a database
outage does not restart every API pod. Use `/healthz` for the dashboard's readiness and liveness probes. On Kubernetes,
mount the certificate as a secret file at `/app/openiddict.pfx` (`fsGroup: 1654`).

## 8. Upgrades

```bash
git pull
docker compose up -d --build --wait
```

Compose rebuilds the images and runs the migrator against the existing database. It then recreates the API and the
dashboard. The API is unavailable for a few seconds while its container is replaced. Agents keep their unsent data
queued and retry. Back up the database before upgrading (section 9).

To run published images instead of building them, set `TRACKER_API_IMAGE`, `TRACKER_MIGRATOR_IMAGE` and
`TRACKER_WEB_IMAGE` and use `docker compose pull && docker compose up -d --no-build --wait`.

## 9. Backups and restore

Back up these four things:

- **PostgreSQL**, which holds all product data and, by default, the captures.
- **`certs/openiddict.pfx` and `.env`.**
- **The `api-dataprotection` volume**, which holds the key ring for account-page cookies. Losing it only signs browser
  sessions on the account pages out.
- **The `caddy-data` volume**, if you use the bundled proxy.

Back up the database:

```bash
docker compose exec -T postgres sh -c 'pg_dump -U "$POSTGRES_USER" -d "$POSTGRES_DB" -Fc' > tracker-$(date +%F).dump
```

Restore into a fresh stack before starting the API:

```bash
docker compose up -d --wait postgres
docker compose exec -T postgres sh -c 'pg_restore -U "$POSTGRES_USER" -d "$POSTGRES_DB" --clean --if-exists' < tracker.dump
docker compose up -d --wait
```

Test a restore regularly.

## 10. Troubleshooting

| Symptom | Cause and fix |
|---------|---------------|
| `required variable … is missing a value` | `.env` lacks a required value. Compare it with `.env.example`. |
| `api` exits with `FATAL: /app/openiddict.pfx is missing or unreadable` | Run `docker compose --profile init run --rm certgen`, or fix the file's permissions (section 5). |
| `migrator` exits non-zero mentioning the administrator password | Set `TRACKER_ADMIN_PASSWORD` to a non-template password that meets the policy. |
| Token requests fail with `This server only accepts HTTPS requests` | The request reached the API without `X-Forwarded-Proto: https` from a trusted proxy. Check the proxy (section 4). |
| Dashboard `/healthz` is 503 | Its body names the problem. Usually `TRACKER_API_URL` is missing or uses plain `http://`. |
| `Pool overlaps with other one on this address space` | `TRACKER_EDGE_SUBNET` collides with an existing Docker network. Pick another private /24. |
| `bind: address already in use` | Change `TRACKER_API_PORT`, `TRACKER_WEB_PORT` or the proxy ports. |

On a fresh database, the migrator logs one `Failed executing DbCommand … __EFMigrationsHistory` error. EF Core logs
it while it checks whether the history table exists. It is harmless.

## 11. Removing the stack

```bash
docker compose --profile proxy down        # stop; data volumes are kept
docker compose --profile proxy down -v     # stop AND delete the database and every other volume
```
