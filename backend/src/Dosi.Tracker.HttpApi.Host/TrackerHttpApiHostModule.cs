using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Security.Cryptography.X509Certificates;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist;
using Dosi.Tracker.AiAssist.Provider;
using Dosi.Tracker.Billing;
using Dosi.Tracker.Activities.Retention;
using Dosi.Tracker.Compatibility;
using Dosi.Tracker.Devices;
using Dosi.Tracker.Erasure;
using Microsoft.AspNetCore.DataProtection;
using Microsoft.Extensions.Logging;
using Dosi.Tracker.Retention;
using Dosi.Tracker.SaaS;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Mvc;
using Volo.Abp.BackgroundWorkers;
using Microsoft.Extensions.Options;
using Microsoft.AspNetCore.Authentication;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Cors;
using Microsoft.AspNetCore.HttpOverrides;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.AspNetCore.Extensions.DependencyInjection;
using OpenIddict.Validation.AspNetCore;
using OpenIddict.Server.AspNetCore;
using Dosi.Tracker.EntityFrameworkCore;
using Dosi.Tracker.Workspaces;
using Dosi.Tracker.HealthChecks;
using Dosi.Tracker.Security;
using Microsoft.OpenApi;
using Volo.Abp;
using Volo.Abp.Studio;
using Volo.Abp.Account;
using Volo.Abp.Account.Web;
using Volo.Abp.AspNetCore.Auditing;
using Volo.Abp.AspNetCore.Mvc;
using Volo.Abp.Autofac;
using Volo.Abp.Localization;
using Volo.Abp.Modularity;
using Volo.Abp.UI.Navigation.Urls;
using Volo.Abp.VirtualFileSystem;
using Volo.Abp.AspNetCore.Mvc.UI.Bundling;
using Volo.Abp.AspNetCore.Mvc.UI.Theme.Shared;
using Volo.Abp.AspNetCore.Mvc.UI.Theme.LeptonXLite;
using Volo.Abp.AspNetCore.Mvc.UI.Theme.LeptonXLite.Bundling;
using Microsoft.AspNetCore.Hosting;
using Volo.Abp.AspNetCore.Serilog;
using Volo.Abp.Identity;
using Volo.Abp.OpenIddict;
using Volo.Abp.Swashbuckle;
using Volo.Abp.Studio.Client.AspNetCore;
using Volo.Abp.Security.Claims;
using Volo.Abp.BlobStoring;
using Volo.Abp.BlobStoring.Aws;

namespace Dosi.Tracker;

[DependsOn(
    typeof(TrackerHttpApiModule),
    typeof(AbpStudioClientAspNetCoreModule),
    typeof(AbpAspNetCoreMvcUiLeptonXLiteThemeModule),
    typeof(AbpAutofacModule),
    typeof(TrackerApplicationModule),
    typeof(TrackerEntityFrameworkCoreModule),
    typeof(AbpAccountWebOpenIddictModule),
    typeof(AbpSwashbuckleModule),
    typeof(AbpAspNetCoreSerilogModule),
    typeof(AbpBlobStoringAwsModule)
    )]
public class TrackerHttpApiHostModule : AbpModule
{
    public override void PreConfigureServices(ServiceConfigurationContext context)
    {
        var hostingEnvironment = context.Services.GetHostingEnvironment();
        var configuration = context.Services.GetConfiguration();

        PreConfigure<OpenIddictBuilder>(builder =>
        {
            builder.AddValidation(options =>
            {
                options.AddAudiences("Tracker");
                options.UseLocalServer();
                options.UseAspNetCore();
            });
        });

        PreConfigure<OpenIddictServerBuilder>(serverBuilder =>
        {
            // Desktop agents bind their tokens to their install (device_id) so a device can be revoked
            // (docs/agent-api-contract.md).
            serverBuilder.AddEventHandler(ValidateDeviceIdParameter.Descriptor);
            serverBuilder.AddEventHandler(AttachDeviceBinding.Descriptor);

            // RFC 7009 token revocation: agents revoke their refresh token on sign-out. ABP serves it at
            // connect/revocat; connect/revoke is the conventional path the agent contract names.
            serverBuilder.SetRevocationEndpointUris("connect/revocat", "connect/revoke");
        });

        if (!hostingEnvironment.IsDevelopment())
        {
            PreConfigure<AbpOpenIddictAspNetCoreOptions>(options =>
            {
                options.AddDevelopmentEncryptionAndSigningCertificate = false;
            });

            PreConfigure<OpenIddictServerBuilder>(serverBuilder =>
            {
                serverBuilder.AddProductionEncryptionAndSigningCertificate(CertificateFileName, configuration["AuthServer:CertificatePassPhrase"]!);
                serverBuilder.SetIssuer(new Uri(configuration["AuthServer:Authority"]!));
            });
        }
    }

    public override void ConfigureServices(ServiceConfigurationContext context)
    {
        var configuration = context.Services.GetConfiguration();
        var hostingEnvironment = context.Services.GetHostingEnvironment();

        // Fail fast rather than ship well-known secrets: every environment but Development (Staging and custom-named
        // environments hold real data too).
        if (!hostingEnvironment.IsDevelopment())
        {
            RejectDefaultDevSecrets(configuration);
        }

        ConfigureRateLimiting(context);

        if (!configuration.GetValue<bool>("App:DisablePII"))
        {
            Microsoft.IdentityModel.Logging.IdentityModelEventSource.ShowPII = true;
            Microsoft.IdentityModel.Logging.IdentityModelEventSource.LogCompleteSecurityArtifact = true;
        }

        if (!configuration.GetValue<bool>("AuthServer:RequireHttpsMetadata"))
        {
            Configure<OpenIddictServerAspNetCoreOptions>(options =>
            {
                options.DisableTransportSecurityRequirement = true;
            });
        }

        // X-Forwarded-For / -Proto are honoured only from the proxies listed under App:ForwardedHeaders (plus loopback),
        // so the per-client sign-in limits see the real caller behind a load balancer and nobody else can spoof it.
        Configure<ForwardedHeadersOptions>(options => TrustedProxies.Configure(options, configuration));

        if (hostingEnvironment.IsDevelopment())
        {
            context.Services.AddRazorPages()
                .AddRazorRuntimeCompilation();
        }

        ConfigureStudio(hostingEnvironment);
        ConfigureAuthentication(context);
        ConfigureUrls(configuration);
        ConfigureBundles(hostingEnvironment);
        ConfigureConventionalControllers();
        ConfigureHealthChecks(context);
        ConfigureSwagger(context, configuration);
        ConfigureVirtualFileSystem(context);
        ConfigureCors(context, configuration);
        ConfigureDataProtection(context, configuration);
        ConfigureBlobStoring(context, configuration);
        context.Services.AddTrackerDataRetention(configuration);
        context.Services.AddTrackerAiAssist();
        ConfigureRequestSizeLimits(context);
        ConfigureAuditing();
        ConfigureLegacyAgentCompatibility();
        ConfigureWorkspaceRequirement();
    }

    /// <summary>Answers desktop agents installed before shared contract v2 in shapes they can read (see
    /// <see cref="LegacyAgentCompatibility"/>): their reporting summary carries a numeric average activity.</summary>
    private void ConfigureLegacyAgentCompatibility()
    {
        Configure<MvcOptions>(options => options.Filters.Add(new LegacyAgentSummaryResultFilter()));
    }

    /// <summary>Tracker endpoints that work inside a workspace answer 400 Tracker:WorkspaceRequired without one.</summary>
    private void ConfigureWorkspaceRequirement()
    {
        Configure<MvcOptions>(options => options.Filters.Add(new WorkspaceRequiredFilter(
            typeof(TrackerApplicationModule).Assembly,
            typeof(TrackerHttpApiModule).Assembly)));
    }

    /// <summary>Every running desktop agent sends a presence heartbeat each minute (shared contract §3). Recording each
    /// one as a request in the audit log would bury everything else in it, so those requests are not audited.</summary>
    private void ConfigureAuditing()
    {
        Configure<AbpAspNetCoreAuditingOptions>(options =>
        {
            options.IgnoredUrls.Add("/api/app/presence/heartbeat");
        });
    }

    /// <summary>
    /// Captures go to S3 or an S3-compatible store (Cloudflare R2, MinIO, …) when <c>S3:AccessKeyId</c> is set, and
    /// to the database blob provider otherwise, so they work out of the box (dev, small deployments). S3 settings:
    /// <c>S3:AccessKeyId</c>, <c>S3:SecretAccessKey</c>, <c>S3:ContainerName</c> (the bucket), <c>S3:Region</c>
    /// (e.g. "eu-west-1"; "auto" for R2) and/or <c>S3:ServiceUrl</c> (the endpoint of an S3-compatible store, e.g.
    /// "https://&lt;account&gt;.r2.cloudflarestorage.com"), and optionally <c>S3:ForcePathStyle</c> (default: on
    /// with a ServiceUrl, off for AWS), <c>S3:CreateContainerIfNotExists</c> and <c>S3:DisablePayloadSigning</c>.
    /// An incomplete S3 configuration stops the start-up instead of failing every capture upload later.
    /// </summary>
    private void ConfigureBlobStoring(ServiceConfigurationContext context, IConfiguration configuration)
    {
        if (string.IsNullOrWhiteSpace(configuration["S3:AccessKeyId"]))
        {
            return;
        }

        var region = configuration["S3:Region"]?.Trim();
        var serviceUrl = configuration["S3:ServiceUrl"]?.Trim();
        var missing = new List<string>();
        if (string.IsNullOrWhiteSpace(configuration["S3:SecretAccessKey"]))
        {
            missing.Add("S3:SecretAccessKey");
        }

        if (string.IsNullOrWhiteSpace(configuration["S3:ContainerName"]))
        {
            missing.Add("S3:ContainerName (the bucket)");
        }

        if (string.IsNullOrEmpty(region) && string.IsNullOrEmpty(serviceUrl))
        {
            missing.Add("S3:Region or S3:ServiceUrl");
        }

        if (missing.Any())
        {
            throw new AbpInitializationException(
                "S3:AccessKeyId is set, so captures are stored in S3, but these settings are missing: " +
                string.Join(", ", missing) + ".");
        }

        Configure<AbpBlobStoringOptions>(options =>
        {
            options.Containers.ConfigureDefault(container =>
            {
                container.UseAws(aws =>
                {
                    aws.AccessKeyId = configuration["S3:AccessKeyId"];
                    aws.SecretAccessKey = configuration["S3:SecretAccessKey"];
                    aws.ContainerName = configuration["S3:ContainerName"];
                    if (!string.IsNullOrEmpty(region))
                    {
                        aws.Region = region;
                    }

                    if (!string.IsNullOrEmpty(serviceUrl))
                    {
                        aws.ServiceURL = serviceUrl;
                    }

                    aws.CreateContainerIfNotExists = configuration.GetValue("S3:CreateContainerIfNotExists", false);
                    aws.DisablePayloadSigning = configuration.GetValue("S3:DisablePayloadSigning", false);
                });
            });
        });

        // ABP addresses a ServiceUrl endpoint path-style and AWS itself virtual-hosted-style; the setting overrides both.
        var forcePathStyle = configuration.GetValue<bool?>("S3:ForcePathStyle");
        if (forcePathStyle.HasValue)
        {
            context.Services.AddTransient<IAmazonS3ClientFactory>(serviceProvider => new PathStyleAmazonS3ClientFactory(
                serviceProvider.GetRequiredService<Volo.Abp.Caching.IDistributedCache<AwsTemporaryCredentialsCacheItem>>(),
                serviceProvider.GetRequiredService<Volo.Abp.Security.Encryption.IStringEncryptionService>(),
                forcePathStyle.Value));
        }
    }

    /// <summary>ABP's S3 client factory, with path-style addressing as configured by <c>S3:ForcePathStyle</c>.</summary>
    [Volo.Abp.DependencyInjection.DisableConventionalRegistration]
    private sealed class PathStyleAmazonS3ClientFactory : DefaultAmazonS3ClientFactory
    {
        private readonly bool _forcePathStyle;

        public PathStyleAmazonS3ClientFactory(
            Volo.Abp.Caching.IDistributedCache<AwsTemporaryCredentialsCacheItem> cache,
            Volo.Abp.Security.Encryption.IStringEncryptionService stringEncryptionService,
            bool forcePathStyle)
            : base(cache, stringEncryptionService)
        {
            _forcePathStyle = forcePathStyle;
        }

        protected override async Task<Amazon.S3.AmazonS3Config> CreateS3ClientConfigAsync(
            AwsBlobProviderConfiguration configuration, Amazon.RegionEndpoint? region)
        {
            var clientConfig = await base.CreateS3ClientConfigAsync(configuration, region);
            clientConfig.ForcePathStyle = _forcePathStyle;
            return clientConfig;
        }
    }

    /// <summary>Largest request body accepted (shared contract §1). An activity upload carries its captures
    /// base64-encoded in the JSON body: screen ≤ 8 MB, thumbnail ≤ 512 KB and webcam ≤ 2 MB decoded (about 14 MB
    /// encoded), plus the block itself.</summary>
    public const long MaxRequestBodyBytes = 32L * 1024 * 1024;

    /// <summary>
    /// Caps request bodies at <see cref="MaxRequestBodyBytes"/> on Kestrel and on IIS. A larger body is answered
    /// with 413, which tells the agents to resend the block without its images. The server reports the overrun as
    /// a BadHttpRequestException while MVC reads the body, and ABP's exception filter would turn that into a 500,
    /// which the agents retry forever; the status code finder keeps the exception's own status instead.
    /// </summary>
    private void ConfigureRequestSizeLimits(ServiceConfigurationContext context)
    {
        Configure<Microsoft.AspNetCore.Server.Kestrel.Core.KestrelServerOptions>(options =>
        {
            options.Limits.MaxRequestBodySize = MaxRequestBodyBytes;
        });

        Configure<IISServerOptions>(options =>
        {
            options.MaxRequestBodySize = MaxRequestBodyBytes;
        });

        context.Services.AddTransient<
            Volo.Abp.AspNetCore.ExceptionHandling.IHttpExceptionStatusCodeFinder, BadRequestAwareStatusCodeFinder>();
    }

    /// <summary>ABP's HTTP status of an exception, except that a request the server refused to read (body too
    /// large, malformed, too slow) keeps its own 4xx status instead of becoming a 500.</summary>
    [Volo.Abp.DependencyInjection.DisableConventionalRegistration]
    private sealed class BadRequestAwareStatusCodeFinder : Volo.Abp.AspNetCore.ExceptionHandling.DefaultHttpExceptionStatusCodeFinder
    {
        public BadRequestAwareStatusCodeFinder(
            Microsoft.Extensions.Options.IOptions<Volo.Abp.AspNetCore.ExceptionHandling.AbpExceptionHttpStatusCodeOptions> options)
            : base(options)
        {
        }

        public override System.Net.HttpStatusCode GetStatusCode(HttpContext httpContext, Exception exception)
        {
            return exception is BadHttpRequestException badRequest
                ? (System.Net.HttpStatusCode)badRequest.StatusCode
                : base.GetStatusCode(httpContext, exception);
        }
    }

    /// <summary>The OpenIddict signing and encryption certificate outside Development.</summary>
    public const string CertificateFileName = "openiddict.pfx";

    /// <summary>Refuses to start outside Development while a secret is still a well-known value or is still read from
    /// the tracked appsettings.json, or the certificate is a localhost development or expired one (see
    /// <see cref="ProductionSecretsGuard"/>).</summary>
    private static void RejectDefaultDevSecrets(IConfiguration configuration)
    {
        var problems = ProductionSecretsGuard.FindProblems(configuration)
            .Concat(ProductionSecretsGuard.FindCertificateProblems(
                CertificateFileName, configuration[ProductionSecretsGuard.CertificatePassPhraseKey], DateTimeOffset.UtcNow))
            .ToList();
        if (problems.Any())
        {
            throw new AbpInitializationException(
                "Refusing to start outside Development with repository/template secrets. Fix: " + string.Join("; ", problems) +
                ". Supply them via environment variables (e.g. ConnectionStrings__Default), appsettings.secrets.json " +
                "or a secret store.");
        }
    }

    /// <summary>
    /// ASP.NET Core Data Protection keys protect sign-in cookies, antiforgery tokens and the emailed password-reset and
    /// email-confirmation tokens. They are kept in the database (table <c>AppDataProtectionKeys</c>), so they survive
    /// restarts and redeploys and every API instance shares them; <c>DataProtection:KeysDirectory</c> keeps them in a
    /// directory instead (a mounted volume). Keys are scoped to <c>DataProtection:ApplicationName</c> (default
    /// "Dosi.Tracker"), which must be the same on every instance.
    /// </summary>
    private static void ConfigureDataProtection(ServiceConfigurationContext context, IConfiguration configuration)
    {
        var applicationName = configuration["DataProtection:ApplicationName"];
        var builder = context.Services.AddDataProtection()
            .SetApplicationName(string.IsNullOrWhiteSpace(applicationName) ? "Dosi.Tracker" : applicationName.Trim());

        var keysDirectory = configuration["DataProtection:KeysDirectory"];
        if (!string.IsNullOrWhiteSpace(keysDirectory))
        {
            builder.PersistKeysToFileSystem(new DirectoryInfo(keysDirectory));
        }
        else
        {
            builder.PersistKeysToDbContext<TrackerDbContext>();
        }
    }

    /// <summary>Brute-force / abuse protection for the anonymous entry points (token endpoint, account endpoints and
    /// workspace self-registration), keyed by the real client IP; see <see cref="AuthRateLimiting"/>.</summary>
    private void ConfigureRateLimiting(ServiceConfigurationContext context)
    {
        context.Services.AddTrackerAuthRateLimiting(context.Services.GetConfiguration());
    }

    private void ConfigureStudio(IHostEnvironment hostingEnvironment)
    {
        if (hostingEnvironment.IsProduction())
        {
            Configure<AbpStudioClientOptions>(options =>
            {
                options.IsLinkEnabled = false;
            });
        }
    }

    private void ConfigureAuthentication(ServiceConfigurationContext context)
    {
        context.Services.ForwardIdentityAuthenticationForBearer(OpenIddictValidationAspNetCoreDefaults.AuthenticationScheme);
        context.Services.Configure<AbpClaimsPrincipalFactoryOptions>(options =>
        {
            options.IsDynamicClaimsEnabled = true;
        });
    }

    private void ConfigureUrls(IConfiguration configuration)
    {
        Configure<AppUrlOptions>(options =>
        {
            options.Applications["MVC"].RootUrl = configuration["App:SelfUrl"];
            options.RedirectAllowedUrls.AddRange(configuration["App:RedirectAllowedUrls"]?.Split(',') ?? Array.Empty<string>());
        });
    }

    private void ConfigureBundles(IHostEnvironment hostingEnvironment)
    {
        Configure<AbpBundlingOptions>(options =>
        {
            options.StyleBundles.Configure(
                LeptonXLiteThemeBundles.Styles.Global,
                bundle =>
                {
                    bundle.AddFiles("/global-styles.css");
                }
            );

            options.ScriptBundles.Configure(
                LeptonXLiteThemeBundles.Scripts.Global,
                bundle =>
                {
                    bundle.AddFiles("/global-scripts.js");
                    if (hostingEnvironment.IsDevelopment())
                    {
                        bundle.AddFiles("/dev-login-helper.js");
                    }
                }
            );
        });
    }


    private void ConfigureVirtualFileSystem(ServiceConfigurationContext context)
    {
        var hostingEnvironment = context.Services.GetHostingEnvironment();

        if (hostingEnvironment.IsDevelopment())
        {
            // Hot-reload embedded resources from the source tree — only when running from a checkout.
            // A published build (e.g. the docker-compose image run as Development) has no ../src folders,
            // and PhysicalFileProvider throws on a missing root.
            var domainSharedPath = Path.Combine(hostingEnvironment.ContentRootPath, $"..{Path.DirectorySeparatorChar}Dosi.Tracker.Domain.Shared");
            if (!Directory.Exists(domainSharedPath))
            {
                return;
            }

            Configure<AbpVirtualFileSystemOptions>(options =>
            {
                options.FileSets.ReplaceEmbeddedByPhysical<TrackerDomainSharedModule>(domainSharedPath);
                options.FileSets.ReplaceEmbeddedByPhysical<TrackerDomainModule>(Path.Combine(hostingEnvironment.ContentRootPath, $"..{Path.DirectorySeparatorChar}Dosi.Tracker.Domain"));
                options.FileSets.ReplaceEmbeddedByPhysical<TrackerApplicationContractsModule>(Path.Combine(hostingEnvironment.ContentRootPath, $"..{Path.DirectorySeparatorChar}Dosi.Tracker.Application.Contracts"));
                options.FileSets.ReplaceEmbeddedByPhysical<TrackerApplicationModule>(Path.Combine(hostingEnvironment.ContentRootPath, $"..{Path.DirectorySeparatorChar}Dosi.Tracker.Application"));
            });
        }
    }

    private void ConfigureConventionalControllers()
    {
        Configure<AbpAspNetCoreMvcOptions>(options =>
        {
            options.ConventionalControllers.Create(typeof(TrackerApplicationModule).Assembly);
        });
    }

    private static void ConfigureSwagger(ServiceConfigurationContext context, IConfiguration configuration)
    {
        context.Services.AddAbpSwaggerGenWithOidc(
            configuration["AuthServer:Authority"]!,
            ["Tracker"],
            [AbpSwaggerOidcFlows.AuthorizationCode],
            null,
            options =>
            {
                options.SwaggerDoc("v1", new OpenApiInfo { Title = "Tracker API", Version = "v1" });
                options.DocInclusionPredicate((docName, description) => true);
                options.CustomSchemaIds(type => type.FullName);
            });
    }

    private void ConfigureCors(ServiceConfigurationContext context, IConfiguration configuration)
    {
        context.Services.AddCors(options =>
        {
            // App:CorsOrigins, validated (no "*": the API admits credentialed requests).
            var origins = CorsOrigins.Parse(configuration);
            options.AddDefaultPolicy(builder =>
            {
                builder
                    .WithOrigins(origins)
                    .WithAbpExposedHeaders()
                    // Rate-limited answers (sign-in, AI assist) say when to retry; the cross-origin web app needs to read it.
                    .WithExposedHeaders("Retry-After")
                    .SetIsOriginAllowedToAllowWildcardSubdomains()
                    .AllowAnyHeader()
                    .AllowAnyMethod()
                    .AllowCredentials();
            });
        });
    }

    private void ConfigureHealthChecks(ServiceConfigurationContext context)
    {
        context.Services.AddTrackerHealthChecks();
    }


    public override void OnApplicationInitialization(ApplicationInitializationContext context)
    {
        var app = context.GetApplicationBuilder();
        var env = context.GetEnvironment();

        app.UseForwardedHeaders();
        WarnAboutUntrustedProxies(context, env);

        if (env.IsDevelopment())
        {
            app.UseDeveloperExceptionPage();
        }

        app.UseAbpRequestLocalization();

        if (!env.IsDevelopment())
        {
            app.UseErrorPage();
        }

        app.UseRouting();
        app.MapAbpStaticAssets();
        app.UseAbpStudioLink();
        app.UseAbpSecurityHeaders();
        app.UseCors();
        // After CORS, so a 429 still carries Access-Control-Allow-Origin and the web app can tell the user to wait.
        app.UseTrackerAuthRateLimiting();
        app.UseCaptureDeliveryCachePolicy();
        app.UseAuthentication();
        app.UseAbpOpenIddictValidation();

        app.UseUnitOfWork();
        app.UseDynamicClaims();
        // X-Workspace-Id: the workspace the request works in (the caller must be an active member of it).
        app.UseWorkspaceResolution();
        app.UseAuthorization();

        // The API map is not published in production unless App:SwaggerEnabled says so.
        var configuration = context.ServiceProvider.GetRequiredService<IConfiguration>();
        if (DiagnosticsExposure.IsSwaggerEnabled(configuration, env))
        {
            app.UseSwagger();
            app.UseAbpSwaggerUI(options =>
            {
                options.SwaggerEndpoint("/swagger/v1/swagger.json", "Tracker API");
                options.OAuthClientId(configuration["AuthServer:SwaggerClientId"]);
            });
        }

        app.UseAuditing();
        app.UseAbpSerilogEnrichers();
        app.UseConfiguredEndpoints();
    }

    /// <summary>Outside Development with HTTPS required, a deployment that trusts no proxy most likely sits behind one
    /// it does not trust (a container network or an ingress): every token request then fails as plain HTTP.</summary>
    private static void WarnAboutUntrustedProxies(ApplicationInitializationContext context, IWebHostEnvironment env)
    {
        var configuration = context.ServiceProvider.GetRequiredService<IConfiguration>();
        if (env.IsDevelopment() ||
            !configuration.GetValue<bool>("AuthServer:RequireHttpsMetadata") ||
            TrustedProxies.HasConfiguredProxies(configuration))
        {
            return;
        }

        context.ServiceProvider.GetRequiredService<ILogger<TrackerHttpApiHostModule>>().LogWarning(
            "HTTPS is required but no reverse proxy is trusted: X-Forwarded-For/-Proto are honoured only from loopback. " +
            "Behind a proxy on another address set {Section}:KnownNetworks (e.g. 10.0.0.0/8) or :KnownProxies, " +
            "otherwise sign-ins fail as plain HTTP.", TrustedProxies.SectionName);
    }

    public override async Task OnApplicationInitializationAsync(ApplicationInitializationContext context)
    {
        // Runs the synchronous OnApplicationInitialization (middleware pipeline) first.
        await base.OnApplicationInitializationAsync(context);

        // Built now, so the AI provider's state is logged at start-up (and a configured key is read only once).
        context.ServiceProvider.GetRequiredService<IAiChatClientProvider>();

        // Trials and invoices exist only on a deployment that bills (Billing:Enabled; off by default).
        if (context.ServiceProvider.GetRequiredService<IOptions<BillingOptions>>().Value.Enabled)
        {
            await context.AddBackgroundWorkerAsync<TrialExpiryWorker>();
            await context.AddBackgroundWorkerAsync<InvoiceGenerationWorker>();
        }

        // Purges deleted workspaces after their grace period and scrubs deleted accounts (DataErasure:IsEnabled).
        if (context.ServiceProvider.GetRequiredService<IOptions<DataErasureOptions>>().Value.IsEnabled)
        {
            await context.AddBackgroundWorkerAsync<DataErasureWorker>();
        }

        var retentionWorkers = DataRetentionHosting.WorkerTypes(
            context.ServiceProvider.GetRequiredService<IOptions<CaptureRetentionOptions>>().Value,
            context.ServiceProvider.GetRequiredService<IOptions<ActivityRetentionOptions>>().Value);
        foreach (var worker in retentionWorkers)
        {
            await context.AddBackgroundWorkerAsync(worker);
        }
    }
}
