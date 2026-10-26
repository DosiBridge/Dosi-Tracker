using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Output;
using Dosi.Tracker.AiAssist.Prompts;
using Dosi.Tracker.AiAssist.Read;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>What the echo card is asked.</summary>
public sealed class EchoInput
{
    /// <summary>"self", "team" or "member".</summary>
    public string Scope { get; set; } = AiFeatureScopes.Self;

    /// <summary>The member a "member" request is about.</summary>
    public Guid? UserId { get; set; }

    public DateTime From { get; set; } = new(2026, 7, 20, 0, 0, 0, DateTimeKind.Utc);
    public DateTime To { get; set; } = new(2026, 7, 27, 0, 0, 0, DateTimeKind.Utc);
    public string? TimeZone { get; set; }
    public bool Regenerate { get; set; }

    /// <summary>The facts the bundle carries (none: an empty bundle).</summary>
    public List<(string Label, string Display)> Facts { get; set; } = new() { ("tracked time", "6 h 52 min") };

    /// <summary>A window title, put in the bundle only while the workspace allows window titles.</summary>
    public string? WindowTitle { get; set; }

    public TimeSpan CacheTtl { get; set; } = TimeSpan.FromHours(1);
}

/// <summary>The echo card's answer.</summary>
public sealed record EchoRaw(string Headline);

public sealed class EchoResult
{
    public AiResultMetaDto Meta { get; set; } = new();
    public string Headline { get; set; } = string.Empty;
}

/// <summary>
/// A test-only card for <see cref="AiInsightRunner"/>: one headline citing the facts it was given. It records the steps
/// the runner asked of it, so tests can assert the order of checks and that refused requests read nothing.
/// </summary>
public sealed class EchoInsightFeature : AiInsightFeatureBase<EchoInput, EchoRaw, EchoResult>
{
    public const string NoDataHeadline = "Nothing was tracked in this period.";
    public const string FallbackPrefix = "Basic summary:";

    private const string Schema =
        """{"type":"object","properties":{"headline":{"type":"string"}},"required":["headline"],"additionalProperties":false}""";

    private readonly AiScopeGuard _scopeGuard;
    private readonly List<string> _steps = new();

    public EchoInsightFeature(AiScopeGuard scopeGuard, AiFeature? feature = null, bool hasFallback = true)
    {
        _scopeGuard = scopeGuard;
        HasFallback = hasFallback;
        Prompt = new AiPromptSpec(
            feature ?? AiFeatureIds.PeriodSummary,
            1,
            "Echo the figures in one headline.",
            "Write one headline that cites the facts by their placeholders.",
            AiPromptSpec.ParseSchema(Schema),
            "echo_headline");
    }

    public override AiPromptSpec Prompt { get; }

    public override bool HasFallback { get; }

    /// <summary>"authorize" and "bundle", in the order the runner asked for them.</summary>
    public IReadOnlyList<string> Steps => _steps.ToList();

    public override string? TimeZoneOf(EchoInput input) => input.TimeZone;

    public override bool IsRegenerate(EchoInput input) => input.Regenerate;

    public override async Task<AiAudienceScope> AuthorizeAsync(AiCallerContext caller, EchoInput input)
    {
        _steps.Add("authorize");
        _scopeGuard.EnsureRange(input.From, input.To, AiAssistConsts.MaxPeriodDays);

        switch (input.Scope)
        {
            case AiFeatureScopes.Team:
                _scopeGuard.EnsureTeamScope(caller);
                return new AiAudienceScope(AiUsageRecord.TeamAudience, RangeFrom: input.From, RangeTo: input.To);
            case AiFeatureScopes.Member:
                var subject = await _scopeGuard.EnsureMemberTargetAsync(caller, input.UserId);
                return new AiAudienceScope(AiUsageRecord.MemberAudience, subject, RangeFrom: input.From, RangeTo: input.To);
            default:
                return new AiAudienceScope(AiUsageRecord.SelfAudience, caller.UserId, RangeFrom: input.From, RangeTo: input.To);
        }
    }

    public override Task<AiContextBundle> BuildBundleAsync(AiCallerContext caller, EchoInput input, AiAudienceScope scope)
    {
        _steps.Add("bundle");
        var writer = new AiBundleWriter(caller.UserId);
        foreach (var (label, display) in input.Facts)
        {
            writer.AddFact(label, display);
        }

        if (input.WindowTitle != null && caller.Policy.AllowWindowTitles)
        {
            writer.AddTitle(input.WindowTitle);
        }

        return Task.FromResult(writer.Build());
    }

    public override IReadOnlyList<string> DataCategories(AiContextBundle bundle) =>
        bundle.Labels.Keys.Any(k => k.StartsWith(AiBundleWriter.TitlePrefix, StringComparison.Ordinal))
            ? new[] { AiDataCategories.Durations, AiDataCategories.WindowTitles }
            : new[] { AiDataCategories.Durations };

    public override IReadOnlyList<AiViolation> Validate(EchoRaw raw, AiContextBundle bundle)
    {
        var violations = new List<AiViolation>();
        if (AiGroundingValidator.ValidateRequired("headline", raw.Headline) is { } missing)
        {
            violations.Add(missing);
        }

        violations.AddRange(AiGroundingValidator.ValidateText("headline", raw.Headline, ValidationProfile, bundle.KnownIds));
        return violations;
    }

    public override EchoRaw Fallback(AiContextBundle bundle) =>
        new(bundle.Facts.Count > 0 ? FallbackPrefix + " {F1}." : FallbackPrefix + " nothing to show.");

    public override EchoResult Render(EchoRaw raw, AiContextBundle bundle, AiResultMetaDto meta) => new()
    {
        Meta = meta,
        Headline = AiGroundingValidator.Cap(AiPlaceholderRenderer.Render(raw.Headline, bundle), AiAssistConsts.MaxHeadlineLength)
    };

    public override EchoResult NoData(AiContextBundle bundle, AiResultMetaDto meta) => new()
    {
        Meta = meta,
        Headline = NoDataHeadline
    };

    public override TimeSpan CacheTtl(EchoInput input, AiContextBundle bundle) => input.CacheTtl;
}
