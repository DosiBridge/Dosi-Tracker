using System;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Features;
using Microsoft.Extensions.AI;
using Shouldly;
using Volo.Abp.Modularity;
using Volo.Abp.Threading;
using Xunit;

namespace Dosi.Tracker.AiAssist.Assistant;

/// <summary>Ask AI's turn limits: tokens are summed across every round, the call budget is enforced mid-turn, and a
/// caller who goes away is answered with a cancellation (spec 9.2, 10.2, 14).</summary>
public abstract class AiAssistantLoopTests<TStartupModule> : AiFeatureTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    [Fact]
    public async Task Tokens_Should_Be_Summed_Across_Every_Round()
    {
        await ReadyAsync();
        Model.EnqueueToolCall("get_time_summary", "{}", Usage(100, 10))
            .EnqueueToolCall("get_app_usage", "{}", Usage(120, 15))
            .EnqueueText("You tracked {F1} this week.", Usage(140, 20));

        await Ai.AskAsync(new AiAskInput { Message = "How much did I track, and which apps?" });

        var row = (await RowsAsync()).ShouldHaveSingleItem();
        row.InputTokens.ShouldBe(100 + 120 + 140);
        row.OutputTokens.ShouldBe(10 + 15 + 20);
        (row.ToolCalls ?? string.Empty).ShouldContain("get_time_summary:1");
        (row.ToolCalls ?? string.Empty).ShouldContain("get_app_usage:1");
    }

    [Fact]
    public async Task The_Call_Budget_Should_Stop_Further_Tool_Calls_Mid_Turn()
    {
        await ReadyAsync();
        AiOptions.Assistant.MaxToolCallsPerTurn = 1;

        Model.EnqueueToolCall("get_time_summary", "{}")
            .EnqueueToolCall("get_app_usage", "{}")
            .EnqueueText("You tracked {F1} this week.");

        await Ai.AskAsync(new AiAskInput { Message = "How much did I track, and which apps?" });

        // The second tool call ran out of budget; its result (sent back to the model as the next round's context)
        // says so.
        var results = Model.Calls.Last().Messages
            .SelectMany(m => m.Contents.OfType<FunctionResultContent>())
            .Select(c => c.Result?.ToString() ?? string.Empty);
        results.ShouldContain(r => r.Contains("budget_exhausted"));
    }

    [Fact]
    public async Task A_Caller_Who_Goes_Away_Should_Get_A_Cancellation_And_The_Row_Should_Say_Cancelled()
    {
        await ReadyAsync();
        Model.EnqueueToolCall("get_time_summary", "{}").EnqueueDelay(TimeSpan.FromSeconds(30));
        using var request = new CancellationTokenSource();

        using (GetRequiredService<ICancellationTokenProvider>().Use(request.Token))
        {
            request.CancelAfter(TimeSpan.FromMilliseconds(200));
            await Should.ThrowAsync<OperationCanceledException>(() => Ai.AskAsync(new AiAskInput { Message = "How much did I track?" }));
        }

        (await RowsAsync()).ShouldHaveSingleItem().Outcome.ShouldBe(AiUsageOutcomes.Cancelled);
    }

    private static UsageDetails Usage(int input, int output) => new() { InputTokenCount = input, OutputTokenCount = output };
}
