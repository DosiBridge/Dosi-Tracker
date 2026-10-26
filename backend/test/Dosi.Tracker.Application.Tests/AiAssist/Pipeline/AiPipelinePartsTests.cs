using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Output;
using Dosi.Tracker.AiAssist.Prompts;
using Dosi.Tracker.AiAssist.Read;
using Microsoft.Extensions.AI;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>The pure pieces of the pipeline: the cache key, token sums, the call options and the error data.</summary>
public class AiPipelinePartsTests
{
    private static readonly Guid Workspace = Guid.Parse("11111111-1111-4111-8111-111111111111");
    private static readonly Guid Caller = Guid.Parse("22222222-2222-4222-8222-222222222222");
    private static readonly DateTime From = new(2026, 10, 19, 0, 0, 0, DateTimeKind.Utc);

    private static readonly AiWorkspacePolicy Policy = AiWorkspacePolicy.Default with
    {
        IsEnabled = true, ConsentVersion = "1.0", EnabledAt = new DateTime(2026, 10, 1, 8, 0, 0, DateTimeKind.Utc)
    };

    private static AiContextBundle Bundle(string display = "6 h 52 min")
    {
        var writer = new AiBundleWriter(Caller);
        writer.AddFact("tracked time", display);
        return writer.Build();
    }

    private static string Key(AiAudienceScope scope, AiWorkspacePolicy? policy = null, string zone = "UTC", AiContextBundle? bundle = null) =>
        AiResultCache.BuildKey("period-summary@1", Workspace, policy ?? Policy, scope, zone, "gpt-5.4-mini", bundle ?? Bundle());

    [Fact]
    public void The_Cache_Key_Should_Follow_The_Documented_Layout()
    {
        var scope = new AiAudienceScope(AiUsageRecord.TeamAudience, RangeFrom: From, RangeTo: From.AddDays(7));
        var parts = Key(scope).Split(':');

        parts.Length.ShouldBe(9);
        parts[0].ShouldBe("ai");
        parts[1].ShouldBe("period-summary@1");
        parts[2].ShouldBe(Workspace.ToString("N"));
        parts[3].ShouldBe(Policy.EnabledAt!.Value.Ticks.ToString());
        parts[4].ShouldBe(AiUsageRecord.TeamAudience);
        parts[5].Length.ShouldBe(64);
        parts[6].ShouldBe(AiResultCache.Language);
        parts[7].ShouldBe("gpt-5.4-mini");
        parts[8].ShouldBe(AiCanonicalJson.Hash(Bundle()));
    }

    [Fact]
    public void The_Cache_Key_Should_Change_With_Every_Part_Of_The_Scope_And_Nothing_Else()
    {
        var scope = new AiAudienceScope(AiUsageRecord.MemberAudience, Caller, Guid.NewGuid(), From, From.AddDays(1), "weekly", "zone");
        var key = Key(scope);

        Key(scope).ShouldBe(key);
        Key(scope with { RangeFrom = DateTime.SpecifyKind(From, DateTimeKind.Unspecified) }).ShouldBe(key);
        Key(scope with { RangeFrom = From.ToLocalTime() }).ShouldBe(key);

        var variants = new[]
        {
            Key(scope with { SubjectUserId = Guid.NewGuid() }),
            Key(scope with { ProjectId = null }),
            Key(scope with { RangeFrom = From.AddMinutes(1) }),
            Key(scope with { RangeTo = null }),
            Key(scope with { ReportSlug = "projects" }),
            Key(scope with { DayBasis = "member" }),
            Key(scope with { Audience = AiUsageRecord.SelfAudience }),
            Key(scope, zone: "Asia/Dhaka"),
            Key(scope, Policy with { AllowWindowTitles = true }),
            Key(scope, Policy with { AllowNotes = true }),
            Key(scope, Policy with { ConsentVersion = "1.1" }),
            Key(scope, Policy with { EnabledAt = Policy.EnabledAt!.Value.AddTicks(1) }),
            Key(scope, bundle: Bundle("7 h"))
        };

        variants.ShouldAllBe(v => v != key);
        variants.ShouldBeUnique();
        Key(scope, Policy with { EnabledAt = null }).Split(':')[3].ShouldBe("0");
    }

    [Fact]
    public void Token_Usage_Should_Sum_Answers_And_Ignore_Unknown_Or_Negative_Counts()
    {
        var usage = AiTokenUsage.None;
        usage.IsEmpty.ShouldBeTrue();

        usage = usage.Add(null);
        usage.IsEmpty.ShouldBeTrue();

        usage = usage.Add(new UsageDetails { InputTokenCount = 100, OutputTokenCount = 20, CachedInputTokenCount = 40 });
        usage = usage.Add(new UsageDetails { InputTokenCount = 50, OutputTokenCount = -1, ReasoningTokenCount = 7 });

        usage.ShouldBe(new AiTokenUsage(150, 20, 40, 7));
        usage.IsEmpty.ShouldBeFalse();

        new AiTokenUsage(int.MaxValue, null, null, null).Add(new UsageDetails { InputTokenCount = 5 })
            .InputTokens.ShouldBe(int.MaxValue);
    }

    [Theory]
    [InlineData("low", ReasoningEffort.Low)]
    [InlineData(" Medium ", ReasoningEffort.Medium)]
    [InlineData("HIGH", ReasoningEffort.High)]
    public void Reasoning_Effort_Should_Map_Known_Values(string effort, ReasoningEffort expected)
    {
        AiInsightRunner.MapReasoning(effort)!.Effort.ShouldBe(expected);
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("none")]
    [InlineData("extreme")]
    public void Unknown_Reasoning_Effort_Should_Send_None(string? effort)
    {
        AiInsightRunner.MapReasoning(effort).ShouldBeNull();
    }

    [Fact]
    public void The_Estimate_Should_Be_Prompt_Characters_Over_Four_Plus_The_Output_Limit()
    {
        var messages = new List<ChatMessage> { new(ChatRole.System, new string('a', 400)), new(ChatRole.User, new string('b', 201)) };

        AiInsightRunner.EstimateTokens(messages, 700).ShouldBe(150 + 700);
        AiInsightRunner.EstimateTokens(messages, -5).ShouldBe(150);
    }

    [Fact]
    public void Chat_Options_Should_Prefer_The_Configured_Output_Limit()
    {
        var prompt = new AiPromptSpec(AiFeatureIds.NoteSuggestions, 1, "Suggest notes.", "Suggest notes.",
            AiPromptSpec.ParseSchema("""{"type":"object","properties":{},"required":[],"additionalProperties":false}"""),
            "notes", maxOutputTokens: 250);
        var options = new AiOptions();

        var schema = AiInsightRunner.BuildChatOptions(prompt, AiFeatureIds.NoteSuggestions, options, jsonObjectMode: false);
        schema.MaxOutputTokens.ShouldBe(250);
        var format = schema.ResponseFormat.ShouldBeOfType<ChatResponseFormatJson>();
        format.SchemaName.ShouldBe("notes");
        format.SchemaDescription.ShouldBe("Suggest notes.");

        options.Features["NoteSuggestions"] = new AiFeatureOptions { MaxOutputTokens = 90 };
        AiInsightRunner.BuildChatOptions(prompt, AiFeatureIds.NoteSuggestions, options, jsonObjectMode: true)
            .MaxOutputTokens.ShouldBe(90);
    }

    [Fact]
    public void Error_Data_Should_Use_Iso_Utc_And_At_Least_One_Second()
    {
        AiErrors.FormatUtc(new DateTime(2026, 11, 1, 0, 0, 0, DateTimeKind.Utc)).ShouldBe("2026-11-01T00:00:00Z");
        AiErrors.FormatUtc(new DateTime(2026, 11, 1, 0, 0, 0, DateTimeKind.Unspecified)).ShouldBe("2026-11-01T00:00:00Z");
        var local = new DateTime(2026, 11, 1, 12, 0, 0, DateTimeKind.Utc).ToLocalTime();
        AiErrors.FormatUtc(local).ShouldBe("2026-11-01T12:00:00Z");

        AiErrors.RateLimited(AiErrors.MinuteReason, 0, "ask").Data["retryAfterSeconds"].ShouldBe(1);
        AiErrors.ProviderUnavailable("error", null, "ask").Data.Contains("retryAfterSeconds").ShouldBeFalse();
        AiErrors.OutputInvalid("task-drafts").Code.ShouldBe(TrackerDomainErrorCodes.AiOutputInvalid);
    }

    [Fact]
    public void A_Feature_Should_Default_To_Grounded_Output_A_Fallback_And_No_Quoted_Inputs()
    {
        var feature = new MinimalFeature();

        feature.FeatureId.ShouldBe(AiFeatureIds.TaskDraftsId);
        feature.PromptVersion.ShouldBe("task-drafts@2");
        feature.ValidationProfile.ShouldBe(AiValidationProfile.Grounded);
        feature.HasFallback.ShouldBeTrue();
        feature.QuotedInputs(Bundle()).ShouldBeEmpty();
        Should.Throw<NotSupportedException>(() => feature.Fallback(Bundle()));
    }

    [Theory]
    [InlineData("Ayesha", "Rahman", "ayesha", "Ayesha Rahman")]
    [InlineData("Ayesha", null, "ayesha", "Ayesha")]
    [InlineData(null, " Rahman ", "ayesha", "Rahman")]
    [InlineData(" ", null, "ayesha", "ayesha")]
    public void A_Roster_Entry_Should_Show_The_Full_Name_Else_The_User_Name(string? name, string? surname, string userName, string expected)
    {
        new AiRosterEntry(Guid.NewGuid(), userName, name, surname, "member", false, From).DisplayName.ShouldBe(expected);
    }

    private sealed class MinimalFeature : AiInsightFeatureBase<object, EchoRaw, object>
    {
        public override AiPromptSpec Prompt { get; } = new(AiFeatureIds.TaskDrafts, 2, "Draft tasks.", "Draft tasks.",
            AiPromptSpec.ParseSchema("""{"type":"object","properties":{},"required":[],"additionalProperties":false}"""), "drafts");

        public override string? TimeZoneOf(object input) => null;
        public override bool IsRegenerate(object input) => false;
        public override Task<AiAudienceScope> AuthorizeAsync(AiCallerContext caller, object input) => throw new NotImplementedException();
        public override Task<AiContextBundle> BuildBundleAsync(AiCallerContext caller, object input, AiAudienceScope scope) => throw new NotImplementedException();
        public override IReadOnlyList<string> DataCategories(AiContextBundle bundle) => Array.Empty<string>();
        public override IReadOnlyList<AiViolation> Validate(EchoRaw raw, AiContextBundle bundle) => Array.Empty<AiViolation>();
        public override object Render(EchoRaw raw, AiContextBundle bundle, AiResultMetaDto meta) => raw;
        public override object NoData(AiContextBundle bundle, AiResultMetaDto meta) => meta;
        public override TimeSpan CacheTtl(object input, AiContextBundle bundle) => TimeSpan.Zero;
    }
}
