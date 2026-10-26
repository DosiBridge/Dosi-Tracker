# syntax=docker/dockerfile:1
#
# Dosi-Tracker API host, built from source. Build context: backend/
#   docker build -f backend/src/Dosi.Tracker.HttpApi.Host/Dockerfile -t dosi-tracker-api backend
#
# Runtime contract (see docs/SELF_HOSTING.md):
#   - listens on plain HTTP :8080 (TLS terminates at the reverse proxy in front of it); runs as the non-root
#     $APP_UID user; the application directory /app is read-only to that user and logs go to stdout.
#   - outside Development the host reads the OpenIddict signing/encryption certificate from /app/openiddict.pfx.
#     It is NEVER part of the image: mount it read-only at run time (compose: secrets), with its passphrase in
#     AuthServer__CertificatePassPhrase.
#   - every secret (connection string, passphrases, API keys) comes from the environment; the image carries only the
#     tracked, secret-free appsettings.json. appsettings.Development.json / appsettings.secrets.json never enter the
#     build context (backend/.dockerignore) and are removed from the publish output as a second line of defence.
#   - /health-status answers {"status":"Healthy"} (HTTP 200) once the host can serve requests and reach its database.
#
# Base images are pinned by tag and digest; Dependabot (docker) proposes the updates.

ARG NODE_IMAGE=node:24-bookworm-slim@sha256:55fb5f5a527ea2c5c4a95f963e48ef90aa38c7b346054fdc8ee9957a2b55d205
ARG SDK_IMAGE=mcr.microsoft.com/dotnet/sdk:10.0@sha256:afd0ccff9d306812a69ef0f47458376b6acb544ae866cfc4b8d28ff0eef35ea7
ARG RUNTIME_IMAGE=mcr.microsoft.com/dotnet/aspnet:10.0@sha256:d247d17dd429d96580eeb96cfce220c1089d860dbbf9f94d599ca64cf3604fe4

# Node.js (and yarn 1) for the ABP client libraries; only the binaries are copied into the build stage.
FROM ${NODE_IMAGE} AS node

FROM ${SDK_IMAGE} AS build
# The ABP CLI that installs wwwroot/libs: the version the solution is maintained with (backend/README.md).
ARG ABP_CLI_VERSION=3.0.7
ENV DOTNET_CLI_TELEMETRY_OPTOUT=1 \
    DOTNET_NOLOGO=1 \
    NUGET_XMLDOC_MODE=skip

COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
COPY --from=node /opt/ /opt/
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
 && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
 && ln -s /opt/yarn-v*/bin/yarn /usr/local/bin/yarn \
 && node --version && yarn --version \
 && dotnet tool install --tool-path /opt/abp-cli Volo.Abp.Studio.Cli --version "${ABP_CLI_VERSION}"

WORKDIR /src

# Restore first, from the project files only, so the package layer is reused until a dependency changes.
COPY common.props NuGet.Config ./
COPY src/Dosi.Tracker.Domain.Shared/Dosi.Tracker.Domain.Shared.csproj src/Dosi.Tracker.Domain.Shared/
COPY src/Dosi.Tracker.Domain/Dosi.Tracker.Domain.csproj src/Dosi.Tracker.Domain/
COPY src/Dosi.Tracker.Application.Contracts/Dosi.Tracker.Application.Contracts.csproj src/Dosi.Tracker.Application.Contracts/
COPY src/Dosi.Tracker.Application/Dosi.Tracker.Application.csproj src/Dosi.Tracker.Application/
COPY src/Dosi.Tracker.EntityFrameworkCore/Dosi.Tracker.EntityFrameworkCore.csproj src/Dosi.Tracker.EntityFrameworkCore/
COPY src/Dosi.Tracker.HttpApi/Dosi.Tracker.HttpApi.csproj src/Dosi.Tracker.HttpApi/
COPY src/Dosi.Tracker.HttpApi.Host/Dosi.Tracker.HttpApi.Host.csproj src/Dosi.Tracker.HttpApi.Host/
RUN dotnet restore src/Dosi.Tracker.HttpApi.Host/Dosi.Tracker.HttpApi.Host.csproj

# Client libraries (wwwroot/libs is git-ignored). Without them ABP answers every browser request with its
# "libs folder is missing" 500 page, so the build fails unless they were actually installed. yarn.lock is honoured
# strictly first; `abp install-libs` then copies the mapped files into wwwroot/libs.
COPY src/Dosi.Tracker.HttpApi.Host/package.json src/Dosi.Tracker.HttpApi.Host/yarn.lock src/Dosi.Tracker.HttpApi.Host/
RUN cd src/Dosi.Tracker.HttpApi.Host \
 && yarn install --frozen-lockfile --ignore-scripts --non-interactive --no-progress

COPY src/ src/
RUN /opt/abp-cli/abp install-libs --working-directory src/Dosi.Tracker.HttpApi.Host \
 && test -f src/Dosi.Tracker.HttpApi.Host/wwwroot/libs/jquery/jquery.js \
 && test -d src/Dosi.Tracker.HttpApi.Host/wwwroot/libs/abp

RUN dotnet publish src/Dosi.Tracker.HttpApi.Host/Dosi.Tracker.HttpApi.Host.csproj \
      -c Release --no-restore -o /out -p:UseAppHost=false \
 && rm -f /out/appsettings.Development.json /out/appsettings.secrets.json /out/web.config \
 && rm -rf /out/Logs \
 && if find /out -name '*.pfx' -o -name 'appsettings.secrets.json' -o -name 'appsettings.Development.json' | grep -q .; then \
      echo 'refusing to ship a certificate or developer settings in the image' >&2; exit 1; \
    fi \
 && test -f /out/wwwroot/libs/jquery/jquery.js

FROM ${RUNTIME_IMAGE} AS final
LABEL org.opencontainers.image.title="Dosi-Tracker API" \
      org.opencontainers.image.source="https://github.com/DosiBridge/Dosi-Tracker" \
      org.opencontainers.image.licenses="MIT"

ENV ASPNETCORE_ENVIRONMENT=Production \
    ASPNETCORE_HTTP_PORTS=8080 \
    DOTNET_CLI_TELEMETRY_OPTOUT=1

WORKDIR /app
# Owned by root and read-only to the app user: nothing (certificate, settings, code) can be changed at run time.
COPY --from=build /out .

# ASP.NET Core data-protection keys (antiforgery and account-page cookies). Compose mounts a volume here so they
# survive a container re-create; the directory exists in the image so the volume inherits the app user's ownership.
RUN mkdir -p /home/app/.aspnet/DataProtection-Keys \
 && chown -R "$APP_UID" /home/app/.aspnet

USER $APP_UID
EXPOSE 8080

# The image has no curl/wget: bash's /dev/tcp is enough for a loopback HTTP probe. 503 (Unhealthy) fails it.
HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 \
  CMD ["bash", "-c", "exec 3<>/dev/tcp/127.0.0.1/8080 && printf 'GET /health-status HTTP/1.1\\r\\nHost: localhost\\r\\nConnection: close\\r\\n\\r\\n' >&3 && head -n 1 <&3 | grep -q ' 200 '"]

ENTRYPOINT ["dotnet", "Dosi.Tracker.HttpApi.Host.dll"]
