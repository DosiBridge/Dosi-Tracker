using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using Volo.Abp.Timing;

namespace Dosi.Tracker.Reporting;

/// <summary>Report range [From, To) in UTC. To must be after From and the range may span at most
/// <see cref="MaxRangeDays"/> days (otherwise Tracker:ReportRangeInvalid). A block is in the range when it
/// started in it (From &lt;= StartedAt &lt; To), and it is always reported whole.</summary>
public class GetReportSummaryInput
{
    public const int MaxRangeDays = 366;

    [Required]
    public DateTime From { get; set; }

    [Required]
    public DateTime To { get; set; }

    public Guid? ProjectId { get; set; }

    /// <summary>Report on a single member. Honored for callers who see the whole team, and for clients (within
    /// the projects they are a client of); a member who sees only their own data always gets their own data.</summary>
    public Guid? UserId { get; set; }

    /// <summary>IANA time zone (e.g. "Asia/Dhaka") whose calendar days and clock hours every day- and hour-based
    /// report uses. Defaults to "UTC"; an unknown id falls back to UTC rather than failing the request.</summary>
    public string? TimeZone { get; set; }

    /// <summary>Which clock assigns a block to its day (and hour): <see cref="ReportDayBasis.Zone"/> (the default)
    /// or <see cref="ReportDayBasis.Member"/>. Any other value reads as "zone".</summary>
    public string? DayBasis { get; set; }
}

/// <summary>Values of <see cref="GetReportSummaryInput.DayBasis"/> (compared case-insensitively).</summary>
public static class ReportDayBasis
{
    /// <summary>Every block is placed on the local day of its start in the requested time zone.</summary>
    public const string Zone = "zone";

    /// <summary>Every block is placed on the local day of its start on the clock of the device that tracked it
    /// (its UTC offset, else its time zone), falling back to the requested zone for blocks that carry neither.</summary>
    public const string Member = "member";
}

public class ReportSummaryDto
{
    public int TotalActivities { get; set; }

    /// <summary>Sum of real block durations (EndedAt - StartedAt), in minutes —
    /// not the block-count × interval approximation the mock UI used.</summary>
    public double TotalTrackedMinutes { get; set; }

    /// <summary>Minutes of the tracked time without keyboard/mouse input. Only blocks whose agent measured idle
    /// time contribute (older blocks count as 0).</summary>
    public double TotalIdleMinutes { get; set; }

    /// <summary>Duration-weighted average activity % (0-100) of the blocks that measured it; null when no block
    /// in the range was measured. (A Windows agent from before shared contract v2 — it sends no TimeZone — reads this
    /// as a plain number, so the API host answers it 0 instead of null.)</summary>
    public double? AverageProductivity { get; set; }

    public long TotalMouseClicks { get; set; }
    public long TotalKeyboardHits { get; set; }

    public List<UserReportRowDto> PerUser { get; set; } = new();
    public List<ProjectReportRowDto> PerProject { get; set; } = new();

    /// <summary>One row per member and project with tracked time (ordered by member, then project).</summary>
    public List<UserProjectReportRowDto> PerUserProject { get; set; } = new();
}

public class UserReportRowDto
{
    public Guid UserId { get; set; }

    /// <summary>The member's user name; empty when the account no longer exists.</summary>
    public string UserName { get; set; } = string.Empty;

    /// <summary>The member's full name (name and surname); null when neither is set.</summary>
    public string? Name { get; set; }

    public int ActivityCount { get; set; }
    public double TrackedMinutes { get; set; }
    public double IdleMinutes { get; set; }

    /// <summary>Null when none of the member's blocks measured activity.</summary>
    public double? AverageProductivity { get; set; }
}

public class ProjectReportRowDto
{
    public Guid ProjectId { get; set; }

    /// <summary>The project's title; empty when the project no longer exists.</summary>
    public string ProjectTitle { get; set; } = string.Empty;

    /// <summary>True when the project is archived (or was deleted): its time still counts in the range.</summary>
    public bool IsArchived { get; set; }

    public int ActivityCount { get; set; }
    public double TrackedMinutes { get; set; }
    public double IdleMinutes { get; set; }

    /// <summary>Null when none of the project's blocks measured activity.</summary>
    public double? AverageProductivity { get; set; }
}

/// <summary>One member's time on one project over the range, priced with the member's effective-dated rates.</summary>
public class UserProjectReportRowDto
{
    public Guid UserId { get; set; }
    public string UserName { get; set; } = string.Empty;
    public Guid ProjectId { get; set; }
    public string ProjectTitle { get; set; } = string.Empty;

    public int ActivityCount { get; set; }
    public double TrackedMinutes { get; set; }
    public double IdleMinutes { get; set; }
    public double? AverageProductivity { get; set; }

    /// <summary>Σ(block minutes × the hourly rate in effect at the block's start) / 60, rounded to 2 decimals.
    /// Only disclosed to callers with Tracker.Team.Manage (pay rates are theirs to see); 0 for everyone else.</summary>
    public decimal Amount { get; set; }

    /// <summary>The effective hourly rate over the range: the rates in effect, weighted by the minutes priced at
    /// each (rounded to 2 decimals). Same disclosure rule as <see cref="Amount"/>.</summary>
    public decimal HourlyRate { get; set; }
}

/// <summary>GET /api/app/reporting/payroll: tracked time and pay per member and project.</summary>
public class PayrollReportDto
{
    /// <summary>Ordered by member, then project.</summary>
    public List<UserProjectReportRowDto> Rows { get; set; } = new();

    public double TotalMinutes { get; set; }

    /// <summary>The sum of the rows' (already rounded) amounts, so the total always matches the rows.</summary>
    public decimal TotalAmount { get; set; }
}

/// <summary>One calendar day of the requested time zone (for time-series charts).</summary>
public class DailyReportRowDto
{
    /// <summary>The local date at 00:00 (Kind Unspecified, serialized without a zone suffix).</summary>
    [DisableDateTimeNormalization]
    public DateTime Date { get; set; }

    public int ActivityCount { get; set; }
    public double TrackedMinutes { get; set; }
    public double IdleMinutes { get; set; }

    /// <summary>Null when nothing tracked that day measured activity.</summary>
    public double? AverageProductivity { get; set; }

    public long MouseClicks { get; set; }
    public long KeyboardHits { get; set; }
}

/// <summary>One member's tracked work on one local day (only days with activity are returned).</summary>
public class UserDailyReportRowDto
{
    public Guid UserId { get; set; }
    public string UserName { get; set; } = string.Empty;

    /// <summary>The local date at 00:00 (Kind Unspecified, serialized without a zone suffix).</summary>
    [DisableDateTimeNormalization]
    public DateTime Date { get; set; }

    public double TrackedMinutes { get; set; }
    public int ActivityCount { get; set; }
    public double IdleMinutes { get; set; }

    /// <summary>UTC instant the day's first block started.</summary>
    public DateTime FirstStartedAt { get; set; }

    /// <summary>UTC instant the day's last block ended.</summary>
    public DateTime LastEndedAt { get; set; }
}
