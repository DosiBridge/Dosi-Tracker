# syntax=docker/dockerfile:1
#
# Dosi-Tracker web dashboard (Next.js, output: "standalone"). Build context: frontend/
#   docker build -t dosi-tracker-web frontend
#
# One image for every environment: nothing deployment-specific is baked in at build time. The server reads its
# configuration from the environment when it serves a request (src/lib/runtime-config.ts):
#   DOSI_API_URL              the Dosi-Tracker API base URL (required; https unless loopback)
#   DOSI_DOWNLOAD_BASE_URL    where the desktop agent downloads live (default: GitHub Releases, latest)
#   DOSI_DEMO_MODE            "true" serves the demo dataset to visitors without a session (demos only)
#   DOSI_ALLOW_INSECURE_HTTP  "true" accepts a plain-http, non-loopback DOSI_API_URL (explicit opt-in)
# Do not pass NEXT_PUBLIC_* build arguments: those would be frozen into the bundle.
#
# Runs as the unprivileged "node" user on :3000 (PORT / HOSTNAME); /healthz answers 200 when the configuration can
# serve sign-ins and 503 (with the reasons) when it cannot. Base images are pinned by tag and digest; Dependabot
# (docker) proposes the updates.

ARG NODE_IMAGE=node:24-alpine3.24@sha256:06d7adf3e0ef61a13f1d28682726366f97a15aa1888c934d3ecec86c33918b53

FROM ${NODE_IMAGE} AS deps
WORKDIR /app
ENV NEXT_TELEMETRY_DISABLED=1
COPY package.json package-lock.json ./
# Exactly the locked dependency tree; install scripts stay off (nothing the build needs relies on them).
RUN npm ci --ignore-scripts --no-audit --no-fund

FROM ${NODE_IMAGE} AS build
WORKDIR /app
ENV NEXT_TELEMETRY_DISABLED=1 \
    NODE_ENV=production
COPY --from=deps /app/node_modules ./node_modules
COPY . .
RUN npm run build \
 && test -f .next/standalone/server.js

FROM ${NODE_IMAGE} AS runtime
LABEL org.opencontainers.image.title="Dosi-Tracker web dashboard" \
      org.opencontainers.image.source="https://github.com/DosiBridge/Dosi-Tracker" \
      org.opencontainers.image.licenses="MIT"

ENV NODE_ENV=production \
    NEXT_TELEMETRY_DISABLED=1 \
    PORT=3000 \
    HOSTNAME=0.0.0.0

WORKDIR /app
# The application files are owned by root and read-only to the "node" user; only Next's cache directory is writable.
COPY --from=build /app/public ./public
COPY --from=build /app/.next/standalone ./
COPY --from=build /app/.next/static ./.next/static
RUN mkdir -p .next/cache && chown node:node .next/cache

USER node
EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD ["node", "-e", "fetch('http://127.0.0.1:'+(process.env.PORT||3000)+'/healthz').then(r=>process.exit(r.ok?0:1),()=>process.exit(1))"]

CMD ["node", "server.js"]
