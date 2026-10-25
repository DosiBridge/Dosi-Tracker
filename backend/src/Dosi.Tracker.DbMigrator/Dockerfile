# syntax=docker/dockerfile:1
#
# Dosi-Tracker database migrator, built from source. Build context: backend/
#   docker build -f backend/src/Dosi.Tracker.DbMigrator/Dockerfile -t dosi-tracker-migrator backend
#
# A one-shot job: applies every EF Core migration, then seeds the plans, the OpenIddict clients and, on an empty
# database, the first platform administrator. Run it before (re)starting the API on every upgrade. It needs:
#   ConnectionStrings__Default                 the PostgreSQL connection string
#   App__HostAdminEmail, App__HostAdminPassword the first administrator (outside Development the migrator refuses to
#                                              create it without a password, or with the template one)
#   OpenIddict__Applications__Tracker_App__RootUrl  the public dashboard URL (sign-in redirect / CORS of the web client)
# The image carries no settings beyond the tracked, secret-free appsettings.json (see backend/.dockerignore).
#
# The final stage is the ASP.NET Core runtime, not the plain .NET runtime: the migrator loads the ABP modules that
# reference Microsoft.AspNetCore.App and fails to start without it.

ARG SDK_IMAGE=mcr.microsoft.com/dotnet/sdk:10.0@sha256:afd0ccff9d306812a69ef0f47458376b6acb544ae866cfc4b8d28ff0eef35ea7
ARG RUNTIME_IMAGE=mcr.microsoft.com/dotnet/aspnet:10.0@sha256:d247d17dd429d96580eeb96cfce220c1089d860dbbf9f94d599ca64cf3604fe4

FROM ${SDK_IMAGE} AS build
ENV DOTNET_CLI_TELEMETRY_OPTOUT=1 \
    DOTNET_NOLOGO=1 \
    NUGET_XMLDOC_MODE=skip
WORKDIR /src

COPY common.props NuGet.Config ./
COPY src/Dosi.Tracker.Domain.Shared/Dosi.Tracker.Domain.Shared.csproj src/Dosi.Tracker.Domain.Shared/
COPY src/Dosi.Tracker.Domain/Dosi.Tracker.Domain.csproj src/Dosi.Tracker.Domain/
COPY src/Dosi.Tracker.Application.Contracts/Dosi.Tracker.Application.Contracts.csproj src/Dosi.Tracker.Application.Contracts/
COPY src/Dosi.Tracker.EntityFrameworkCore/Dosi.Tracker.EntityFrameworkCore.csproj src/Dosi.Tracker.EntityFrameworkCore/
COPY src/Dosi.Tracker.DbMigrator/Dosi.Tracker.DbMigrator.csproj src/Dosi.Tracker.DbMigrator/
RUN dotnet restore src/Dosi.Tracker.DbMigrator/Dosi.Tracker.DbMigrator.csproj

COPY src/ src/
RUN dotnet publish src/Dosi.Tracker.DbMigrator/Dosi.Tracker.DbMigrator.csproj \
      -c Release --no-restore -o /out -p:UseAppHost=false \
 && rm -f /out/appsettings.Development.json /out/appsettings.secrets.json \
 && rm -rf /out/Logs \
 && if find /out -name '*.pfx' -o -name 'appsettings.secrets.json' -o -name 'appsettings.Development.json' | grep -q .; then \
      echo 'refusing to ship a certificate or developer settings in the image' >&2; exit 1; \
    fi

FROM ${RUNTIME_IMAGE} AS final
LABEL org.opencontainers.image.title="Dosi-Tracker database migrator" \
      org.opencontainers.image.source="https://github.com/DosiBridge/Dosi-Tracker" \
      org.opencontainers.image.licenses="MIT"

# Explicit, so the administrator-password guard always applies (only Development falls back to the template password).
ENV DOTNET_ENVIRONMENT=Production \
    DOTNET_CLI_TELEMETRY_OPTOUT=1

WORKDIR /app
# Owned by root and read-only to the app user; logs go to stdout.
COPY --from=build /out .
USER $APP_UID

ENTRYPOINT ["dotnet", "Dosi.Tracker.DbMigrator.dll"]
