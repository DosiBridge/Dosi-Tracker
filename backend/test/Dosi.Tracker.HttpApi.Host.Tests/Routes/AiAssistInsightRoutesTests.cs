using System;
using Dosi.Tracker.AiAssist;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Routes;

/// <summary>The insight cards' routes (frozen for the web app), as ABP derives them from the application service.</summary>
public class AiAssistInsightRoutesTests
{
    [Theory]
    [InlineData(nameof(AiAssistAppService.CreatePeriodSummaryAsync), "api/app/ai-assist/period-summary")]
    [InlineData(nameof(AiAssistAppService.CreateMemberDaySummaryAsync), "api/app/ai-assist/member-day-summary")]
    [InlineData(nameof(AiAssistAppService.CreateReportExplanationAsync), "api/app/ai-assist/report-explanation")]
    public void Insight_Cards_Should_Have_The_Contract_Routes(string methodName, string route)
    {
        ConventionalRoutes.Of(typeof(AiAssistAppService), methodName).ShouldBe(("POST", route));
    }
}
