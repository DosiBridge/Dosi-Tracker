using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Features;
using Shouldly;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.AiAssist.Assistant;

/// <summary>
/// Ask AI's answer must cite every number as a fact placeholder, use no judging language and carry no link: a
/// violation earns one repair turn, and an answer that stays invalid falls back to the deterministic facts list
/// (spec 9.2, 12.4, 12.5).
/// </summary>
public abstract class AiAssistantValidationTests<TStartupModule> : AiFeatureTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    [Fact]
    public async Task A_Digit_In_The_Answer_Should_Be_Repaired()
    {
        await ReadyAsync();
        Model.EnqueueToolCall("get_time_summary", "{}")
            .EnqueueText("You tracked 5 hours this week.")
            .EnqueueText("You tracked {F1} this week.");

        var result = await Ai.AskAsync(new AiAskInput { Message = "How much did I track?" });

        Model.Calls.Count.ShouldBe(3);
        result.Answer.ShouldNotContain("5 hours");
        result.Answer.ShouldContain("tracked");
        result.IsClarification.ShouldBeFalse();
    }

    [Fact]
    public async Task Banned_Language_That_Never_Gets_Fixed_Should_Fall_Back_To_The_Facts_List()
    {
        await ReadyAsync();
        Model.EnqueueToolCall("get_time_summary", "{}")
            .EnqueueText("{ME} was lazy about it.")
            .EnqueueText("{ME} was still lazy about it.");

        var result = await Ai.AskAsync(new AiAskInput { Message = "How much did I track?" });

        Model.Calls.Count.ShouldBe(3);
        result.Answer.ShouldContain("I couldn't put together a reliable answer");
        result.Answer.ShouldNotContain("lazy");
    }

    [Fact]
    public async Task A_Link_In_The_Answer_Should_Be_Repaired()
    {
        await ReadyAsync();
        Model.EnqueueToolCall("get_time_summary", "{}")
            .EnqueueText("See https://example.com for details.")
            .EnqueueText("Check the reports page for details.");

        var result = await Ai.AskAsync(new AiAskInput { Message = "How much did I track?" });

        Model.Calls.Count.ShouldBe(3);
        result.Answer.ShouldNotContain("https://");
    }

    [Fact]
    public async Task A_Clean_Answer_Should_Need_No_Repair()
    {
        await ReadyAsync();
        Model.EnqueueToolCall("get_time_summary", "{}").EnqueueText("You tracked {F1} this week.");

        var result = await Ai.AskAsync(new AiAskInput { Message = "How much did I track?" });

        Model.Calls.Count.ShouldBe(2);
        result.Answer.ShouldNotBeEmpty();
    }
}
