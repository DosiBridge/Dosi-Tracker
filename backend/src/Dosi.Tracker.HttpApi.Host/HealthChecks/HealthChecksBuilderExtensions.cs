using System;
using Dosi.Tracker.Security;
using HealthChecks.UI.Client;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Diagnostics.HealthChecks;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Routing;
using Microsoft.Extensions.DependencyInjection;

namespace Dosi.Tracker.HealthChecks;

public static class HealthChecksBuilderExtensions
{
    /// <summary>
    /// <c>/health-status</c> always answers (load balancers, container probes), with the status only unless the
    /// diagnostics are exposed (<see cref="DiagnosticsExposure.HealthChecksUiEnabledKey"/>, Development by default);
    /// then it returns the detailed report and the HealthChecks UI is served at <c>/health-ui</c> / <c>/health-api</c>.
    /// </summary>
    public static void AddTrackerHealthChecks(this IServiceCollection services)
    {
        // Add your health checks here
        var healthChecksBuilder = services.AddHealthChecks();
        healthChecksBuilder.AddCheck<TrackerDatabaseCheck>("Tracker DbContext Check", tags: new string[] { "database" });

        var configuration = services.GetConfiguration();
        var detailed = DiagnosticsExposure.IsHealthChecksUiEnabled(configuration, services.GetHostingEnvironment());

        services.ConfigureHealthCheckEndpoint("/health-status", detailed);

        if (!detailed)
        {
            return;
        }

        var healthCheckUrl = configuration["App:HealthCheckUrl"];

        if (string.IsNullOrEmpty(healthCheckUrl))
        {
            healthCheckUrl = "/health-status";
        }

        var healthChecksUiBuilder = services.AddHealthChecksUI(settings =>
        {
            settings.AddHealthCheckEndpoint("Tracker Health Status", configuration["App:HealthUiCheckUrl"] ?? healthCheckUrl);
        });

        // Set your HealthCheck UI Storage here
        healthChecksUiBuilder.AddInMemoryStorage();

        services.MapHealthChecksUiEndpoints(options =>
        {
            options.UIPath = "/health-ui";
            options.ApiPath = "/health-api";
        });
    }

    private static IServiceCollection ConfigureHealthCheckEndpoint(this IServiceCollection services, string path, bool detailed)
    {
        services.Configure<AbpEndpointRouterOptions>(options =>
        {
            options.EndpointConfigureActions.Add(endpointContext =>
            {
                endpointContext.Endpoints.MapHealthChecks(
                    new PathString(path.EnsureStartsWith('/')),
                    new HealthCheckOptions
                    {
                        Predicate = _ => true,
                        ResponseWriter = detailed
                            ? UIResponseWriter.WriteHealthCheckUIResponse
                            : DiagnosticsExposure.WriteStatusOnlyAsync,
                        AllowCachingResponses = false,
                    });
            });
        });

        return services;
    }

    private static IServiceCollection MapHealthChecksUiEndpoints(this IServiceCollection services, Action<global::HealthChecks.UI.Configuration.Options>? setupOption = null)
    {
        services.Configure<AbpEndpointRouterOptions>(routerOptions =>
        {
            routerOptions.EndpointConfigureActions.Add(endpointContext =>
            {
                endpointContext.Endpoints.MapHealthChecksUI(setupOption);
            });
        });

        return services;
    }
}
