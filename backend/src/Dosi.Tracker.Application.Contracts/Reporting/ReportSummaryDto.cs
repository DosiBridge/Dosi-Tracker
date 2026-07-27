using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.Reporting;

public class GetReportSummaryInput
{
    [Required]
    public DateTime From { get; set; }

    [Required]
    public DateTime To { get; set; }

    public Guid? ProjectId { get; set; }
}

public class ReportSummaryDto
{
    public int TotalActivities { get; set; }

    /// <summary>Sum of real block durations (EndedAt - StartedAt), in minutes —
    /// not the block-count × interval approximation the mock UI used.</summary>
    public double TotalTrackedMinutes { get; set; }

    /// <summary>Duration-weighted average productivity (0-100).</summary>
    public double AverageProductivity { get; set; }

    public long TotalMouseClicks { get; set; }
    public long TotalKeyboardHits { get; set; }

    public List<UserReportRowDto> PerUser { get; set; } = new();
    public List<ProjectReportRowDto> PerProject { get; set; } = new();
}

public class UserReportRowDto
{
    public Guid UserId { get; set; }
    public int ActivityCount { get; set; }
    public double TrackedMinutes { get; set; }
    public double AverageProductivity { get; set; }
}

public class ProjectReportRowDto
{
    public Guid ProjectId { get; set; }
    public int ActivityCount { get; set; }
    public double TrackedMinutes { get; set; }
    public double AverageProductivity { get; set; }
}

/// <summary>One UTC calendar day of tracked work (for time-series charts).</summary>
public class DailyReportRowDto
{
    public DateTime Date { get; set; }
    public int ActivityCount { get; set; }
    public double TrackedMinutes { get; set; }
    public double AverageProductivity { get; set; }
    public long MouseClicks { get; set; }
    public long KeyboardHits { get; set; }
}
