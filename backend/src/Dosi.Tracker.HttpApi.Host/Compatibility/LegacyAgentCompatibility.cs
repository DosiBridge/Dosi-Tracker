using System;
using Dosi.Tracker.Reporting;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Mvc;
using Microsoft.AspNetCore.Mvc.Filters;

namespace Dosi.Tracker.Compatibility;

/// <summary>
/// Keeps desktop agents installed before shared contract v2 working against this server. The Windows agent of that
/// time asks <c>GET /api/app/reporting/summary</c> for its "Tracked today" and "This week" cards (User-Agent
/// <c>dosi-tracker/…</c>, From and To only) and reads <c>averageProductivity</c> as a plain number: the null that now
/// means "nothing was measured" makes it discard the whole answer, so the cards would keep yesterday's or last week's
/// totals. Such a request gets 0 there instead, which is what that agent always got for a range without measured time.
/// Every contract v2 caller — the web app and the current agents, which send <c>TimeZone</c> — gets the null the
/// contract specifies. (The macOS agent of that time never read the summary.)
/// </summary>
public static class LegacyAgentCompatibility
{
    /// <summary>The product token of the Windows agent's User-Agent (<c>dosi-tracker/0.1.0</c>).</summary>
    public const string WindowsAgentProduct = "dosi-tracker";

    /// <summary>The report parameter contract v2 added; agents from before it never send it.</summary>
    public const string TimeZoneParameter = nameof(GetReportSummaryInput.TimeZone);

    /// <summary>True for a request from a Windows agent that predates contract v2: its User-Agent names the agent and
    /// it carries no <see cref="TimeZoneParameter"/> (query keys are matched case-insensitively).</summary>
    public static bool IsPreV2WindowsAgent(HttpRequest request)
    {
        var userAgent = request.Headers.UserAgent.ToString().TrimStart();
        var namesAgent =
            userAgent.StartsWith(WindowsAgentProduct + "/", StringComparison.OrdinalIgnoreCase) ||
            string.Equals(userAgent.TrimEnd(), WindowsAgentProduct, StringComparison.OrdinalIgnoreCase);

        return namesAgent && !request.Query.ContainsKey(TimeZoneParameter);
    }

    /// <summary>The summary as such an agent can read it: <see cref="ReportSummaryDto.AverageProductivity"/> is a number
    /// (0 when nothing was measured). The per-member, per-project and per-day rows stay as they are; the agent never
    /// reads them.</summary>
    public static void AdaptForPreV2Agent(ReportSummaryDto summary)
    {
        summary.AverageProductivity ??= 0;
    }
}

/// <summary>Applies <see cref="LegacyAgentCompatibility"/> to the reporting summary a pre-v2 Windows agent receives.</summary>
public sealed class LegacyAgentSummaryResultFilter : IResultFilter
{
    public void OnResultExecuting(ResultExecutingContext context)
    {
        if (context.Result is ObjectResult { Value: ReportSummaryDto summary } &&
            LegacyAgentCompatibility.IsPreV2WindowsAgent(context.HttpContext.Request))
        {
            LegacyAgentCompatibility.AdaptForPreV2Agent(summary);
        }
    }

    public void OnResultExecuted(ResultExecutedContext context)
    {
    }
}
