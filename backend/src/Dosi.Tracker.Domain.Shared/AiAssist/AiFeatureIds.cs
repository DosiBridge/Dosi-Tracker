using System;
using System.Collections.Generic;
using System.Linq;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// One AI assist feature: its <paramref name="Id"/> on the wire (routes, DTOs, the usage ledger), its
/// <paramref name="ConfigKey"/> under <c>AI:Features</c> (PascalCase, so it can be set from an environment variable such
/// as <c>AI__Features__MemberDaySummary__Enabled</c>) and the output-token cap used when none is configured.
/// </summary>
public sealed record AiFeature(string Id, string ConfigKey, int DefaultMaxOutputTokens);

/// <summary>The AI assist features. Nothing outside this list is ever served or metered.</summary>
public static class AiFeatureIds
{
    public const string PeriodSummaryId = "period-summary";
    public const string MemberDaySummaryId = "member-day-summary";
    public const string ReportExplanationId = "report-explanation";
    public const string DataQualityReviewId = "data-quality-review";
    public const string NoteSuggestionsId = "note-suggestions";
    public const string TaskDraftsId = "task-drafts";
    public const string AskId = "ask";

    public static readonly AiFeature PeriodSummary = new(PeriodSummaryId, "PeriodSummary", 700);
    public static readonly AiFeature MemberDaySummary = new(MemberDaySummaryId, "MemberDaySummary", 600);
    public static readonly AiFeature ReportExplanation = new(ReportExplanationId, "ReportExplanation", 800);
    public static readonly AiFeature DataQualityReview = new(DataQualityReviewId, "DataQualityReview", 600);
    public static readonly AiFeature NoteSuggestions = new(NoteSuggestionsId, "NoteSuggestions", 200);
    public static readonly AiFeature TaskDrafts = new(TaskDraftsId, "TaskDrafts", 1200);
    public static readonly AiFeature Ask = new(AskId, "Ask", 700);

    public static readonly IReadOnlyList<AiFeature> All = new[]
    {
        PeriodSummary, MemberDaySummary, ReportExplanation, DataQualityReview, NoteSuggestions, TaskDrafts, Ask
    };

    /// <summary>True for a feature id in <see cref="All"/> (exact, case-sensitive: ids are wire values).</summary>
    public static bool IsKnown(string? featureId) => Find(featureId) != null;

    /// <summary>The feature with this id, or null.</summary>
    public static AiFeature? Find(string? featureId) =>
        featureId == null ? null : All.FirstOrDefault(f => string.Equals(f.Id, featureId, StringComparison.Ordinal));
}
