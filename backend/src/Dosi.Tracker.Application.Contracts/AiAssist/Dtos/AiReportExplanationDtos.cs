using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using Dosi.Tracker.Activities;

namespace Dosi.Tracker.AiAssist;

/// <summary>A report explanation request (<c>POST api/app/ai-assist/report-explanation</c>): the report a page shows,
/// with the same filters.</summary>
public class AiReportExplanationInput
{
    /// <summary>One of the web app's report slugs, payroll excepted.</summary>
    [Required]
    [StringLength(32)]
    public string ReportSlug { get; set; } = string.Empty;

    /// <summary>UTC, inclusive.</summary>
    [Required]
    public DateTime From { get; set; }

    /// <summary>UTC, exclusive; at most <see cref="AiAssistConsts.MaxReportDays"/> days after <see cref="From"/>.</summary>
    [Required]
    public DateTime To { get; set; }

    public Guid? ProjectId { get; set; }

    public Guid? UserId { get; set; }

    [StringLength(ActivityConsts.MaxTimeZoneLength)]
    public string? TimeZone { get; set; }

    /// <summary>"zone" (default) or "member", as the report uses.</summary>
    [RegularExpression("^(zone|member)$")]
    public string? DayBasis { get; set; }

    public bool Regenerate { get; set; }
}

public class AiReportExplanationDto
{
    public AiResultMetaDto Meta { get; set; } = new();

    /// <summary>Two to four sentences.</summary>
    public string Summary { get; set; } = string.Empty;

    /// <summary>At most 6; kind is one of <see cref="AiInsightKinds.Report"/>.</summary>
    public List<AiInsightItemDto> KeyFindings { get; set; } = new();

    /// <summary>At most 4.</summary>
    public List<string> Caveats { get; set; } = new();

    /// <summary>At most 3 neutral questions.</summary>
    public List<string> QuestionsToExplore { get; set; } = new();
}
