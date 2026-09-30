using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.Reporting;

/// <summary>Report range [From, To) in UTC. To must be after From and the range may span at most
/// <see cref="MaxRangeDays"/> days (otherwise Tracker:ReportRangeInvalid).</summary>
public class GetReportSummaryInput
{
    public const int MaxRangeDays = 366;

    [Required]
    public DateTime From { get; set; }

    [Required]
    public DateTime To { get; set; }

    public Guid? ProjectId { get; set; }

    /// <summary>Report on a single member. Only honored for callers with Tracker.Activities.ViewAll;
    /// everyone else is always scoped to their own data regardless of this value.</summary>
    public Guid? UserId { get; set; }
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

/// <summary>One member's tracked work on one UTC calendar day (only days with activity are returned).</summary>
public class UserDailyReportRowDto
{
    public Guid UserId { get; set; }
    public string UserName { get; set; } = string.Empty;

    /// <summary>The UTC day (00:00, Kind=Utc).</summary>
    public DateTime Date { get; set; }

    public double TrackedMinutes { get; set; }
    public int ActivityCount { get; set; }
}
