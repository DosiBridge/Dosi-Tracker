#!/usr/bin/env bash
# Deploy smoke test: boots the shipped container images the way an operator would, on a fresh PostgreSQL database,
# with ASPNETCORE_ENVIRONMENT=Production, and proves the result can be signed in to.
#
#   1. The DbMigrator applies every migration and seeds the first administrator (non-template password).
#   2. The API host starts in Production with a throwaway openiddict.pfx and non-template secrets.
#   3. /health-status answers {"status":"Healthy"}; Swagger and the HealthChecks UI are not served.
#   4. OpenIddict refuses plain-HTTP token requests; behind a (simulated) TLS-terminating proxy the agent sign-in
#      contract works: password grant with device_id, wrong password -> invalid_grant, refresh grant with device_id,
#      and GET /api/app/workspaces/mine with / without the access token.
#   5. The Production secrets guard refuses to start the API with the tracked template secrets.
#   6. Optionally (SMOKE_WEB_IMAGE), the dashboard image serves /login.
#
# Every secret is generated here, lives only for this run and is masked in GitHub Actions logs. Nothing is written
# outside a temporary directory, and every container, the network and the database are removed on exit.
#
# Inputs (environment):
#   SMOKE_API_IMAGE, SMOKE_MIGRATOR_IMAGE   images to test (required)
#   SMOKE_WEB_IMAGE                         dashboard image (optional; skipped when empty)
#   SMOKE_NETWORK, SMOKE_DB_HOST            an existing Docker network with PostgreSQL on it (CI: the job's
#   SMOKE_DB_USER, SMOKE_DB_PASSWORD        `services: postgres`). When SMOKE_NETWORK is empty a throwaway
#   SMOKE_DB_NAME                           network and postgres container are created instead (local runs).
#   SMOKE_API_PORT                          port the API image listens on (default 8080)
#   SMOKE_WEB_PORT                          port the dashboard image listens on (default 3000)
#   SMOKE_WEB_API_URL_VAR                   name of the dashboard's runtime API-base-URL variable (optional)
#
# Local run (Git Bash or Linux), after building the images:
#   SMOKE_API_IMAGE=dosi-tracker-api:smoke SMOKE_MIGRATOR_IMAGE=dosi-tracker-migrator:smoke \
#     bash .github/scripts/deploy-smoke.sh
set -euo pipefail

# Git Bash on Windows would otherwise rewrite container paths such as /app/ into C:/Program Files/Git/app/.
export MSYS_NO_PATHCONV=1

: "${SMOKE_API_IMAGE:?set SMOKE_API_IMAGE}"
: "${SMOKE_MIGRATOR_IMAGE:?set SMOKE_MIGRATOR_IMAGE}"
SMOKE_WEB_IMAGE="${SMOKE_WEB_IMAGE:-}"
SMOKE_NETWORK="${SMOKE_NETWORK:-}"
SMOKE_API_PORT="${SMOKE_API_PORT:-8080}"
SMOKE_WEB_PORT="${SMOKE_WEB_PORT:-3000}"
SMOKE_WEB_API_URL_VAR="${SMOKE_WEB_API_URL_VAR:-}"

readonly CURL_IMAGE="curlimages/curl:8.22.0"
readonly POSTGRES_IMAGE="postgres:16-alpine"
# Where the host looks for openiddict.pfx outside Development (its working directory in the image).
readonly PFX_DIR="/app"
# Public URLs the API is told it lives behind. They are never resolved: requests go to the container directly.
readonly PUBLIC_API_URL="https://tracker.smoke.invalid"
readonly PUBLIC_WEB_URL="https://dashboard.smoke.invalid"

run_id="dosi-smoke-$$-${RANDOM}"
api_container="${run_id}-api"
web_container="${run_id}-web"
guard_container="${run_id}-guard"
db_container="${run_id}-db"
created_network=""
work_dir="$(mktemp -d)"
# Native Windows tools (Git Bash's openssl) need a Windows path, and MSYS_NO_PATHCONV below stops the automatic rewrite.
if command -v cygpath >/dev/null 2>&1; then
  work_dir="$(cygpath -m "${work_dir}")"
fi

log() { printf '\n==> %s\n' "$*"; }
fail() { printf '\nDEPLOY SMOKE FAILED: %s\n' "$*" >&2; exit 1; }

mask() {
  if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
    echo "::add-mask::$1"
  fi
}

secret() { openssl rand -hex "$1"; }

# A random RFC 4122 version-4 UUID (the per-install device_id an agent persists).
uuid_v4() {
  local h variant
  h="$(openssl rand -hex 16)"
  variant="$(printf '%x' $(( (16#${h:16:1} & 0x3) | 0x8 )))"
  printf '%s-%s-4%s-%s%s-%s\n' "${h:0:8}" "${h:8:4}" "${h:13:3}" "${variant}" "${h:17:3}" "${h:20:12}"
}

cleanup() {
  local status=$?
  if [[ ${status} -ne 0 ]]; then
    for c in "${api_container}" "${web_container}"; do
      if docker container inspect "${c}" >/dev/null 2>&1; then
        echo "---- docker logs ${c} (last 200 lines) ----" >&2
        docker logs --tail 200 "${c}" >&2 2>&1 || true
      fi
    done
  fi
  docker rm -f "${api_container}" "${web_container}" "${guard_container}" "${db_container}" >/dev/null 2>&1 || true
  if [[ -n "${created_network}" ]]; then
    docker network rm "${created_network}" >/dev/null 2>&1 || true
  fi
  rm -rf "${work_dir}"
  exit "${status}"
}
trap cleanup EXIT

# --- Database --------------------------------------------------------------------------------------------------------

if [[ -n "${SMOKE_NETWORK}" ]]; then
  network="${SMOKE_NETWORK}"
  db_host="${SMOKE_DB_HOST:?set SMOKE_DB_HOST with SMOKE_NETWORK}"
  db_user="${SMOKE_DB_USER:?set SMOKE_DB_USER with SMOKE_NETWORK}"
  db_password="${SMOKE_DB_PASSWORD:?set SMOKE_DB_PASSWORD with SMOKE_NETWORK}"
  db_name="${SMOKE_DB_NAME:?set SMOKE_DB_NAME with SMOKE_NETWORK}"
else
  log "Starting a throwaway PostgreSQL (${POSTGRES_IMAGE})"
  network="${run_id}"
  docker network create "${network}" >/dev/null
  created_network="${network}"
  db_host="${db_container}"
  db_user="tracker"
  db_password="$(secret 16)"
  db_name="Tracker"
  POSTGRES_USER="${db_user}" POSTGRES_PASSWORD="${db_password}" POSTGRES_DB="${db_name}" \
    docker run -d --name "${db_container}" --network "${network}" \
      -e POSTGRES_USER -e POSTGRES_PASSWORD -e POSTGRES_DB "${POSTGRES_IMAGE}" >/dev/null
  # Over TCP: the image's init-time server listens on the unix socket only and is restarted afterwards.
  for _ in $(seq 1 60); do
    if docker exec "${db_container}" pg_isready -h 127.0.0.1 -U "${db_user}" -d "${db_name}" >/dev/null 2>&1; then
      break
    fi
    sleep 1
  done
  docker exec "${db_container}" pg_isready -h 127.0.0.1 -U "${db_user}" -d "${db_name}" >/dev/null || fail "PostgreSQL did not become ready"
fi
mask "${db_password}"
connection_string="Host=${db_host};Port=5432;Database=${db_name};User ID=${db_user};Password=${db_password};"

# --- Throwaway secrets and signing certificate -----------------------------------------------------------------------

log "Generating throwaway secrets and openiddict.pfx"
cert_passphrase="$(secret 24)"
encryption_passphrase="$(secret 24)"
# ASP.NET Identity's default policy: upper, lower, digit and a non-alphanumeric character.
admin_password="Smoke-$(secret 12)-Aa1"
for s in "${cert_passphrase}" "${encryption_passphrase}" "${admin_password}"; do mask "${s}"; done

# One RSA key for signing and encryption, as ABP's AddProductionEncryptionAndSigningCertificate expects; the key
# usages are explicit so OpenIddict accepts it for both roles.
openssl req -x509 -newkey rsa:2048 -sha256 -days 2 -nodes \
  -subj "/CN=Dosi Tracker deploy smoke" \
  -addext "keyUsage=critical,digitalSignature,keyEncipherment" \
  -keyout "${work_dir}/key.pem" -out "${work_dir}/cert.pem" >"${work_dir}/openssl.log" 2>&1 \
  || { cat "${work_dir}/openssl.log" >&2; fail "openssl could not create the certificate"; }
SMOKE_CERT_PASSPHRASE="${cert_passphrase}" openssl pkcs12 -export \
  -inkey "${work_dir}/key.pem" -in "${work_dir}/cert.pem" \
  -out "${work_dir}/openiddict.pfx" -passout env:SMOKE_CERT_PASSPHRASE >"${work_dir}/openssl.log" 2>&1 \
  || { cat "${work_dir}/openssl.log" >&2; fail "openssl could not create openiddict.pfx"; }
rm -f "${work_dir}/key.pem"

# Copies openiddict.pfx into a created (not yet started) container. A tar stream avoids host-path bind mounts, which
# differ between Linux runners and Docker Desktop.
install_pfx() {
  tar -C "${work_dir}" --owner=0 --group=0 --mode=0444 -cf - openiddict.pfx | docker cp - "$1:${PFX_DIR}/"
}

# --- 1. Migrator -----------------------------------------------------------------------------------------------------

log "Running the DbMigrator (Production, fresh database)"
ConnectionStrings__Default="${connection_string}" \
App__HostAdminEmail="smoke-admin@example.invalid" \
App__HostAdminPassword="${admin_password}" \
OpenIddict__Applications__Tracker_App__RootUrl="${PUBLIC_WEB_URL}" \
OpenIddict__Applications__Tracker_Swagger__RootUrl="${PUBLIC_API_URL}" \
  docker run --rm --network "${network}" \
    -e DOTNET_ENVIRONMENT=Production \
    -e ConnectionStrings__Default -e App__HostAdminEmail -e App__HostAdminPassword \
    -e OpenIddict__Applications__Tracker_App__RootUrl -e OpenIddict__Applications__Tracker_Swagger__RootUrl \
    "${SMOKE_MIGRATOR_IMAGE}" || fail "the DbMigrator exited non-zero"

# --- 2. API in Production --------------------------------------------------------------------------------------------

log "Starting the API host (Production)"
ConnectionStrings__Default="${connection_string}" \
AuthServer__CertificatePassPhrase="${cert_passphrase}" \
StringEncryption__DefaultPassPhrase="${encryption_passphrase}" \
  docker create --name "${api_container}" --network "${network}" \
    -e ASPNETCORE_ENVIRONMENT=Production \
    -e ConnectionStrings__Default -e AuthServer__CertificatePassPhrase -e StringEncryption__DefaultPassPhrase \
    -e App__SelfUrl="${PUBLIC_API_URL}" \
    -e AuthServer__Authority="${PUBLIC_API_URL}" \
    -e App__CorsOrigins="${PUBLIC_WEB_URL}" \
    "${SMOKE_API_IMAGE}" >/dev/null
install_pfx "${api_container}"
docker start "${api_container}" >/dev/null

# Requests run from a curl container that shares the API container's network namespace, so they arrive from
# loopback: the only proxy the host trusts without configuration. "proxy" adds X-Forwarded-Proto: https, which is
# what a TLS-terminating reverse proxy in front of the API sends. Prints the body, a newline, then the status code.
http() {
  local mode="$1" method="$2" path="$3"
  shift 3
  local -a args=(-sS -X "${method}" -H "Accept: application/json" -w '\n%{http_code}')
  if [[ "${mode}" == "proxy" ]]; then
    args+=(-H "X-Forwarded-Proto: https")
  fi
  docker run --rm -i --network "container:${api_container}" "${CURL_IMAGE}" \
    "${args[@]}" "$@" "http://127.0.0.1:${SMOKE_API_PORT}${path}"
}

status_of() { printf '%s' "${1##*$'\n'}"; }
body_of() { printf '%s' "${1%$'\n'*}"; }
# The value of a top-level string property in a JSON document (compact or indented); empty when absent.
json_string() {
  printf '%s' "$1" | tr -d '\r\n' | sed -n "s/.*\"$2\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p"
}

expect_status() {
  local response="$1" expected="$2" what="$3" actual
  actual="$(status_of "${response}")"
  if [[ "${actual}" != "${expected}" ]]; then
    printf '%s\n' "$(body_of "${response}")" | head -c 2000 >&2
    fail "${what}: expected HTTP ${expected}, got ${actual}"
  fi
  echo "ok - ${what} (HTTP ${actual})"
}

log "Waiting for /health-status"
healthy=""
for _ in $(seq 1 90); do
  if [[ "$(docker inspect -f '{{.State.Running}}' "${api_container}")" != "true" ]]; then
    fail "the API container exited during start-up"
  fi
  response="$(http proxy GET /health-status </dev/null 2>/dev/null || true)"
  if [[ "$(status_of "${response}")" == "200" ]]; then
    healthy="${response}"
    break
  fi
  sleep 2
done
[[ -n "${healthy}" ]] || fail "/health-status never answered 200 (last: $(status_of "${response:-}"))"
[[ "$(json_string "$(body_of "${healthy}")" status)" == "Healthy" ]] || fail "/health-status is not Healthy: $(body_of "${healthy}")"
echo "ok - /health-status is Healthy"

# --- 3. Diagnostics stay closed in Production -------------------------------------------------------------------------

# An unmapped path is answered by the error-page pipeline (a redirect or a 404), so "not exposed" means: neither the
# Swagger UI nor the OpenAPI document is served.
for path in /swagger/index.html /swagger/v1/swagger.json /health-ui; do
  code="$(status_of "$(http proxy GET "${path}" </dev/null)")"
  [[ "${code}" != "200" ]] || fail "${path} is served in Production (HTTP 200); diagnostics must be opt-in"
  echo "ok - ${path} is not served in Production (HTTP ${code})"
done

# --- 4. Agent sign-in contract ---------------------------------------------------------------------------------------

device_id="$(uuid_v4)"
password_grant() {
  printf 'grant_type=password&client_id=Tracker_App&scope=Tracker%%20offline_access&username=admin&password=%s&device_id=%s' \
    "$1" "${device_id}"
}

log "Agent sign-in contract"
response="$(password_grant "${admin_password}" | http direct POST /connect/token \
  -H "Content-Type: application/x-www-form-urlencoded" --data-binary @-)"
expect_status "${response}" 400 "plain-HTTP token request is refused (transport security enforced)"

response="$(password_grant "Wrong-$(secret 8)-Aa1" | http proxy POST /connect/token \
  -H "Content-Type: application/x-www-form-urlencoded" --data-binary @-)"
expect_status "${response}" 400 "wrong password is refused"
[[ "$(json_string "$(body_of "${response}")" error)" == "invalid_grant" ]] || fail "wrong password: expected error=invalid_grant"

response="$(password_grant "${admin_password}" | http proxy POST /connect/token \
  -H "Content-Type: application/x-www-form-urlencoded" --data-binary @-)"
expect_status "${response}" 200 "password grant with device_id"
access_token="$(json_string "$(body_of "${response}")" access_token)"
refresh_token="$(json_string "$(body_of "${response}")" refresh_token)"
[[ -n "${access_token}" ]] || fail "password grant returned no access_token"
[[ -n "${refresh_token}" ]] || fail "password grant returned no refresh_token (offline_access)"
mask "${access_token}"
mask "${refresh_token}"

response="$(printf 'grant_type=refresh_token&client_id=Tracker_App&refresh_token=%s&device_id=%s' \
  "${refresh_token}" "${device_id}" | http proxy POST /connect/token \
  -H "Content-Type: application/x-www-form-urlencoded" --data-binary @-)"
expect_status "${response}" 200 "refresh grant with device_id"
[[ -n "$(json_string "$(body_of "${response}")" access_token)" ]] || fail "refresh grant returned no access_token"

# An agent whose access token expired or was revoked must get a 401 (its cue to refresh), never data or a redirect.
response="$(printf 'Authorization: Bearer %s\n' "not-a-valid-token" | http proxy GET /api/app/workspaces/mine -H @-)"
expect_status "${response}" 401 "workspaces/mine with an invalid access token"

code="$(status_of "$(http proxy GET /api/app/workspaces/mine </dev/null)")"
[[ "${code}" != 2* ]] || fail "workspaces/mine without a token answered HTTP ${code}"
echo "ok - workspaces/mine without a token is refused (HTTP ${code})"

# The bearer header goes through stdin (curl -H @-) so the token never appears in a process list.
response="$(printf 'Authorization: Bearer %s\n' "${access_token}" | http proxy GET /api/app/workspaces/mine -H @-)"
expect_status "${response}" 200 "workspaces/mine with the access token"
[[ "$(body_of "${response}")" == \[* ]] || fail "workspaces/mine did not return a JSON array"

# --- 5. Production secrets guard -------------------------------------------------------------------------------------

log "Production secrets guard refuses the tracked template secrets"
# Everything is supplied except StringEncryption:DefaultPassPhrase, which therefore still comes from the tracked
# appsettings.json. The certificate and its passphrase are valid, so a certificate error cannot be why the host stops.
ConnectionStrings__Default="${connection_string}" \
AuthServer__CertificatePassPhrase="${cert_passphrase}" \
  docker create --name "${guard_container}" --network "${network}" \
    -e ASPNETCORE_ENVIRONMENT=Production -e ConnectionStrings__Default -e AuthServer__CertificatePassPhrase \
    -e App__SelfUrl="${PUBLIC_API_URL}" -e AuthServer__Authority="${PUBLIC_API_URL}" \
    "${SMOKE_API_IMAGE}" >/dev/null
install_pfx "${guard_container}"
guard_status=0
timeout 120 docker start -a "${guard_container}" >"${work_dir}/guard.log" 2>&1 || guard_status=$?
if [[ ${guard_status} -eq 0 || ${guard_status} -eq 124 ]]; then
  tail -50 "${work_dir}/guard.log" >&2
  fail "the API started (or hung) with template secrets in Production (exit ${guard_status})"
fi
grep -q "Refusing to start in Production with repository/template secrets" "${work_dir}/guard.log" \
  || { tail -50 "${work_dir}/guard.log" >&2; fail "the API stopped, but not because of the secrets guard"; }
echo "ok - the API refuses to start with template secrets (exit ${guard_status})"

# --- 6. Dashboard image ----------------------------------------------------------------------------------------------

if [[ -n "${SMOKE_WEB_IMAGE}" ]]; then
  log "Starting the dashboard image"
  web_env=()
  if [[ -n "${SMOKE_WEB_API_URL_VAR}" ]]; then
    web_env=(-e "${SMOKE_WEB_API_URL_VAR}=${PUBLIC_API_URL}")
  fi
  docker run -d --name "${web_container}" --network "${network}" "${web_env[@]}" "${SMOKE_WEB_IMAGE}" >/dev/null
  web_ok=""
  for _ in $(seq 1 60); do
    if [[ "$(docker inspect -f '{{.State.Running}}' "${web_container}")" != "true" ]]; then
      fail "the dashboard container exited during start-up"
    fi
    code="$(docker run --rm --network "container:${web_container}" "${CURL_IMAGE}" \
      -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${SMOKE_WEB_PORT}/login" 2>/dev/null || true)"
    if [[ "${code}" == "200" ]]; then
      web_ok="yes"
      break
    fi
    sleep 2
  done
  [[ -n "${web_ok}" ]] || fail "the dashboard never served /login (last HTTP ${code:-none})"
  echo "ok - the dashboard serves /login"
fi

log "Deploy smoke passed"
