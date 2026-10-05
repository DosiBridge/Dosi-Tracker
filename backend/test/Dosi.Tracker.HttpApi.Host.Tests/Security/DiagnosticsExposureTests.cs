using System;
using System.Collections.Generic;
using System.IO;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Http;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.Diagnostics.HealthChecks;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Hosting.Internal;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Security;

public class DiagnosticsExposureTests
{
    private static IHostEnvironment Environment(string name) => new HostingEnvironment { EnvironmentName = name };

    private static IConfiguration Configuration(string? key = null, string? value = null)
    {
        var settings = new Dictionary<string, string?>();
        if (key != null)
        {
            settings[key] = value;
        }

        return new ConfigurationBuilder().AddInMemoryCollection(settings).Build();
    }

    [Theory]
    [InlineData("Development", true)]
    [InlineData("Production", false)]
    [InlineData("Staging", false)]
    public void Swagger_And_The_Health_UI_Default_To_Development_Only(string environment, bool exposed)
    {
        DiagnosticsExposure.IsSwaggerEnabled(Configuration(), Environment(environment)).ShouldBe(exposed);
        DiagnosticsExposure.IsHealthChecksUiEnabled(Configuration(), Environment(environment)).ShouldBe(exposed);
    }

    [Fact]
    public void The_Configuration_Flags_Win_In_Either_Direction()
    {
        DiagnosticsExposure.IsSwaggerEnabled(Configuration(DiagnosticsExposure.SwaggerEnabledKey, "true"), Environment("Production"))
            .ShouldBeTrue();
        DiagnosticsExposure.IsSwaggerEnabled(Configuration(DiagnosticsExposure.SwaggerEnabledKey, "false"), Environment("Development"))
            .ShouldBeFalse();
        DiagnosticsExposure.IsHealthChecksUiEnabled(Configuration(DiagnosticsExposure.HealthChecksUiEnabledKey, "true"), Environment("Production"))
            .ShouldBeTrue();
        DiagnosticsExposure.IsHealthChecksUiEnabled(Configuration(DiagnosticsExposure.HealthChecksUiEnabledKey, "false"), Environment("Development"))
            .ShouldBeFalse();
    }

    [Fact]
    public async Task The_Public_Health_Response_Should_Carry_The_Status_Only()
    {
        var context = new DefaultHttpContext();
        context.Response.Body = new MemoryStream();
        var report = new HealthReport(
            new Dictionary<string, HealthReportEntry>
            {
                ["Tracker DbContext Check"] = new(
                    HealthStatus.Unhealthy,
                    "Error when trying to get database record.",
                    TimeSpan.FromMilliseconds(12),
                    new InvalidOperationException("Failed to connect to db.internal:5432"),
                    null)
            },
            TimeSpan.FromMilliseconds(12));

        await DiagnosticsExposure.WriteStatusOnlyAsync(context, report);

        context.Response.ContentType.ShouldBe("application/json");
        context.Response.Body.Position = 0;
        var body = await new StreamReader(context.Response.Body).ReadToEndAsync();
        body.ShouldBe("{\"status\":\"Unhealthy\"}");
    }
}
