# Desktop agent API contract

This is the authoritative contract between the Dosi Tracker server and its desktop agents (Windows, macOS,
Linux). Agents implement against this page; the server's tests pin it (`backend/test/**/Devices`,
`Routes/ContractRoutesTests`, the activity and presence HTTP tests). When the two disagree, the server code wins
and this page is the bug.

All times are UTC ISO 8601 (`2026-10-26T09:30:00Z`). All ids are UUIDs. JSON property names are camelCase and
case-insensitive on input; unknown properties are ignored.

## 1. Transport

- HTTPS only. An agent refuses a plain `http://` server unless it is loopback (`127.0.0.1`, `::1`,
  `localhost`) or the administrator opted in with `allow_insecure_http = true` in the agent's configuration.
- Never follow redirects on API calls (a redirect is an error), so a token is never replayed to another host.
- Request bodies are at most 32 MB (a block upload carries its captures base64-encoded).
- Agents send `Accept: application/json` and a `User-Agent` naming the agent and its version (for the server's
  logs; the server does not depend on it).

## 2. Sign-in

### 2.1 Device id

Each install generates a random UUIDv4 **device id** once, on first sign-in, and persists it with its settings
(not with the credentials, so signing out keeps it). The same id goes into the token requests (`device_id`),
every heartbeat (`deviceId`) and every block (`deviceId`). Never regenerate it silently: a new id is a new
device to the server (see 7.3).

### 2.2 Password grant

```
POST /connect/token
Content-Type: application/x-www-form-urlencoded

grant_type=password&client_id=Tracker_App&username=<user name or email>&password=<password>
&scope=Tracker%20offline_access&device_id=<device id>
```

- No `__tenant` parameter and no `X-Workspace-Id` header: a sign-in is not tied to a workspace.
- `device_id` binds the tokens to the install: the access token carries a `device_id` claim, the refresh token
  keeps it, and the sign-in's OpenIddict authorization records it so revoking the device can find it.
- Answer (200):

```json
{ "access_token": "<JWT>", "token_type": "Bearer", "expires_in": 3599,
  "refresh_token": "<opaque>", "scope": "Tracker offline_access" }
```

The access token lives about an hour and the refresh token about 14 days (server settings; use `expires_in`,
never hard-code). Keep the refresh token in the platform's secret store (Windows Credential Manager / DPAPI,
macOS Keychain, Linux Secret Service or a 0600 file). Never store the password.

### 2.3 Refresh grant

```
grant_type=refresh_token&client_id=Tracker_App&refresh_token=<refresh token>&device_id=<device id>
```

- Refresh tokens rotate: every answer carries a new `refresh_token`. Persist it before using the new access token.
  Redeeming an already used refresh token is refused and, outside a short leeway, revokes the whole sign-in.
- `device_id` is optional on a refresh but must be the install's own: a refresh naming another device than the
  one its token was issued to is refused (`invalid_grant`). A refresh token issued before device binding existed
  is bound by its first refresh that names a device.
- Refresh when a call answers 401, or shortly before `expires_in` runs out.

### 2.4 Token endpoint errors

Errors follow RFC 6749: status 400 with `{"error": "...", "error_description": "..."}`.

| `error` | When | Agent |
|---|---|---|
| `invalid_grant` on a password grant | wrong user name or password, account locked, inactive or deleted | show the message, stay signed out |
| `invalid_grant` on a refresh | refresh token expired, revoked (device revoked, sign-out elsewhere, account deleted), reused, or issued to another device | enter **"sign-in expired"**: stop capture, keep the queue, ask the user to sign in again. Never re-send a stored password. |
| `invalid_request` | malformed request, e.g. `device_id` is not a UUID (or is the empty UUID) | bug: report it |
| `invalid_client`, `unauthorized_client` | wrong `client_id` | bug |

429 with `Retry-After` (seconds): the per-client sign-in limits were hit (by default 60 password grants and
120 refreshes a minute per client IP, 10 password grants a minute per account). Wait at least `Retry-After`.

A forgotten password is recovered in the web app (its reset page emails a one-time link); an agent links the user
there instead of implementing recovery itself.

### 2.5 Sign-out

Revoke the refresh token (RFC 7009), then delete it locally:

```
POST /connect/revoke
Content-Type: application/x-www-form-urlencoded

client_id=Tracker_App&token=<refresh token>&token_type_hint=refresh_token
```

200 with an empty body when the token was revoked. A 400 (`invalid_token`, `unsupported_token_type`) means the
token was already unusable; delete it locally either way. `/connect/revocat` (ABP's path) is the same endpoint. The access token stays valid until it expires (at most about an hour), so discard it too. Sign-out keeps
the device id and the queue.

## 3. Workspaces and headers

After signing in, list the caller's workspaces **without** an `X-Workspace-Id` header:

```
GET /api/app/workspaces/mine
Authorization: Bearer <access token>
```

```json
[ { "id": "…", "name": "Acme", "role": "member", "planName": "Team", "isOwner": false,
    "isArchived": false, "archivedTime": null } ]
```

- Skip `isArchived` workspaces. With one usable workspace select it; with several let the user choose (match what
  the user typed against `id` or `name`; refuse an unknown or ambiguous one). Persist the chosen id, and key the
  local queue by (server, user, workspace id).
- Every workspace-scoped call sends `X-Workspace-Id: <workspace id>`: everything under `/api/app/` except
  `/api/app/workspaces/*`, and `/api/abp/application-configuration` when the agent wants that workspace's
  permissions. Do not send it to `/connect/*` or `/api/app/workspaces/*`.
- `GET /api/abp/application-configuration` (with the header) gives `currentUser.id`, used as `UserId` in reporting
  queries (7.4), and `auth.grantedPolicies`.

The server answers a bad header before anything else: 400 `Tracker:WorkspaceRequired` (missing on a
workspace-scoped call, or not a UUID), 403 `Tracker:NotWorkspaceMember`, 403 `Tracker:WorkspaceArchived`, then
403 `Tracker:DeviceRevoked` when the token's device is revoked in that workspace (section 6).

A workspace an owner **deleted** is gone at once: it no longer appears in `mine`, and every call in it answers 403
`Tracker:NotWorkspaceMember` (its data is purged later, queued blocks included, so there is nothing to upload to).

## 4. Endpoints

All calls below need `Authorization: Bearer <access token>` and `X-Workspace-Id`.

### 4.1 `GET /api/app/project/my-projects`

The projects the caller tracks time on (archived projects and projects where the caller is only a client are left
out), as a plain JSON array:

```json
[ { "id": "…", "title": "Website", "intervalMinutes": 10,
    "allowScreenshot": true, "allowWebcam": false, "allowKeyboard": true, "allowMouse": true,
    "allowActiveWindow": true, "allowRunningPrograms": false,
    "idleThresholdMinutes": 5, "blurScreenshots": false } ]
```

Capture something only when the project allows it **and** the user consented to that capability locally. Webcam
is off by default. `idleThresholdMinutes` 0 means idle detection off (default it to 5 when absent);
`blurScreenshots` true means blur the screen capture and its thumbnail before upload.

### 4.2 `POST /api/app/activity` (upload a block)

```json
{
  "projectId": "…",
  "clientActivityId": "…",
  "startedAt": "2026-10-26T09:00:00Z",
  "endedAt": "2026-10-26T09:10:00Z",
  "deviceId": "…",
  "productivity": 72,
  "mouseClicks": 140,
  "keyboardHits": null,
  "description": "Release notes",
  "activeWindows": [ { "appName": "code", "windowTitle": "main.rs", "seconds": 420,
                       "keyboardHits": null, "mouseClicks": 51 } ],
  "runningPrograms": [ { "appName": "firefox", "windowTitle": "" } ],
  "timeline": [ { "minute": 0, "active": true, "activeSeconds": 55, "appName": "code",
                  "keyboardHits": null, "mouseClicks": 12 } ],
  "screenshotPngBase64": "<JPEG or PNG, base64>",
  "screenshotThumbJpgBase64": "<JPEG ≤ 480 px wide, base64>",
  "screenshotCapturedAt": "2026-10-26T09:04:31Z",
  "screenshotBlurred": false,
  "webcamJpgBase64": null,
  "platform": "linux",
  "osVersion": "Ubuntu 24.04 (6.8.0-45)",
  "agentVersion": "1.0.0",
  "deviceName": "ana-thinkpad",
  "timeZone": "Asia/Dhaka",
  "utcOffsetMinutes": 360,
  "idleSeconds": 35,
  "endReason": "interval",
  "sentAt": "2026-10-26T09:10:02Z",
  "injectedInputEvents": 0
}
```

Required: `projectId`, `clientActivityId`, `startedAt`, `endedAt`, and a device (the token's `device_id` claim
or `deviceId`; current agents send both, the same id).

**Idempotency.** `clientActivityId` is a UUID the agent generates once per block and keeps across retries.
Re-uploading a block the caller already stored answers 200 with the stored block, whatever changed since
(membership, archive, revocation of a non-token device). A `clientActivityId` used by another member's block is
refused (409 `Tracker:ActivityClientIdConflict`). Always retry with the same body and id.

**Counts are nullable.** `mouseClicks`, `keyboardHits` and the per-window and per-minute counts are `null` (or
left out) when not measured: the capability is off, not consented, not allowed by the project, or unsupported in
the session (e.g. Wayland on Linux). Send `0` only for a measured block without input. Counts are numbers only:
never key codes, typed text, clipboard content or input timing. `productivity` (0..100) is `null` when not
measured.

**Limits** (a violation of a validation limit is a 400 with `validationErrors`):

| Field | Limit |
|---|---|
| block length | `endedAt` after `startedAt`, at most 120 minutes |
| `startedAt` | at most 45 days ago (the offline-queue window) |
| `endedAt` | at most 5 minutes after the server's clock |
| overlap | another block of the member may overlap by at most min(60 s, a quarter of the shorter block); none may start at the same instant |
| `activeWindows`, `runningPrograms` | at most 200 entries each |
| `timeline` | at most 120 entries; `minute` counts from `startedAt`; `activeSeconds` 0..90 |
| `description` | at most 2048 characters |
| `productivity` | 0..100; counts ≥ 0 |
| captures (decoded) | screen 8 MB (JPEG preferred, or PNG), thumbnail 512 KB (JPEG), webcam 2 MB (JPEG) |
| context strings | `platform` one of `windows`, `macos`, `linux`; `osVersion` 64, `agentVersion` 32, `deviceName` 128, `timeZone` 64 (IANA id) characters; `utcOffsetMinutes` −840..840; `endReason` one of `interval`, `pause`, `break`, `idle`, `lock`, `sleep`, `project`, `task`, `signout`, `quit`, `shutdown` |

The server enforces the project's capture policy as well: data the project does not allow is dropped, an invalid
or oversized capture is dropped (logged), and an unblurred screenshot of a project that requires blurring is
dropped. None of these refuses the block, and an unusable context value is stored as null. `runningPrograms`
titles are never stored; send them empty.

**Answer (200):** the stored block, `ActivityDto`:

```json
{ "id": "…", "userId": "…", "projectId": "…", "clientActivityId": "…",
  "startedAt": "…", "endedAt": "…", "productivity": 72, "mouseClicks": 140, "keyboardHits": null,
  "description": "…", "activeWindowsJson": "[…]", "runningProgramsJson": "[]", "timelineJson": "[…]",
  "platform": "linux", "osVersion": "…", "agentVersion": "1.0.0", "deviceId": "…", "deviceName": "…",
  "timeZone": "Asia/Dhaka", "utcOffsetMinutes": 360, "idleSeconds": 35, "endReason": "interval",
  "clockSkewSeconds": -2, "injectedInputEvents": 0,
  "captures": [ { "id": "…", "kind": "screen", "capturedAt": "…", "blurred": false } ],
  "creationTime": "…", "creatorId": "…" }
```

Delete the queued row only after a 200, or a 410 `Tracker:ActivityDeleted`: an owner or manager deleted that
block from the workspace (with its captures), and the server keeps its client id so that the block is never
recreated. Drop the row and its captures; resending can never succeed.

**No payload signature.** The server does not verify, and ignores, a `payloadSignature` field. A key the
device generated and keeps can only make the local queue tamper-evident; it proves nothing to the server, so no
agent may describe it as server-side verification. Server-side integrity of what an agent reports would need an
enrolled device key pair with signatures verified on ingest, which the server does not implement.

### 4.3 `POST /api/app/presence/heartbeat`

Every 60 s while the agent runs and is signed in, and at once on every status change (`stopped` on quit,
`signedout` on sign-out):

```json
{ "deviceId": "…", "status": "tracking", "projectId": "…", "statusSince": "…",
  "deviceName": "ana-thinkpad", "platform": "linux", "osVersion": "…", "agentVersion": "1.0.0",
  "currentApp": "code", "timeZone": "Asia/Dhaka", "utcOffsetMinutes": 360, "sentAt": "…" }
```

Required: `deviceId` (non-empty) and `status`, one of `tracking`, `paused`, `break`, `idle`, `locked`,
`choosing`, `signedout`, `stopped` (anything else is a 400). `currentApp` is kept only when the project allows the
active window; send it only then. Answer (200): `{ "serverTime": "…" }`, which an agent can use to warn about a
wrong device clock. A device counts as online for 150 s after its last heartbeat. Heartbeat failures never
affect the queue, but a terminal error (section 5) stops them.

### 4.4 `GET /api/app/reporting/summary`

Query: `From`, `To` (UTC, `[From, To)`, at most 366 days), `UserId` (the caller's `currentUser.id`, so an owner
or manager sees their own time rather than the team's), `TimeZone` (IANA id; always send it, older-agent
compatibility answers a numeric `averageProductivity` to requests without one), optional `ProjectId`.

The answer (`ReportSummaryDto`) has `totalActivities`, `totalTrackedMinutes`, `totalIdleMinutes`,
`averageProductivity` (null when nothing was measured, show "—" rather than 0%), `totalMouseClicks`,
`totalKeyboardHits`, `mouseMeasuredActivities`, `keyboardMeasuredActivities` (how many blocks measured each
count; a total with 0 measured blocks means "not measured"), and `perUser`, `perProject`, `perUserProject` rows.
`GET /api/app/reporting/daily-series` takes the same query for per-day totals.

### 4.5 Devices: `/api/app/device`

| Call | Who | Answer |
|---|---|---|
| `GET /api/app/device` | anyone (own devices); `Tracker.Devices.Manage` sees every member's | `{ "items": [WorkspaceDeviceDto] }` |
| `GET /api/app/device/{deviceId}/status` | the caller, about their own device | `{ "deviceId", "isRevoked", "revokedAt", "reason" }` |
| `POST /api/app/device/revoke` `{ "deviceId", "reason"? }` | the caller, for their own device | `DeviceRevocationDto` |
| `GET /api/app/device/my-revocations` | the caller | `{ "items": [DeviceRevocationDto] }` (newest 100) |
| `POST /api/app/device/revoke-member` `{ "userId", "deviceId", "reason"? }` | owners and managers (`Tracker.Devices.Manage`); only an owner may revoke an owner's device | `DeviceRevocationDto` |
| `POST /api/app/device/restore` `{ "userId", "deviceId" }` | owners and managers; only an owner may restore an owner's device | `{ "deviceId", "isRevoked": false }`, or 404 `Tracker:DeviceNotFound` when not revoked |

`DeviceRevocationDto`: `id`, `workspaceId`, `userId`, `deviceId`, `revokedAt`, `reason` (≤ 256 characters),
`revokedByUserId`. `WorkspaceDeviceDto`: `userId`, `userName`, `name`, `deviceId`, `deviceName`, `platform`,
`osVersion`, `agentVersion`, `lastSeenAt` (null when only the revocation is left), `isRevoked`, `revokedAt`,
`revokedByUserId`, `reason`; unrevoked devices first, most recently seen first. Only a device seen for that member
in the workspace (a heartbeat or a block) can be revoked (otherwise 400 `Tracker:DeviceNotObserved`). Revoking is
idempotent: a repeated call answers the existing record and revokes the device's sessions again.

An agent with a device-bound token needs none of these: a revoked install learns it from the 403 on its next call.

## 5. Errors

Server errors other than the token endpoint use ABP's format, with the header `_AbpErrorFormat: true`:

```json
{ "error": { "code": "Tracker:ActivityOverlaps", "message": "…", "details": null,
             "data": { "startedAt": "…", "endedAt": "…" }, "validationErrors": null } }
```

Decide by status **and** `error.code`; never by the message (it is localized). A 400 without a code but with
`validationErrors` is a validation error.

**Terminal** errors stop capture and heartbeats for that workspace, park the queued rows (keep them, do not retry
them in a loop), and tell the user. They need a new sign-in or a workspace switch:

| Status | `error.code` | Agent |
|---|---|---|
| 403 | `Tracker:DeviceRevoked` | this computer was removed from the workspace and its sessions were revoked: also delete the saved credentials for that server and show "This computer was removed; ask a workspace owner or manager to restore it." |
| 403 | `Tracker:NotWorkspaceMember` | removed from the workspace: list workspaces again |
| 403 | `Tracker:WorkspaceArchived` | workspace archived: list workspaces again |
| 400 | `Tracker:WorkspaceRequired` | no or malformed `X-Workspace-Id`: a local configuration error, choose a workspace again |
| 400 (token endpoint) | `invalid_grant` on refresh | "sign-in expired" (2.4) |

**Per block** (park that row, keep uploading the others):

| Status | `error.code` | Meaning |
|---|---|---|
| 400 | `Tracker:ActivityClientIdRequired` | empty `clientActivityId` (bug) |
| 400 | `Tracker:DeviceIdRequired` | neither the token nor the block names a device (bug: sign in with `device_id`, send `deviceId`) |
| 400 | validation error | a limit of 4.2 is broken (bug) |
| 409 | `Tracker:ActivityClientIdConflict` | the client id belongs to another member's block |
| 409 | `Tracker:ActivityOverlaps` | overlaps time already tracked (often the same member on another computer) |
| 409 | `Tracker:ProjectArchived` | the project was archived before the block started; refresh the projects |
| 403 | `Tracker:NotProjectMember` | no longer a member of the project, or it was deleted; refresh the projects |
| 422 | `Tracker:ActivityTimeRangeInvalid` | `endedAt` is not after `startedAt` |
| 422 | `Tracker:ActivityTooLong` | longer than 120 minutes (`data.maxMinutes`) |
| 422 | `Tracker:ActivityTooOld` | started more than 45 days ago (`data.maxAgeDays`) |

**Drop** (delete the queued row, it can never be stored):

| Status | `error.code` | Meaning |
|---|---|---|
| 410 | `Tracker:ActivityDeleted` | an owner or manager deleted this block; log it, do not tell the user as an error |

**Retry later** (keep the row, back off exponentially, cap the delay at a few minutes):

| Status | `error.code` | Meaning |
|---|---|---|
| 425 | `Tracker:ActivityInFuture` | ends more than 5 minutes after the server's clock: tell the user to fix the clock; retry, and park after a day |
| 402 | `Tracker:SubscriptionInactive` | the workspace's subscription is not active (canceled, lapsed trial or overdue): tell the user, keep the rows queued and retry about hourly. Never answered by a deployment with billing off (`Billing:Enabled`, the self-hosted default) |
| 401 | — | access token expired or rejected: refresh (2.3) once, then retry |
| 429, 5xx, timeouts, no network | — | transient (a 500 is also what a concurrent upload of the same member gets) |

`Tracker:InvalidCaptureData` and `Tracker:CaptureTooLarge` exist but are not returned by the upload (a bad capture
is dropped and the block stored). Any other code: treat as per block for an upload, and log it.

## 6. Device revocation

- **What it does.** A revocation is kept per (workspace, member, device id). In that workspace every call with a
  token bound to the device, and every upload or heartbeat naming it (by token or body), answers 403
  `Tracker:DeviceRevoked`. The device's sign-in sessions are revoked everywhere, including in the member's other
  workspaces: its refresh tokens answer `invalid_grant`, and its access tokens stop working in the revoking
  workspace at once and elsewhere when they expire (about an hour). Web sign-ins, which name no device, are not
  affected.
- **Who.** A member revokes their own devices; an owner or manager (`Tracker.Devices.Manage`) revokes and
  restores any member's device in the workspace, except that only an owner may touch an owner's device. Only a
  device seen in the workspace can be revoked there, so another workspace's owner cannot sign the device out.
- **Restore.** Restoring lifts the refusal; the sessions revoked with the device stay revoked, so the install signs
  in again with the password.
- **A new device id does not undo a revocation silently.** The revoked install has no usable token left and cannot
  refresh. Continuing needs an interactive sign-in with the member's password; an install that then presents a new
  device id is a new device, listed as such to the workspace's owners and managers, and an install that presents
  the revoked id is still refused in that workspace. Uploads that name no device at all are refused
  (`Tracker:DeviceIdRequired`), and a fresh `deviceId` in a body cannot hide a revoked token's device: both are
  checked. Device revocation cuts off an install (a lost or stolen computer); to stop a person, remove them from
  the workspace.
- **Limits.** Agents that sign in without `device_id` (versions from before this contract) get unbound sessions,
  which revoking a device cannot find; their uploads are still refused by the `deviceId` they send, and their
  sessions become revocable once the agent refreshes or signs in with `device_id`. A server running several instances must share its
  distributed cache (for example Redis) so that every instance sees a revocation at once; with one instance (the
  default deployment) it is immediate.

## 7. Deleted data

- **Blocks and captures.** Owners and managers can delete a member's block with its captures, or a capture alone,
  and members can delete their own captures for a short while after upload (the workspace's window, 60 minutes by
  default). Nothing changes for an agent except the 410 on a replay of a deleted block (4.2): an agent never needs
  to re-send or re-capture deleted data.
- **Workspaces.** See section 3: a deleted workspace answers 403 `Tracker:NotWorkspaceMember`, a terminal error.
- **Accounts.** A person who deletes their account loses every session at once (refresh: `invalid_grant`, sign-in
  expired); a later password sign-in answers `invalid_grant`. The agent keeps its queue, which can no longer be
  uploaded; offering to clear it is up to the agent.
