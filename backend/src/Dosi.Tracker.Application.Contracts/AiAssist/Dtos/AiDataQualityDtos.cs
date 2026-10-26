using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using Dosi.Tracker.Activities;

namespace Dosi.Tracker.AiAssist;

/// <summary>A data-quality review request (<c>POST api/app/ai-assist/data-quality-review</c>): the timesheet's "Week
/// review" (team or self) and the team page's "Agent health".</summary>
public class AiDataQualityReviewInput
{
    [Required]
    public DateTime From { get; set; }

    /// <summary>UTC, exclusive; at most <see cref="AiAssistConsts.MaxPeriodDays"/> days after <see cref="From"/>.</summary>
    [Required]
    public DateTime To { get; set; }

    /// <summary>"team" (owners and managers who see the team) or "self".</summary>
    [Required]
    [RegularExpression("^(team|self)$")]
    public string Scope { get; set; } = string.Empty;

    public Guid? ProjectId { get; set; }

    [StringLength(ActivityConsts.MaxTimeZoneLength)]
    public string? TimeZone { get; set; }

    /// <summary>The kinds to look for (<see cref="AiDataQualityKinds"/>); every kind the scope allows when null.
    /// never-tracked and outdated-agent are for team scope only.</summary>
    [MaxLength(6)]
    public List<string>? Kinds { get; set; }

    public bool Regenerate { get; set; }
}

public class AiDataQualityReviewDto
{
    public AiResultMetaDto Meta { get; set; } = new();

    /// <summary>At most 5, chosen from the candidates the server's rules found.</summary>
    public List<AiDataQualityItemDto> Items { get; set; } = new();

    /// <summary>How many candidates the rules found (at most 20 are considered).</summary>
    public int CandidateCount { get; set; }
}

public class AiDataQualityItemDto
{
    public string CandidateId { get; set; } = string.Empty;

    /// <summary>One of <see cref="AiDataQualityKinds"/>.</summary>
    public string Kind { get; set; } = string.Empty;

    /// <summary>"info" or "notice".</summary>
    public string Severity { get; set; } = string.Empty;

    public string Title { get; set; } = string.Empty;

    public string Explanation { get; set; } = string.Empty;

    /// <summary>At most 3, always benign.</summary>
    public List<string> PossibleReasons { get; set; } = new();

    /// <summary>One of the candidate's allowed actions, or null.</summary>
    public string? ActionId { get; set; }

    /// <summary>The page the action opens (an allowlisted route id), or null.</summary>
    public string? ActionRouteId { get; set; }

    /// <summary>The member's name, or "you".</summary>
    public string? SubjectName { get; set; }

    public Guid? SubjectUserId { get; set; }

    public List<AiFactRefDto> Facts { get; set; } = new();
}

/// <summary>What a data-quality review looks for.</summary>
public static class AiDataQualityKinds
{
    public const string DataGap = "data-gap";
    public const string ClockSkew = "clock-skew";
    public const string UnmeasuredActivity = "unmeasured-activity";
    public const string SoftwareInput = "software-input";

    /// <summary>Team scope only.</summary>
    public const string NeverTracked = "never-tracked";

    /// <summary>Team scope only.</summary>
    public const string OutdatedAgent = "outdated-agent";

    /// <summary>Every kind, in the order candidates are listed.</summary>
    public static readonly IReadOnlyList<string> All = new[]
    {
        DataGap, ClockSkew, UnmeasuredActivity, SoftwareInput, NeverTracked, OutdatedAgent
    };

    /// <summary>The kinds a self-scoped review may look for.</summary>
    public static readonly IReadOnlyList<string> Self = new[] { DataGap, ClockSkew, UnmeasuredActivity, SoftwareInput };
}

/// <summary>How much a data-quality item asks for attention (never a judgement of a person).</summary>
public static class AiDataQualitySeverities
{
    public const string Info = "info";
    public const string Notice = "notice";
}
