using System;
using System.Collections.Generic;

namespace Dosi.Tracker.AiAssist;

/// <summary>What every AI result carries besides its content.</summary>
public class AiResultMetaDto
{
    /// <summary>The usage-ledger row of this result (what feedback refers to).</summary>
    public Guid ResultId { get; set; }

    /// <summary>One of <see cref="AiFeatureIds"/>.</summary>
    public string FeatureId { get; set; } = string.Empty;

    /// <summary>"generated", "cached", "fallback" or "no-data".</summary>
    public string Status { get; set; } = string.Empty;

    /// <summary>UTC.</summary>
    public DateTime GeneratedAt { get; set; }

    /// <summary>Null for an answer the server wrote itself (no-data, fallback).</summary>
    public string? Model { get; set; }

    public string PromptVersion { get; set; } = string.Empty;

    /// <summary>The <see cref="AiDataCategories"/> that were sent to the provider.</summary>
    public List<string> DataCategories { get; set; } = new();

    /// <summary>The server-localized label every AI result is shown with.</summary>
    public string Disclaimer { get; set; } = string.Empty;
}

/// <summary>A figure a result used, rendered with real names, for "Show the figures used".</summary>
public class AiFactRefDto
{
    public string Id { get; set; } = string.Empty;
    public string Label { get; set; } = string.Empty;
    public string Display { get; set; } = string.Empty;
}

public class AiInsightItemDto
{
    public string Kind { get; set; } = string.Empty;
    public string Text { get; set; } = string.Empty;
    public List<AiFactRefDto> Facts { get; set; } = new();

    /// <summary>An allowlisted route id the web app maps to a page, or null.</summary>
    public string? RouteId { get; set; }
}

public class AiNextStepDto
{
    public string Label { get; set; } = string.Empty;
    public string RouteId { get; set; } = string.Empty;
}

/// <summary>Whether the caller may use one feature (<see cref="AiStatusDto.Features"/>).</summary>
public class AiFeatureStatusDto
{
    /// <summary>One of <see cref="AiFeatureIds"/>.</summary>
    public string FeatureId { get; set; } = string.Empty;

    public bool Allowed { get; set; }

    /// <summary>Why this feature is not available to the caller (<see cref="AiFeatureReasons"/>); null when nothing
    /// about the feature itself stands in the way (the status' own reason may still).</summary>
    public string? Reason { get; set; }

    /// <summary>What the caller may ask it about: "team", "member", "self" (see <see cref="AiFeatureScopes"/>); empty for
    /// features without scopes.</summary>
    public List<string> Scopes { get; set; } = new();
}

/// <summary>An allowance and how much of it is used.</summary>
public class AiQuotaDto
{
    public int RequestsUsed { get; set; }

    /// <summary>Null: unlimited.</summary>
    public int? RequestsLimit { get; set; }

    public long TokensUsed { get; set; }

    /// <summary>Null: unlimited.</summary>
    public long? TokensLimit { get; set; }

    /// <summary>When the allowance renews (UTC).</summary>
    public DateTime ResetsAt { get; set; }
}

/// <summary>Why a feature is not available to the caller (<see cref="AiFeatureStatusDto.Reason"/>).</summary>
public static class AiFeatureReasons
{
    /// <summary>Turned off on this server (<c>AI:Features:{Key}:Enabled</c>, or Ask AI without tool support).</summary>
    public const string Operator = "operator";

    /// <summary>Not for the caller's role (clients; managers the owner did not allow; members for team-only features).</summary>
    public const string Role = "role";

    /// <summary>The owner turned off AI on members' own data.</summary>
    public const string SelfServiceOff = "self-service-off";

    /// <summary>The caller lacks a permission the feature needs.</summary>
    public const string Permission = "permission";
}

/// <summary>What a feature may be asked about (<see cref="AiFeatureStatusDto.Scopes"/>).</summary>
public static class AiFeatureScopes
{
    public const string Team = "team";
    public const string Member = "member";
    public const string Self = "self";
}
