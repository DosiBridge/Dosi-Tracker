using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Http;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.Diagnostics.HealthChecks;
using Microsoft.Extensions.Hosting;

namespace Dosi.Tracker.Security;

/// <summary>
/// Whether the API serves its diagnostic surfaces anonymously: the Swagger UI and document (the full map of every
/// endpoint, the platform and account ones included) and the HealthChecks UI with detailed health JSON (which carries
/// exception text such as the database host on a failure). Both default to Development only; production keeps a
/// minimal <c>/health-status</c> that reports just the status, for load balancers and container probes.
/// </summary>
public static class DiagnosticsExposure
{
    /// <summary>Serve Swagger (<c>/swagger</c>); default: Development only.</summary>
    public const string SwaggerEnabledKey = "App:SwaggerEnabled";

    /// <summary>Serve the HealthChecks UI (<c>/health-ui</c>, <c>/health-api</c>) and detailed <c>/health-status</c>
    /// JSON; default: Development only.</summary>
    public const string HealthChecksUiEnabledKey = "App:HealthChecksUiEnabled";

    public static bool IsSwaggerEnabled(IConfiguration configuration, IHostEnvironment environment)
    {
        return configuration.GetValue<bool?>(SwaggerEnabledKey) ?? environment.IsDevelopment();
    }

    public static bool IsHealthChecksUiEnabled(IConfiguration configuration, IHostEnvironment environment)
    {
        return configuration.GetValue<bool?>(HealthChecksUiEnabledKey) ?? environment.IsDevelopment();
    }

    /// <summary>The public health response: <c>{"status":"Healthy"}</c> and nothing else (the status code still follows
    /// the result: 503 when unhealthy).</summary>
    public static Task WriteStatusOnlyAsync(HttpContext context, HealthReport report)
    {
        context.Response.ContentType = "application/json";
        return context.Response.WriteAsync(JsonSerializer.Serialize(new { status = report.Status.ToString() }));
    }
}
