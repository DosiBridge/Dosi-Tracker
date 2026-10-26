# Dosi-Tracker web dashboard

The Next.js 16 app where owners, managers and members see their workspace's
time, activity and screenshots, manage projects and the team, and download the
desktop agents. It talks to the Dosi-Tracker API (`backend/`) from the browser
with OpenIddict tokens; it has no database of its own.

## Configuration

Everything that differs per deployment is read by the server **per request**
([`src/lib/runtime-config.ts`](src/lib/runtime-config.ts)) and rendered into
the page, so the same build runs in every environment. See
[`.env.example`](.env.example).

| Variable | Default | Meaning |
|---|---|---|
| `DOSI_API_URL` | dev: `https://localhost:6006`; production: **required** | The API the app and the agents sign in to. https only, except loopback. Trailing slashes are ignored. `NEXT_PUBLIC_API_URL` is still honoured as a legacy name. |
| `DOSI_DOWNLOAD_BASE_URL` | `https://github.com/DosiBridge/Dosi-Tracker/releases/latest/download` | Where the agent release assets live (see below). |
| `DOSI_DEMO_MODE` | off | `1` shows the built-in demo dataset to visitors without a session. For demos and the E2E suite only. |
| `DOSI_ALLOW_INSECURE_HTTP` | off | `true` accepts a plain-http, non-loopback API or download address. |
| `PORT`, `HOSTNAME` | `3000`, `0.0.0.0` | Listening address of the standalone server. |

The API must allow this site's origin in its CORS settings.

Without demo mode, a visitor with no session is signed out: the `dosi-token`
cookie only says a session *may* exist, and a cookie without a token in the
browser leads to `/login`, never to demo data. The demo dataset
(`src/lib/demo-tenants.ts`) is loaded with a dynamic `import()` only in demo
mode; `.dependency-cruiser.cjs` forbids static imports of it.

## Accounts, email and billing

Some pages depend on how the API is set up:

- **Emailed links.** Invitations, password resets and email confirmations
  are emailed by the API (when it has SMTP configured) as links into this
  site: `/invite/{token}`, `/reset-password?userId=&resetToken=` and
  `/confirm-email?userId=&token=`. Set the API's `App:ClientUrl` to this
  site's address. `/forgot-password`, `/reset-password` and `/confirm-email`
  are public pages; the forgot page answers the same whether or not an
  address has an account. Without SMTP, nothing is emailed: the team page
  says so and shows the invitation link to share by hand.
- **Invitations.** Nobody joins a workspace without accepting. Signed-in
  people answer the invitations sent to their *confirmed* email on
  `/workspaces`, or open the invitation link.
- **Billing.** The app asks the API whether it bills workspaces
  (`Billing:Enabled`, off by default). While it doesn't, the app shows no
  plans, trials, seat limits or invoices. While it does, owners can switch
  to a free plan their team fits, or cancel a paid one, on `/billing`;
  paid plans are assigned by the platform operator.
- **Deletion and retention.** Owners and managers delete a time block or a
  capture from the activity report (with a reason the API records); members
  delete their own captures for a while after upload. Owners set capture and
  activity retention, and can delete the workspace, in Settings → Workspace;
  anyone can delete their account in Settings → Security.

## Downloads

[`src/lib/agent-downloads.ts`](src/lib/agent-downloads.ts) is the single source
of truth for the agent downloads and the commands the guides show. Each release
publishes these version-less assets under the download base URL:

| Platform | Asset |
|---|---|
| Windows | `dosi-tracker-setup.exe` |
| macOS | `DosiTracker-macos.zip` |
| Linux | `dosi-tracker_amd64.deb`, `dosi-tracker.x86_64.rpm`, `dosi-tracker-linux-x86_64.tar.gz` |
| All | `SHA256SUMS` |

The signed-in **Download** page (`/download`) and the public guides
(`/install`, `/install/windows`, `/install/macos`, `/install/linux`) link them
and show this deployment's server address in every command.

## Security

- **Content-Security-Policy** per request from [`src/proxy.ts`](src/proxy.ts):
  a fresh nonce for scripts (`'strict-dynamic'`), `connect-src` limited to the
  site and the configured API origin, `frame-ancestors 'none'`. Every page is
  therefore rendered on demand.
- Static headers for every route in [`next.config.ts`](next.config.ts):
  `X-Content-Type-Options`, `Referrer-Policy`, `Permissions-Policy`,
  `X-Frame-Options`, `Cross-Origin-Opener-Policy`, HSTS (outside `next dev`);
  no `X-Powered-By`.
- `?next=` redirects go through one check
  ([`src/lib/safe-redirect.ts`](src/lib/safe-redirect.ts)) in the proxy and on
  the sign-in page.

## Running it

```bash
npm ci
cp .env.example .env.local            # then edit
npm run dev                           # http://localhost:6005
DOSI_DEMO_MODE=1 npm run dev          # explore without a backend (bash)
```

Production build and server:

```bash
npm run build
node .next/standalone/server.js       # copy .next/static and public next to it first
```

`next.config.ts` sets `output: "standalone"`: `.next/standalone` holds a
self-contained server; a container image copies `.next/standalone`,
`.next/static` (to `.next/standalone/.next/static`) and `public` (to
`.next/standalone/public`). Point the container health check at **`/healthz`**:
`200 {"status":"ok"}` when the app can serve sign-ins, `503` with the reasons
(for example no `DOSI_API_URL`) when its configuration can't work. It never
calls the API, so an API outage doesn't restart the web container.

## Errors

Each route segment has an `error.tsx` (and the root a `global-error.tsx`): a
crash shows a card with **Try again** and a way back while the surrounding
shell stays up. Errors are logged to the browser console as
`[dosi-tracker] render error` with the route pattern and Next.js's digest,
never personal data ([`src/lib/error-report.ts`](src/lib/error-report.ts)).

## Quality gates

```bash
npx tsc --noEmit
npm run lint
npm run test:coverage   # Vitest; coverage floors in vitest.config.ts
npm run test:arch       # dependency-cruiser + src/test/architecture.spec.ts
npm run build && npm run build:budget
npx playwright test --project=chromium --grep-invert "@visual"
```

The Playwright suite starts the production server with `DOSI_DEMO_MODE=1`
(see `playwright.config.ts`). On Windows, if the default E2E port is
reserved, set `E2E_PORT` (for example `E2E_PORT=5391`).
