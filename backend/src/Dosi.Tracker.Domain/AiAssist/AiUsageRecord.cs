using System;
using System.Linq;
using Dosi.Tracker.Workspaces;
using Volo.Abp;
using Volo.Abp.Domain.Entities.Auditing;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// The usage ledger: one row per AI request that was served (refusals write none). It holds who asked, which feature,
/// about whom and what range, how it ended, and the tokens and cost — never a prompt, an answer or a name. Quotas,
/// the usage views and the platform overview are all sums over these rows. Kept for <c>AI:Usage:RetentionDays</c>
/// (<see cref="AiUsageRetentionWorker"/>).
/// </summary>
public class AiUsageRecord : CreationAuditedEntity<Guid>, IWorkspaceScoped
{
    public const int MaxFeatureIdLength = 48;
    public const int MaxPromptVersionLength = 64;
    public const int MaxAudienceLength = 16;
    public const int MaxDataCategoriesLength = 256;
    public const int MaxProviderLength = 32;
    public const int MaxModelLength = 128;
    public const int MaxOutcomeLength = 24;
    public const int MaxErrorCodeLength = 64;
    public const int MaxToolCallsLength = 512;
    public const int MaxFeedbackRatingLength = 8;
    public const int MaxFeedbackReasonLength = 16;

    // Audiences: whose data the request was about.
    public const string SelfAudience = "self";
    public const string MemberAudience = "member";
    public const string TeamAudience = "team";
    public const string ProjectAudience = "project";
    public const string WorkspaceAudience = "workspace";
    public const string HelpAudience = "help";

    public const string FeedbackUp = "up";
    public const string FeedbackDown = "down";

    public Guid WorkspaceId { get; private set; }

    /// <summary>Who asked.</summary>
    public Guid UserId { get; private set; }

    /// <summary>One of <see cref="AiFeatureIds"/>.</summary>
    public string FeatureId { get; private set; } = null!;

    public string PromptVersion { get; private set; } = null!;

    /// <summary><see cref="SelfAudience"/>, <see cref="MemberAudience"/>, <see cref="TeamAudience"/>,
    /// <see cref="ProjectAudience"/>, <see cref="WorkspaceAudience"/> or <see cref="HelpAudience"/>.</summary>
    public string Audience { get; private set; } = null!;

    /// <summary>The member the request was about, when it was about one.</summary>
    public Guid? SubjectUserId { get; private set; }

    public Guid? ProjectId { get; private set; }
    public DateTime? RangeFrom { get; private set; }
    public DateTime? RangeTo { get; private set; }

    /// <summary>The <see cref="AiDataCategories"/> that were sent, comma-separated.</summary>
    public string DataCategories { get; private set; } = string.Empty;

    /// <summary>The configured protocol (<c>AI:Provider</c>).</summary>
    public string Provider { get; private set; } = null!;

    public string? Model { get; private set; }

    /// <summary>One of <see cref="AiUsageOutcomes"/>.</summary>
    public string Outcome { get; private set; } = null!;

    /// <summary>The Tracker error code a failed request was answered with.</summary>
    public string? ErrorCode { get; private set; }

    public bool CacheHit { get; private set; }
    public bool IsRegenerate { get; private set; }

    public int? InputTokens { get; private set; }
    public int? OutputTokens { get; private set; }
    public int? CachedInputTokens { get; private set; }

    /// <summary>Already counted in <see cref="OutputTokens"/> (OpenAI reports them inside the output).</summary>
    public int? ReasoningTokens { get; private set; }

    /// <summary>Null when the model has no configured price.</summary>
    public decimal? EstimatedCostUsd { get; private set; }

    public int? LatencyMs { get; private set; }

    /// <summary>The Ask AI tools that were called, comma-separated (names only, never arguments).</summary>
    public string? ToolCalls { get; private set; }

    /// <summary><see cref="FeedbackUp"/> or <see cref="FeedbackDown"/>, once the caller rated the result.</summary>
    public string? FeedbackRating { get; private set; }

    public string? FeedbackReason { get; private set; }

    protected AiUsageRecord()
    {
    }

    public AiUsageRecord(
        Guid id,
        Guid workspaceId,
        Guid userId,
        string featureId,
        string promptVersion,
        string audience,
        string provider,
        string? model,
        string outcome,
        string? dataCategories = null,
        Guid? subjectUserId = null,
        Guid? projectId = null,
        DateTime? rangeFrom = null,
        DateTime? rangeTo = null,
        bool cacheHit = false,
        bool isRegenerate = false)
    {
        Id = id;
        WorkspaceId = workspaceId;
        UserId = userId;
        FeatureId = Check.NotNullOrWhiteSpace(featureId, nameof(featureId), MaxFeatureIdLength);
        PromptVersion = Check.NotNullOrWhiteSpace(promptVersion, nameof(promptVersion), MaxPromptVersionLength);
        Audience = Check.NotNullOrWhiteSpace(audience, nameof(audience), MaxAudienceLength);
        Provider = Check.NotNullOrWhiteSpace(provider, nameof(provider), MaxProviderLength);
        Model = Truncate(model, MaxModelLength);
        DataCategories = Truncate(dataCategories, MaxDataCategoriesLength) ?? string.Empty;
        SubjectUserId = subjectUserId;
        ProjectId = projectId;
        RangeFrom = rangeFrom;
        RangeTo = rangeTo;
        CacheHit = cacheHit;
        IsRegenerate = isRegenerate;
        SetOutcome(outcome);
    }

    /// <summary>Records what the provider reported; negative counts (never valid) are stored as unknown.</summary>
    public void SetUsage(
        int? inputTokens,
        int? outputTokens,
        int? cachedInputTokens = null,
        int? reasoningTokens = null,
        decimal? estimatedCostUsd = null,
        int? latencyMs = null,
        string? toolCalls = null)
    {
        InputTokens = NonNegative(inputTokens);
        OutputTokens = NonNegative(outputTokens);
        CachedInputTokens = NonNegative(cachedInputTokens);
        ReasoningTokens = NonNegative(reasoningTokens);
        EstimatedCostUsd = estimatedCostUsd is < 0m ? null : estimatedCostUsd;
        LatencyMs = NonNegative(latencyMs);
        ToolCalls = Truncate(toolCalls, MaxToolCallsLength);
    }

    /// <param name="outcome">One of <see cref="AiUsageOutcomes"/>.</param>
    /// <param name="errorCode">The Tracker error code a failed request was answered with.</param>
    public void SetOutcome(string outcome, string? errorCode = null)
    {
        if (!AiUsageOutcomes.IsKnown(outcome))
        {
            throw new ArgumentException($"'{outcome}' is not an AI usage outcome.", nameof(outcome));
        }

        Outcome = outcome;
        ErrorCode = Truncate(errorCode, MaxErrorCodeLength);
    }

    /// <param name="rating"><see cref="FeedbackUp"/> or <see cref="FeedbackDown"/>.</param>
    /// <param name="reason">A short reason code (inaccurate, not-useful, tone, other), or null.</param>
    public void SetFeedback(string rating, string? reason)
    {
        if (rating != FeedbackUp && rating != FeedbackDown)
        {
            throw new ArgumentException($"'{rating}' is not a feedback rating.", nameof(rating));
        }

        FeedbackRating = rating;
        FeedbackReason = Truncate(reason, MaxFeedbackReasonLength);
    }

    /// <summary>True when the request reached the provider: what the daily and monthly allowances count. Cache hits,
    /// no-data answers and clarifications made no call.</summary>
    public bool CalledProvider => AiUsageRecordQueries.CallsProvider(CacheHit, Outcome);

    private static int? NonNegative(int? value) => value is < 0 ? null : value;

    private static string? Truncate(string? value, int maxLength)
    {
        var trimmed = value?.Trim();
        if (string.IsNullOrEmpty(trimmed))
        {
            return null;
        }

        return trimmed.Length > maxLength ? trimmed[..maxLength] : trimmed;
    }
}

/// <summary>Ledger filters shared by the quotas, the usage views and the platform overview.</summary>
public static class AiUsageRecordQueries
{
    /// <summary>Only the rows that reached the provider (see <see cref="AiUsageRecord.CalledProvider"/>), as a query
    /// the database runs.</summary>
    public static IQueryable<AiUsageRecord> WhereCalledProvider(this IQueryable<AiUsageRecord> query)
    {
        return query.Where(r => !r.CacheHit &&
                                r.Outcome != AiUsageOutcomes.Cached &&
                                r.Outcome != AiUsageOutcomes.NoData &&
                                r.Outcome != AiUsageOutcomes.Clarification);
    }

    public static bool CallsProvider(bool cacheHit, string outcome) =>
        !cacheHit &&
        outcome != AiUsageOutcomes.Cached &&
        outcome != AiUsageOutcomes.NoData &&
        outcome != AiUsageOutcomes.Clarification;

    /// <summary>The first instant of the UTC month of <paramref name="now"/>.</summary>
    public static DateTime MonthStart(DateTime now) => new(now.Year, now.Month, 1, 0, 0, 0, DateTimeKind.Utc);

    /// <summary>The first instant of the UTC day of <paramref name="now"/>.</summary>
    public static DateTime DayStart(DateTime now) => new(now.Year, now.Month, now.Day, 0, 0, 0, DateTimeKind.Utc);
}
