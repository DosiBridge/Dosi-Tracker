using System;
using System.Collections.Generic;

namespace Dosi.Tracker.Platform;

/// <summary>AI assist across the platform this UTC month. Only requests that reached the provider count as requests.</summary>
public class PlatformAiOverviewDto
{
    /// <summary>"configured", "disabled", "missing-api-key", "missing-model", "invalid-endpoint" or "unsupported-provider".</summary>
    public string ProviderState { get; set; } = string.Empty;

    /// <summary>Null unless configured.</summary>
    public string? ProviderDisplayName { get; set; }

    /// <summary>Null unless configured.</summary>
    public string? DefaultModel { get; set; }

    /// <summary><c>AI:Enabled</c> (needs a restart to change).</summary>
    public bool ConfigEnabled { get; set; }

    /// <summary>The runtime switch (<see cref="IPlatformAiAppService.UpdateStatusAsync"/>).</summary>
    public bool RuntimeEnabled { get; set; }

    /// <summary>"closed", "open" or "half-open".</summary>
    public string BreakerState { get; set; } = "closed";

    public DateTime MonthStart { get; set; }
    public long Requests { get; set; }
    public long InputTokens { get; set; }
    public long OutputTokens { get; set; }

    /// <summary>Null when no request had a configured price.</summary>
    public decimal? EstimatedCostUsd { get; set; }

    /// <summary><c>AI:Budget:MonthlyTokenLimit</c>; null: none.</summary>
    public long? BudgetTokens { get; set; }

    /// <summary><c>AI:Budget:MonthlyUsdLimit</c>; null: none.</summary>
    public decimal? BudgetUsd { get; set; }

    /// <summary>Share of requests answered with the deterministic fallback (0 to 1).</summary>
    public double FallbackRate { get; set; }

    /// <summary>Share of requests that failed at the provider or timed out (0 to 1).</summary>
    public double ErrorRate { get; set; }

    /// <summary>The ten workspaces that used the most tokens.</summary>
    public List<PlatformAiWorkspaceUsageDto> TopWorkspaces { get; set; } = new();
}

public class PlatformAiWorkspaceUsageDto
{
    public Guid WorkspaceId { get; set; }
    public string WorkspaceName { get; set; } = string.Empty;
    public long Requests { get; set; }
    public long Tokens { get; set; }
    public decimal? EstimatedCostUsd { get; set; }
}

/// <summary>The platform's runtime switch of AI assist.</summary>
public class PlatformAiStatusDto
{
    public bool Enabled { get; set; }
}
