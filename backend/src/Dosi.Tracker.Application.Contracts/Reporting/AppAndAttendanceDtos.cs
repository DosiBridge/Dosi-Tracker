using System;
using System.Collections.Generic;
using Volo.Abp.Timing;

namespace Dosi.Tracker.Reporting;

/// <summary>One row of the Apps &amp; URLs report: how much focused time an application received. Every spelling
/// of the same application (e.g. "chrome.exe" from Windows and "Google Chrome" from macOS) is one row.</summary>
public class AppUsageRowDto
{
    /// <summary>The name an agent reported for the app (its most-credited spelling in the range).</summary>
    public string AppName { get; set; } = string.Empty;

    /// <summary>The canonical display name the row is grouped by (shared contract §8).</summary>
    public string DisplayName { get; set; } = string.Empty;

    /// <summary>Minutes of tracked time whose focused (active) window was this app.</summary>
    public double TrackedMinutes { get; set; }

    /// <summary>Minutes of <see cref="TrackedMinutes"/> without keyboard/mouse input (never more than it).</summary>
    public double IdleMinutes { get; set; }

    /// <summary>Activities in which this app appeared (active or running).</summary>
    public int ActivityCount { get; set; }

    /// <summary>Distinct members who used this app.</summary>
    public int UserCount { get; set; }
}

/// <summary>Per-member attendance derived from tracked activity over the range.</summary>
public class AttendanceRowDto
{
    public Guid UserId { get; set; }

    /// <summary>The member's user name; empty when the account no longer exists.</summary>
    public string UserName { get; set; } = string.Empty;

    /// <summary>Weekdays of <see cref="AttendanceReportDto.Days"/> on which the member tracked any activity.</summary>
    public int DaysPresent { get; set; }

    /// <summary>Weekdays on which the member tracked nothing. Only days fully inside the range, not after today
    /// (in the report's time zone) and after the member joined count.</summary>
    public int DaysAbsent { get; set; }

    /// <summary>Saturdays and Sundays on which the member tracked any activity.</summary>
    public int WeekendDaysWorked { get; set; }

    /// <summary>Total tracked minutes across the range.</summary>
    public double WorkedMinutes { get; set; }

    /// <summary>Minutes of the worked time without keyboard/mouse input (blocks that measured it).</summary>
    public double IdleMinutes { get; set; }

    /// <summary>Minutes after local midnight of the day's first start, averaged over the present days; null
    /// when the member was never present.</summary>
    public double? AverageStartMinutes { get; set; }

    /// <summary>Minutes after local midnight of the day's last end, averaged over the present days (above 1440
    /// when work ran past midnight); null when the member was never present.</summary>
    public double? AverageEndMinutes { get; set; }

    /// <summary>Every local date (oldest first, weekends included) on which the member was present, for the
    /// calendar grid. Local dates at 00:00, serialized without a zone suffix.</summary>
    [DisableDateTimeNormalization]
    public List<DateTime> PresentDays { get; set; } = new();
}

/// <summary>
/// Attendance report. Presence and worked time are derived honestly from tracked activity; concepts
/// the tracker does not model yet (late arrival, on-site vs remote) are intentionally not invented.
/// The roster is every member of the reported projects (clients excluded), so absent members are listed too.
/// </summary>
public class AttendanceReportDto
{
    /// <summary>The weekdays (Monday to Friday) of the range in the report's time zone, oldest first. Local
    /// dates at 00:00, serialized without a zone suffix.</summary>
    [DisableDateTimeNormalization]
    public List<DateTime> Days { get; set; } = new();

    public List<AttendanceRowDto> Rows { get; set; } = new();
}
