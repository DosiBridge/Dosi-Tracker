using System;
using System.Linq;

namespace Dosi.Tracker.AiAssist;

/// <summary>What became of one AI request (<see cref="AiUsageRecord.Outcome"/>).</summary>
public static class AiUsageOutcomes
{
    /// <summary>The provider answered and the answer was valid.</summary>
    public const string Generated = "generated";

    /// <summary>Served from the result cache; no provider call.</summary>
    public const string Cached = "cached";

    /// <summary>Nothing to summarize; a fixed answer, no provider call.</summary>
    public const string NoData = "no-data";

    /// <summary>The provider failed or answered invalidly; the deterministic fallback was served.</summary>
    public const string Fallback = "fallback";

    public const string ProviderError = "provider-error";
    public const string Timeout = "timeout";

    /// <summary>The caller went away mid-request.</summary>
    public const string Cancelled = "cancelled";

    /// <summary>Ask AI asked the caller to clarify without calling the provider.</summary>
    public const string Clarification = "clarification";

    public static readonly string[] All =
    {
        Generated, Cached, NoData, Fallback, ProviderError, Timeout, Cancelled, Clarification
    };

    public static bool IsKnown(string? outcome) => outcome != null && All.Contains(outcome, StringComparer.Ordinal);
}
