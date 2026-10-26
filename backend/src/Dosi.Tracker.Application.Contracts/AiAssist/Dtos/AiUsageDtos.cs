using System;
using System.Collections.Generic;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// AI usage this UTC month (<c>GET /api/app/ai-assist/usage</c>). Owners see the workspace's totals against its monthly
/// allowance, with the estimated cost and a per-feature breakdown; everyone else sees their own requests and tokens
/// (the limits are then null: the workspace's allowance is the owners' to watch). Everyone sees their own requests
/// today against their daily limit. Only requests that reached the provider count.
/// </summary>
public class AiUsageSummaryDto
{
    public DateTime MonthStart { get; set; }
    public DateTime ResetsAt { get; set; }

    public int RequestsUsed { get; set; }

    /// <summary>Null: unlimited, or not shown to the caller.</summary>
    public int? RequestsLimit { get; set; }

    public long TokensUsed { get; set; }

    /// <summary>Null: unlimited, or not shown to the caller.</summary>
    public long? TokensLimit { get; set; }

    /// <summary>Owners only; null when no request had a configured price.</summary>
    public decimal? EstimatedCostUsd { get; set; }

    /// <summary>Owners only.</summary>
    public List<AiFeatureUsageDto> ByFeature { get; set; } = new();

    public int MyRequestsToday { get; set; }

    /// <summary>Null: unlimited.</summary>
    public int? MyDailyLimit { get; set; }
}

public class AiFeatureUsageDto
{
    public string FeatureId { get; set; } = string.Empty;
    public int Requests { get; set; }
    public long Tokens { get; set; }
}
