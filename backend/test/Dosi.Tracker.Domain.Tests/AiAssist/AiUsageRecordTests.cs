using System;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist;

/// <summary>The usage-ledger row: what it accepts, what it trims, and which rows count as provider calls.</summary>
public class AiUsageRecordTests
{
    private static AiUsageRecord NewRecord(string outcome = AiUsageOutcomes.Generated, bool cacheHit = false, string? model = "gpt-5.4-mini") =>
        new(Guid.NewGuid(), Guid.NewGuid(), Guid.NewGuid(), AiFeatureIds.AskId, "ask.v1", AiUsageRecord.SelfAudience,
            AiOptions.ProviderOpenAi, model, outcome, cacheHit: cacheHit);

    [Theory]
    [InlineData(AiUsageOutcomes.Generated, false, true)]
    [InlineData(AiUsageOutcomes.Fallback, false, true)]
    [InlineData(AiUsageOutcomes.ProviderError, false, true)]
    [InlineData(AiUsageOutcomes.Timeout, false, true)]
    [InlineData(AiUsageOutcomes.Cancelled, false, true)]
    [InlineData(AiUsageOutcomes.Generated, true, false)]
    [InlineData(AiUsageOutcomes.Cached, false, false)]
    [InlineData(AiUsageOutcomes.NoData, false, false)]
    [InlineData(AiUsageOutcomes.Clarification, false, false)]
    public void Only_Rows_That_Reached_The_Provider_Should_Count(string outcome, bool cacheHit, bool calledProvider)
    {
        NewRecord(outcome, cacheHit).CalledProvider.ShouldBe(calledProvider);
    }

    [Fact]
    public void An_Unknown_Outcome_Should_Be_Refused()
    {
        Should.Throw<ArgumentException>(() => NewRecord("exploded"));
        Should.Throw<ArgumentException>(() => NewRecord().SetOutcome("GENERATED"));
        AiUsageOutcomes.IsKnown(null).ShouldBeFalse();
    }

    [Fact]
    public void Usage_Should_Keep_Valid_Counts_And_Drop_Impossible_Ones()
    {
        var record = NewRecord();

        record.SetUsage(1200, 300, cachedInputTokens: 1000, reasoningTokens: 64, estimatedCostUsd: 0.0012m, latencyMs: 850,
            toolCalls: " list_projects,get_summary ");

        (record.InputTokens, record.OutputTokens, record.CachedInputTokens, record.ReasoningTokens).ShouldBe((1200, 300, 1000, 64));
        record.EstimatedCostUsd.ShouldBe(0.0012m);
        record.LatencyMs.ShouldBe(850);
        record.ToolCalls.ShouldBe("list_projects,get_summary");

        record.SetUsage(-1, -5, -2, -3, -0.5m, -10, "   ");

        record.InputTokens.ShouldBeNull();
        record.OutputTokens.ShouldBeNull();
        record.CachedInputTokens.ShouldBeNull();
        record.ReasoningTokens.ShouldBeNull();
        record.EstimatedCostUsd.ShouldBeNull();
        record.LatencyMs.ShouldBeNull();
        record.ToolCalls.ShouldBeNull();
    }

    [Fact]
    public void Text_Should_Be_Cut_To_The_Column_Lengths()
    {
        var record = NewRecord(model: new string('m', AiUsageRecord.MaxModelLength + 5));
        record.SetUsage(1, 1, toolCalls: new string('t', AiUsageRecord.MaxToolCallsLength + 5));
        record.SetOutcome(AiUsageOutcomes.ProviderError, new string('e', AiUsageRecord.MaxErrorCodeLength + 5));

        record.Model!.Length.ShouldBe(AiUsageRecord.MaxModelLength);
        record.ToolCalls!.Length.ShouldBe(AiUsageRecord.MaxToolCallsLength);
        record.ErrorCode!.Length.ShouldBe(AiUsageRecord.MaxErrorCodeLength);
        NewRecord(model: "  ").Model.ShouldBeNull();
        NewRecord().DataCategories.ShouldBe(string.Empty);
    }

    [Fact]
    public void Feedback_Should_Be_Up_Or_Down_With_A_Short_Reason()
    {
        var record = NewRecord();

        record.SetFeedback(AiUsageRecord.FeedbackDown, "inaccurate");
        (record.FeedbackRating, record.FeedbackReason).ShouldBe(("down", "inaccurate"));

        record.SetFeedback(AiUsageRecord.FeedbackUp, null);
        (record.FeedbackRating, record.FeedbackReason).ShouldBe(("up", (string?)null));

        Should.Throw<ArgumentException>(() => record.SetFeedback("meh", null));
    }

    [Fact]
    public void Month_And_Day_Should_Start_At_Midnight_UTC()
    {
        var now = new DateTime(2026, 10, 26, 13, 45, 12, DateTimeKind.Utc);

        AiUsageRecordQueries.MonthStart(now).ShouldBe(new DateTime(2026, 10, 1, 0, 0, 0, DateTimeKind.Utc));
        AiUsageRecordQueries.DayStart(now).ShouldBe(new DateTime(2026, 10, 26, 0, 0, 0, DateTimeKind.Utc));
        AiUsageRecordQueries.MonthStart(now).Kind.ShouldBe(DateTimeKind.Utc);
    }
}
