using System.Collections.Generic;
using System.Text.Json;
using Dosi.Tracker.Reporting;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Mvc;
using Microsoft.AspNetCore.Mvc.Abstractions;
using Microsoft.AspNetCore.Mvc.Filters;
using Microsoft.AspNetCore.Routing;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Compatibility;

/// <summary>
/// The reporting summary as a Windows agent from before shared contract v2 receives it: it parses
/// <c>averageProductivity</c> as a plain number, so "nothing measured" reaches it as 0 rather than null. Every other
/// caller gets the contract's null.
/// </summary>
public sealed class LegacyAgentCompatibilityTests
{
    private const string PreV2Agent = "dosi-tracker/0.1.0";

    private static HttpContext Request(string? userAgent, string query = "?From=2026-10-05T00:00:00.000Z&To=2026-10-06T00:00:00.000Z")
    {
        var context = new DefaultHttpContext();
        context.Request.Method = HttpMethods.Get;
        context.Request.Path = "/api/app/reporting/summary";
        context.Request.QueryString = new QueryString(query);
        if (userAgent != null)
        {
            context.Request.Headers.UserAgent = userAgent;
        }

        return context;
    }

    /// <summary>Runs the filter over the summary as MVC hands it the action's result, and returns what is serialized.</summary>
    private static string Serialize(HttpContext request, ReportSummaryDto summary)
    {
        var context = new ResultExecutingContext(
            new ActionContext(request, new RouteData(), new ActionDescriptor()),
            new List<IFilterMetadata>(),
            new ObjectResult(summary),
            controller: new object());

        new LegacyAgentSummaryResultFilter().OnResultExecuting(context);

        return JsonSerializer.Serialize(((ObjectResult)context.Result).Value, new JsonSerializerOptions(JsonSerializerDefaults.Web));
    }

    [Theory]
    [InlineData(PreV2Agent)]
    [InlineData("dosi-tracker/0.0.9")]
    [InlineData("Dosi-Tracker/0.1.0")]
    [InlineData("dosi-tracker")]
    public void A_Windows_Agent_Asking_Without_A_Time_Zone_Should_Be_A_Pre_V2_Agent(string userAgent)
    {
        LegacyAgentCompatibility.IsPreV2WindowsAgent(Request(userAgent).Request).ShouldBeTrue();
    }

    [Theory]
    [InlineData(PreV2Agent, "?From=2026-10-05T00:00:00.000Z&To=2026-10-06T00:00:00.000Z&TimeZone=Asia%2FDhaka")]
    [InlineData(PreV2Agent, "?from=2026-10-05&to=2026-10-06&timezone=UTC")]   // query keys are case-insensitive
    [InlineData("DosiTracker/0.2.0 (macOS 14.5)", "")]                          // the macOS agent
    [InlineData("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36", "")] // the web app
    [InlineData("dosi-trackerd/1.0", "")]                                        // another product
    [InlineData(null, "")]
    public void Every_Other_Caller_Should_Not_Be_A_Pre_V2_Agent(string? userAgent, string query)
    {
        LegacyAgentCompatibility.IsPreV2WindowsAgent(Request(userAgent, query).Request).ShouldBeFalse();
    }

    [Fact]
    public void A_Pre_V2_Agent_Should_Read_Nothing_Measured_As_Zero()
    {
        var json = Serialize(Request(PreV2Agent), new ReportSummaryDto { TotalTrackedMinutes = 30, AverageProductivity = null });

        using var body = JsonDocument.Parse(json);
        body.RootElement.GetProperty("averageProductivity").GetDouble().ShouldBe(0);
        body.RootElement.GetProperty("totalTrackedMinutes").GetDouble().ShouldBe(30);
    }

    [Fact]
    public void A_Measured_Average_Should_Reach_A_Pre_V2_Agent_Unchanged()
    {
        var json = Serialize(Request(PreV2Agent), new ReportSummaryDto { AverageProductivity = 72.5 });

        using var body = JsonDocument.Parse(json);
        body.RootElement.GetProperty("averageProductivity").GetDouble().ShouldBe(72.5);
    }

    [Fact]
    public void Contract_V2_Callers_Should_Keep_The_Null()
    {
        var current = Request(PreV2Agent, "?From=2026-10-05T00:00:00.000Z&To=2026-10-06T00:00:00.000Z&TimeZone=Asia%2FDhaka");
        var json = Serialize(current, new ReportSummaryDto { AverageProductivity = null });

        using var body = JsonDocument.Parse(json);
        body.RootElement.GetProperty("averageProductivity").ValueKind.ShouldBe(JsonValueKind.Null);
    }

    [Fact]
    public void Other_Results_Should_Be_Left_Alone()
    {
        var context = new ResultExecutingContext(
            new ActionContext(Request(PreV2Agent), new RouteData(), new ActionDescriptor()),
            new List<IFilterMetadata>(),
            new ObjectResult(new DailyReportRowDto { AverageProductivity = null }),
            controller: new object());

        new LegacyAgentSummaryResultFilter().OnResultExecuting(context);

        ((DailyReportRowDto)((ObjectResult)context.Result).Value!).AverageProductivity.ShouldBeNull();
    }
}
