# Dosi.Tracker

## About this solution

This is a layered startup solution based on [Domain Driven Design (DDD)](https://abp.io/docs/latest/framework/architecture/domain-driven-design) practises. All the fundamental ABP modules are already installed. Check the [Application Startup Template](https://abp.io/docs/latest/solution-templates/layered-web-application) documentation for more info.

### Pre-requirements

* [.NET10.0+ SDK](https://dotnet.microsoft.com/download/dotnet)
* [Node v18 or 20](https://nodejs.org/en)

### Configurations

The solution comes with a default configuration that works out of the box. However, you may consider to change the following configuration before running your solution:


### Before running the application

* Run `abp install-libs` command on your solution folder to install client-side package dependencies. This step is automatically done when you create a new solution, if you didn't especially disabled it. However, you should run it yourself if you have first cloned this solution from your source control, or added a new client-side package dependency to your solution.
* Run `Dosi.Tracker.DbMigrator` to create the initial database. This step is also automatically done when you create a new solution, if you didn't especially disabled it. This should be done in the first run. It is also needed if a new database migration is added to the solution later.

#### The first platform (host) administrator

The first DbMigrator run on an empty database creates the platform administrator `admin` (email `App:HostAdminEmail`, default `admin@abp.io`) with the password in `App:HostAdminPassword` — set it, e.g. as the environment variable `App__HostAdminPassword`. Outside the `Development` environment the migrator **refuses** to create that account without the setting (or with the template password `1q2w3E*`), because anyone could sign in to a fresh deployment with it. In `Development` (`dotnet run` uses the project's launch profile, which sets `DOTNET_ENVIRONMENT=Development`) it falls back to `1q2w3E*`, and the administrator must change it at the first sign-in. Later runs never create or change administrators.

#### Upgrading to the agent data v2 migrations

`AgentDataV2` and `AgentDataV2Followup` also bring existing data in line with the new rules (PostgreSQL only):

* Subscriptions are enforced from now on: a workspace whose trial ended cannot record time or grow its team until the host activates it (`POST /api/app/platform/activate-subscription`). Workspaces whose trial had already ended before the upgrade (status `past_due`, nothing invoiced) get a new 14-day trial from the moment of the upgrade, so their owners are reminded three days before it ends and told when it ended, and the host can activate the paying ones first.
* The Identity `manager` role is taken from everyone who no longer holds an Admin/Manager membership on an existing project.
* Duplicate memberships are merged (the oldest stays), and every membership's rate matches its latest rate-history entry.
* Activity of blocks that never measured input (projects that allow neither keyboard nor mouse, agents that sent no timeline) becomes "not measured" instead of 0%.

#### Generating a Signing Certificate

In the production environment, you need to use a production signing certificate. ABP Framework sets up signing and encryption certificates in your application and expects an `openiddict.pfx` file in your application.

To generate a signing certificate, you can use the following command:

```bash
dotnet dev-certs https -v -ep openiddict.pfx -p 3f027835-7c52-4bf7-bc1f-77582ca37aef
```

> `3f027835-7c52-4bf7-bc1f-77582ca37aef` is the password of the certificate, you can change it to any password you want.

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
