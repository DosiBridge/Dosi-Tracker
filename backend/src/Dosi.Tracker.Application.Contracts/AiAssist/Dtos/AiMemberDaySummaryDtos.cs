using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using Dosi.Tracker.Activities;

namespace Dosi.Tracker.AiAssist;

/// <summary>A day summary request (<c>POST api/app/ai-assist/member-day-summary</c>).</summary>
public class AiMemberDaySummaryInput
{
    /// <summary>The member; the caller when null. Another member needs a caller who sees the team.</summary>
    public Guid? UserId { get; set; }

    /// <summary>The local day, yyyy-MM-dd, in <see cref="TimeZone"/>.</summary>
    [Required]
    [RegularExpression(@"^\d{4}-\d{2}-\d{2}$")]
    public string Date { get; set; } = string.Empty;

    /// <summary>The IANA zone the day is read in; the caller's reporting zone when null.</summary>
    [StringLength(ActivityConsts.MaxTimeZoneLength)]
    public string? TimeZone { get; set; }

    public bool Regenerate { get; set; }
}

public class AiMemberDaySummaryDto
{
    public AiResultMetaDto Meta { get; set; } = new();

    public string Headline { get; set; } = string.Empty;

    /// <summary>Two to four sentences.</summary>
    public string Summary { get; set; } = string.Empty;

    /// <summary>At most 5; kind is one of <see cref="AiInsightKinds.MemberDay"/>.</summary>
    public List<AiInsightItemDto> Highlights { get; set; } = new();

    /// <summary>At most 3.</summary>
    public List<string> Caveats { get; set; } = new();
}
