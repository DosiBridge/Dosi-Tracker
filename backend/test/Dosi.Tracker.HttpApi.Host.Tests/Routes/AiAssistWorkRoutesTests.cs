using System;
using Dosi.Tracker.AiAssist;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Routes;

/// <summary>The work helpers' and feedback's routes (frozen for the web app).</summary>
public class AiAssistWorkRoutesTests
{
    [Theory]
    [InlineData(nameof(AiAssistAppService.CreateDataQualityReviewAsync), "api/app/ai-assist/data-quality-review")]
    [InlineData(nameof(AiAssistAppService.CreateNoteSuggestionsAsync), "api/app/ai-assist/note-suggestions")]
    [InlineData(nameof(AiAssistAppService.CreateTaskDraftsAsync), "api/app/ai-assist/task-drafts")]
    [InlineData(nameof(AiAssistAppService.CreateFeedbackAsync), "api/app/ai-assist/feedback")]
    public void Work_Helpers_Should_Have_The_Contract_Routes(string methodName, string route)
    {
        ConventionalRoutes.Of(typeof(AiAssistAppService), methodName).ShouldBe(("POST", route));
    }
}
