using Dosi.Tracker.AiAssist;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Routes;

/// <summary>Ask AI's route (frozen for the web app).</summary>
public class AiAssistantRoutesTests
{
    [Fact]
    public void Ask_Should_Have_The_Contract_Route()
    {
        ConventionalRoutes.Of(typeof(AiAssistAppService), nameof(AiAssistAppService.AskAsync)).ShouldBe(("POST", "api/app/ai-assist/ask"));
    }
}
