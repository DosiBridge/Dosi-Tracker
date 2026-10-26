using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using Dosi.Tracker.Activities;

namespace Dosi.Tracker.AiAssist;

/// <summary>A period summary request (<c>POST api/app/ai-assist/period-summary</c>).</summary>
public class AiPeriodSummaryInput
{
    /// <summary>The start of the range (UTC, inclusive).</summary>
    [Required]
    public DateTime From { get; set; }

    /// <summary>The end of the range (UTC, exclusive); at most <see cref="AiAssistConsts.MaxPeriodDays"/> days after
    /// <see cref="From"/>.</summary>
    [Required]
    public DateTime To { get; set; }

    /// <summary>One project ("project pulse"): the project's time, with team scope; the caller's own time on it with self
    /// scope.</summary>
    public Guid? ProjectId { get; set; }

    /// <summary>"team" (owners and managers who see the team) or "self".</summary>
    [Required]
    [RegularExpression("^(team|self)$")]
    public string Scope { get; set; } = string.Empty;

    /// <summary>The IANA zone days are read in; the caller's reporting zone when null.</summary>
    [StringLength(ActivityConsts.MaxTimeZoneLength)]
    public string? TimeZone { get; set; }

    /// <summary>Ask for a new answer instead of a cached one (at most 3 times a day).</summary>
    public bool Regenerate { get; set; }
}

public class AiPeriodSummaryDto
{
    public AiResultMetaDto Meta { get; set; } = new();

    public string Headline { get; set; } = string.Empty;

    /// <summary>At most 5; kind is one of <see cref="AiInsightKinds.Period"/>.</summary>
    public List<AiInsightItemDto> Insights { get; set; } = new();

    /// <summary>At most 3.</summary>
    public List<AiNextStepDto> NextSteps { get; set; } = new();

    /// <summary>At most 3.</summary>
    public List<string> Caveats { get; set; } = new();
}

/// <summary>The kinds an insight item of each card may have.</summary>
public static class AiInsightKinds
{
    public static readonly IReadOnlyList<string> Period = new[] { "trend", "distribution", "coverage", "rhythm", "data-quality" };

    public static readonly IReadOnlyList<string> MemberDay = new[] { "focus", "apps", "projects", "breaks", "rhythm", "data-quality" };

    public static readonly IReadOnlyList<string> Report = new[]
    {
        "trend", "distribution", "coverage", "rhythm", "attendance", "apps", "devices", "data-quality"
    };
}
