# Dosi.Tracker

## About this solution

This is a layered startup solution based on [Domain Driven Design (DDD)](https://abp.io/docs/latest/framework/architecture/domain-driven-design) practises. All the fundamental ABP modules are already installed. Check the [Application Startup Template](https://abp.io/docs/latest/solution-templates/layered-web-application) documentation for more info.

### Pre-requirements

* [.NET10.0+ SDK](https://dotnet.microsoft.com/download/dotnet)
* [Node v18 or 20](https://nodejs.org/en)

### Configurations

The solution comes with a default configuration that works out of the box for development. The API host reads every
setting from configuration, so in a deployment set them as environment variables (`Section__Key`) or in a secret
store — never in the tracked `appsettings.json`. Outside `Development` the host **refuses to start** while a secret is
a well-known value (the template passphrases, a database password published in this repository or a stock default),
is read from the tracked `appsettings.json`, or while `openiddict.pfx` is a localhost development certificate or has
expired.

| Setting | Purpose | Default |
|---|---|---|
| `App__ClientUrl` | The web app's address; emailed links (invitations, password reset, email confirmation, billing) open it | the first `App__CorsOrigins` entry |
| `App__CorsOrigins` | Browser origins the API admits, comma-separated (`https://app.example.com`, `https://*.example.com`); `*` is refused | `http://localhost:6005` |
| `App__ForwardedHeaders__KnownNetworks` / `__KnownProxies` | Reverse proxies trusted for `X-Forwarded-For`/`-Proto` (CIDR / IPs). Behind a proxy that is not on the same host, set it, or sign-ins fail as plain HTTP; the host warns at start-up when HTTPS is required and nothing is trusted | loopback only |
| `Billing__Enabled` | Bill workspaces (trials, seat limits, invoices, owner downgrade/cancel). Off, every workspace is active and has no limits | `false` |
| `Settings__Abp.Mailing.Smtp.Host`, `…Port`, `…UserName`, `…EnableSsl` | SMTP server for the emails. Until a host is set, nothing is sent and the host logs that an email was not sent | none |
| `Email__SmtpPassword` | SMTP password in plain text (environment or secret store only); alternatively the setting `Abp.Mailing.Smtp.Password`, encrypted with `StringEncryption__DefaultPassPhrase` | none |
| `Settings__Abp.Mailing.DefaultFromAddress`, `…DefaultFromDisplayName` | Sender of the emails | ABP's defaults |
| `DataErasure__IsEnabled`, `__PeriodMinutes`, `__WorkspaceGraceDays`, `__BatchSize` | The erasure sweep: purges deleted workspaces (captures and files included) after the grace period and scrubs deleted accounts | on, 60, 7, 200 |
| `DataProtection__KeysDirectory`, `__ApplicationName` | Data Protection keys (sign-in cookies, password-reset and confirmation tokens) are kept in the database table `AppDataProtectionKeys`; a directory instead when set. The application name must be the same on every instance | database, `Dosi.Tracker` |

Emails are queued as ABP background jobs (table `AbpBackgroundJobs`) and retried with a growing delay when the SMTP
server fails. For local testing, point SMTP at a mail catcher such as Mailpit (`Settings__Abp.Mailing.Smtp.Host=localhost`,
`…Port=1025`, `…EnableSsl=false`).

**Local secrets.** The tracked `appsettings.Development.json` holds template values only. Put your local database
password (and an AI key, if you try AI assist) in the git-ignored
`src/Dosi.Tracker.HttpApi.Host/appsettings.secrets.json`, for example
`{ "ConnectionStrings": { "Default": "Host=localhost;Port=5432;Database=Tracker;User ID=postgres;Password=<yours>;" } }`.
It is copied to `bin` but never into a publish folder, so it cannot reach a container image (it would override the
container's environment there).

#### AI assist (off by default)

AI assist will send aggregated tracker data to an OpenAI-protocol provider, only in workspaces whose owner turns it on.
So far the server has the provider configuration, the workspace settings and the usage views; no feature calls the
provider yet. It is off unless `AI__Enabled=true`.

- **Owners opt in** (`PUT /api/app/ai-assist/settings`) after acknowledging the current disclosure version
  (`"1.{AI:Disclosure:Version}"`); turning on window titles or task notes needs the same acknowledgement. Every change
  is kept in the consent history (`AppAiConsentEvents`), and every active member gets an in-app notification when AI
  assist starts or may send more of their data. Only the workspace's own setting counts: a global or configured value
  can never turn it on. Raising `AI__Disclosure__Version` asks every owner to consent again.
- **Everyone reads** `GET /api/app/ai-assist/status` (what is available to them and why not) and
  `GET /api/app/ai-assist/usage`; owners and managers read the settings.
- **The platform console** (`/api/app/platform-ai/overview`, `PUT /api/app/platform-ai/status`) shows the provider state
  and the month's usage across workspaces, and switches AI assist off everywhere at once without a restart.
- **The usage ledger** (`AppAiUsageRecords`) holds one row per served request (feature, tokens, cost, outcome; never a
  prompt, an answer or a name). Rows older than `AI__Usage__RetentionDays` (400) are deleted daily. A deleted
  workspace's ledger and consent history are purged with the rest of its data.

At start-up the host logs one line with the provider state (`Disabled`,
`Configured`, or what is missing). The key is never logged, and it must never be written into a tracked file:
the start-up guard refuses an `AI:OpenAI:ApiKey` read from `appsettings.json`, and a test fails on any settings file
under `backend/` that carries one.

| Setting | Purpose | Default |
|---|---|---|
| `AI__Enabled` | The hard switch (read at start-up) | `false` |
| `AI__Provider` | `OpenAI`, `AzureOpenAI` (v1 API; `Model` is the deployment name) or `OpenAICompatible` | `OpenAI` |
| `AI__OpenAI__ApiKey` | The provider key: environment, `appsettings.secrets.json` or a secret store only | none |
| `AI__OpenAI__Model` | The default model, e.g. `gpt-5.4-mini` (works with strict structured output and `AI__ReasoningEffort=low`) or `gpt-4.1-mini` (leave `ReasoningEffort` unset) | none: AI stays off |
| `AI__OpenAI__BaseUrl`, `__AllowedHosts` | Endpoint, required for Azure and compatible servers; absolute https (plain http only for localhost in Development) and its host must be in the comma-separated allowlist | OpenAI's own, `api.openai.com` |
| `AI__OpenAI__OrganizationId`, `__ProjectId` | Sent as OpenAI's organization and project headers | none |
| `AI__RequestTimeoutSeconds`, `AI__MaxRetries` | Per-attempt timeout; retries of 408, 429, every 5xx (OpenAI's edge answers 520 now and then) and network errors, with exponential back-off. A Retry-After longer than 10 s is reported instead of waited for | `30`, `2` |
| `AI__ReasoningEffort`, `AI__Temperature` | Sent only when set (reasoning models reject a temperature) | none |
| `AI__Features__<Feature>__Enabled`, `__Model`, `__MaxOutputTokens` | Per feature: `PeriodSummary`, `MemberDaySummary`, `ReportExplanation`, `DataQualityReview`, `NoteSuggestions`, `TaskDrafts`, `Ask` | on, the default model, per feature |
| `AI__Pricing__<n>__Model`, `__InputPer1M`, `__CachedInputPer1M`, `__OutputPer1M` | USD per million tokens, for cost estimates and the USD budget | none |
| `AI__Quotas__Default__*`, `AI__Quotas__Plans__<Plan>__*` | `MonthlyRequests`, `MonthlyTokens`, `UserDailyRequests`, `UserPerMinute`, by plan name (built in: Free, Starter, Business; anything else uses Default: 200 / 800,000 / 20 / 4) | see text |
| `AI__Disclosure__ProviderDisplayName`, `__ProcessingRegion`, `__RetentionNote`, `__Version` | What owners are shown before they opt in; raise `Version` to ask for consent again | `OpenAI`, none, none, `0` |
| `AI__Usage__RetentionDays` | How long usage-ledger rows are kept | `400` |

List the models your key can use locally (`GET https://api.openai.com/v1/models`), never in CI. Prefer a dedicated
OpenAI project per environment with a project-scoped key, a model allowlist and a monthly budget.


### Before running the application

* Run `abp install-libs` command on your solution folder to install client-side package dependencies. This step is automatically done when you create a new solution, if you didn't especially disabled it. However, you should run it yourself if you have first cloned this solution from your source control, or added a new client-side package dependency to your solution.
* Run `Dosi.Tracker.DbMigrator` to create the initial database. This step is also automatically done when you create a new solution, if you didn't especially disabled it. This should be done in the first run. It is also needed if a new database migration is added to the solution later.

#### The first platform (host) administrator

The first DbMigrator run on an empty database creates the platform administrator `admin` (email `App:HostAdminEmail`, default `admin@abp.io`) with the password in `App:HostAdminPassword` — set it, e.g. as the environment variable `App__HostAdminPassword`. Outside the `Development` environment the migrator **refuses** to create that account without the setting (or with the template password `1q2w3E*`), because anyone could sign in to a fresh deployment with it. In `Development` (`dotnet run` uses the project's launch profile, which sets `DOTNET_ENVIRONMENT=Development`) it falls back to `1q2w3E*`, and the administrator must change it at the first sign-in. Later runs never create or change administrators.

#### Upgrading to the agent data v2 migrations

`AgentDataV2` and `AgentDataV2Followup` also bring existing data in line with the new rules (PostgreSQL only):

* Subscriptions are enforced from now on (on a deployment with `Billing__Enabled=true`): a workspace whose trial ended cannot record time or grow its team until the host activates it (`POST /api/app/platform/activate-subscription`). Workspaces whose trial had already ended before the upgrade (status `past_due`, nothing invoiced) get a new 14-day trial from the moment of the upgrade, so their owners are reminded three days before it ends and told when it ended, and the host can activate the paying ones first.
* The Identity `manager` role is taken from everyone who no longer holds an Admin/Manager membership on an existing project.
* Duplicate memberships are merged (the oldest stays), and every membership's rate matches its latest rate-history entry.
* Activity of blocks that never measured input (projects that allow neither keyboard nor mouse, agents that sent no timeline) becomes "not measured" instead of 0%.

#### Generating a Signing Certificate

In the production environment, you need to use a production signing certificate. ABP Framework sets up signing and encryption certificates in your application and expects an `openiddict.pfx` file in your application.

To generate a self-signed signing certificate (named for your API host, not `localhost`, and protected with a
passphrase of your own, which you then supply as `AuthServer__CertificatePassPhrase`):

```bash
openssl req -x509 -newkey rsa:3072 -sha256 -days 730 -nodes -subj "/CN=api.example.com" -keyout openiddict.key -out openiddict.crt
openssl pkcs12 -export -inkey openiddict.key -in openiddict.crt -out openiddict.pfx
```

The host refuses a `dotnet dev-certs` certificate (subject `localhost`), an expired certificate and the template
passphrase outside `Development`. Renew the certificate before it expires: every token signed with it becomes
invalid when it is replaced.

It is recommended to use **two** RSA certificates, distinct from the certificate(s) used for HTTPS: one for encryption, one for signing.

For more information, please refer to: [OpenIddict Certificate Configuration](https://documentation.openiddict.com/configuration/encryption-and-signing-credentials.html#registering-a-certificate-recommended-for-production-ready-scenarios)

> Also, see the [Configuring OpenIddict](https://abp.io/docs/latest/Deployment/Configuring-OpenIddict#production-environment) documentation for more information.

### Solution structure

This is a layered monolith application that consists of the following applications:

* `Dosi.Tracker.DbMigrator`: A console application which applies the migrations and also seeds the initial data. It is useful on development as well as on production environment.
* `Dosi.Tracker.HttpApi.Host`: ASP.NET Core API application that is used to expose the APIs to the clients.

#### Test Projects

The `test` folder contains the following test projects:

* `Dosi.Tracker.Application.Tests`: Application layer tests.
* `Dosi.Tracker.Domain.Tests`: Domain layer tests.
* `Dosi.Tracker.EntityFrameworkCore.Tests`: Entity Framework Core integration tests.



## Deploying the application

Deploying an ABP application follows the same process as deploying any .NET or ASP.NET Core application. However, there are important considerations to keep in mind. For detailed guidance, refer to ABP's [deployment documentation](https://abp.io/docs/latest/Deployment/Index).

### Additional resources

You can see the following resources to learn more about your solution and the ABP Framework:

* [Web Application Development Tutorial](https://abp.io/docs/latest/tutorials/book-store/part-1)
* [Application Startup Template](https://abp.io/docs/latest/startup-templates/application/index)
