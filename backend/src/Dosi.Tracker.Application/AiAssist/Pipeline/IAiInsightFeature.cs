using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Output;
using Dosi.Tracker.AiAssist.Prompts;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>
/// Whose data one insight request is about, as <see cref="IAiInsightFeature{TIn,TRaw,TOut}.AuthorizeAsync"/> approved
/// it: the ledger's audience columns, the reader the prompt is written for, and part of the result-cache key.
/// </summary>
/// <param name="Audience"><see cref="AiUsageRecord.SelfAudience"/>, <see cref="AiUsageRecord.MemberAudience"/>,
/// <see cref="AiUsageRecord.TeamAudience"/> or <see cref="AiUsageRecord.ProjectAudience"/>.</param>
/// <param name="SubjectUserId">The member the request is about, when it is about one.</param>
/// <param name="RangeFrom">The start of the range (UTC), when there is one.</param>
/// <param name="RangeTo">The end of the range (UTC, exclusive).</param>
/// <param name="ReportSlug">The report a report explanation is about.</param>
/// <param name="DayBasis">The day basis of a report explanation.</param>
public sealed record AiAudienceScope(
    string Audience,
    Guid? SubjectUserId = null,
    Guid? ProjectId = null,
    DateTime? RangeFrom = null,
    DateTime? RangeTo = null,
    string? ReportSlug = null,
    string? DayBasis = null);

/// <summary>
/// One AI card (period summary, member day, …) as <see cref="AiInsightRunner"/> runs it. The runner owns the checks,
/// the limits, the cache, the provider call, the repair turn and the ledger; a feature only says who may ask what
/// (<see cref="AuthorizeAsync"/>), what the model is told (<see cref="BuildBundleAsync"/>, <see cref="Prompt"/>), what
/// counts as a valid answer (<see cref="Validate"/>) and how an answer is shown (<see cref="Render"/>).
/// </summary>
/// <typeparam name="TIn">The request DTO.</typeparam>
/// <typeparam name="TRaw">The model's answer as parsed (placeholders, no real names); also what the result cache holds.
/// Read with <see cref="AiOutputParser"/>, so positional records with camelCase-mapped parameters.</typeparam>
/// <typeparam name="TOut">The DTO the caller gets.</typeparam>
public interface IAiInsightFeature<TIn, TRaw, TOut>
    where TRaw : class
{
    /// <summary>One of <see cref="AiFeatureIds"/>.</summary>
    string FeatureId { get; }

    /// <summary>"{featureId}@{n}" (<see cref="AiPromptSpec.Version"/>): stored in the ledger and part of the cache key.</summary>
    string PromptVersion { get; }

    /// <summary>Which output rules <see cref="Validate"/> applies.</summary>
    AiValidationProfile ValidationProfile { get; }

    /// <summary>The instructions, task line and answer schema.</summary>
    AiPromptSpec Prompt { get; }

    /// <summary>False when the feature has no deterministic answer of its own (task drafts): a failed or invalid answer
    /// is then refused with <c>Tracker:AiProviderUnavailable</c> or <c>Tracker:AiOutputInvalid</c>.</summary>
    bool HasFallback { get; }

    /// <summary>The time zone the request names, or null (the caller's reporting zone is used).</summary>
    string? TimeZoneOf(TIn input);

    /// <summary>The caller asked for a fresh answer instead of a cached one.</summary>
    bool IsRegenerate(TIn input);

    /// <summary>
    /// Decides what the request is about, refusing (never narrowing) anything the caller may not see: through
    /// <see cref="Read.AiScopeGuard"/>, so team scope without the team permissions is an <c>AbpAuthorizationException</c>,
    /// another member must be an active member, ranges are capped.
    /// </summary>
    Task<AiAudienceScope> AuthorizeAsync(AiCallerContext caller, TIn input);

    /// <summary>Reads, as the caller, what the model may use, already minimized and pseudonymized. An empty bundle is
    /// answered with <see cref="NoData"/> and sends nothing.</summary>
    Task<AiContextBundle> BuildBundleAsync(AiCallerContext caller, TIn input, AiAudienceScope scope);

    /// <summary>The <see cref="AiDataCategories"/> the bundle carries (what the result says was sent).</summary>
    IReadOnlyList<string> DataCategories(AiContextBundle bundle);

    /// <summary>What is wrong with a parsed answer (empty when nothing is): the shape the schema cannot express, the
    /// grounding rules of <see cref="ValidationProfile"/>, route and choice allowlists.</summary>
    IReadOnlyList<AiViolation> Validate(TRaw raw, AiContextBundle bundle);

    /// <summary>The deterministic answer written from the same facts (only called when <see cref="HasFallback"/>).</summary>
    TRaw Fallback(AiContextBundle bundle);

    /// <summary>The caller's DTO: placeholders replaced from the current bundle, text capped.</summary>
    TOut Render(TRaw raw, AiContextBundle bundle, AiResultMetaDto meta);

    /// <summary>The fixed answer for an empty bundle.</summary>
    TOut NoData(AiContextBundle bundle, AiResultMetaDto meta);

    /// <summary>How long a generated answer is served from the cache (zero: not cached).</summary>
    TimeSpan CacheTtl(TIn input, AiContextBundle bundle);

    /// <summary>The texts a <see cref="AiValidationProfile.Creative"/> answer is made from, whose digits it may repeat.</summary>
    IReadOnlyCollection<string> QuotedInputs(AiContextBundle bundle);
}

/// <summary>Defaults shared by the insight features: grounded output, a fallback, nothing quoted.</summary>
public abstract class AiInsightFeatureBase<TIn, TRaw, TOut> : IAiInsightFeature<TIn, TRaw, TOut>
    where TRaw : class
{
    public string FeatureId => Prompt.Feature.Id;

    public string PromptVersion => Prompt.Version;

    public virtual AiValidationProfile ValidationProfile => AiValidationProfile.Grounded;

    public abstract AiPromptSpec Prompt { get; }

    public virtual bool HasFallback => true;

    public abstract string? TimeZoneOf(TIn input);

    public abstract bool IsRegenerate(TIn input);

    public abstract Task<AiAudienceScope> AuthorizeAsync(AiCallerContext caller, TIn input);

    public abstract Task<AiContextBundle> BuildBundleAsync(AiCallerContext caller, TIn input, AiAudienceScope scope);

    public abstract IReadOnlyList<string> DataCategories(AiContextBundle bundle);

    public abstract IReadOnlyList<AiViolation> Validate(TRaw raw, AiContextBundle bundle);

    public virtual TRaw Fallback(AiContextBundle bundle) =>
        throw new NotSupportedException($"The {FeatureId} AI feature has no fallback.");

    public abstract TOut Render(TRaw raw, AiContextBundle bundle, AiResultMetaDto meta);

    public abstract TOut NoData(AiContextBundle bundle, AiResultMetaDto meta);

    public abstract TimeSpan CacheTtl(TIn input, AiContextBundle bundle);

    public virtual IReadOnlyCollection<string> QuotedInputs(AiContextBundle bundle) => Array.Empty<string>();
}
