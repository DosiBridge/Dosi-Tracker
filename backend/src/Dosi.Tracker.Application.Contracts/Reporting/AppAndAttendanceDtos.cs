using System;
using System.Collections.Generic;

namespace Dosi.Tracker.Reporting;

/// <summary>One row of the Apps &amp; URLs report: how much focused time an application received.</summary>
public class AppUsageRowDto
{
    public string AppName { get; set; } = string.Empty;

    /// <summary>Minutes of tracked time whose focused (active) window was this app.</summary>
    public double TrackedMinutes { get; set; }

    /// <summary>Activities in which this app appeared (active or running).</summary>
    public int ActivityCount { get; set; }

    /// <summary>Distinct members who used this app.</summary>
    public int UserCount { get; set; }
}

/// <summary>Per-member attendance derived from tracked activity over the range.</summary>
public class AttendanceRowDto
{
    public Guid UserId { get; set; }

    /// <summary>Distinct UTC days on which the member tracked any activity.</summary>
    public int DaysPresent { get; set; }

    /// <summary>Weekdays in the range on which the member tracked nothing.</summary>
    public int DaysAbsent { get; set; }

    /// <summary>Total tracked minutes across the range.</summary>
    public double WorkedMinutes { get; set; }

    /// <summary>The distinct UTC days (oldest first) on which the member was present, for the calendar grid.</summary>
    public List<DateTime> PresentDays { get; set; } = new();
}

/// <summary>
/// Attendance report. Presence and worked time are derived honestly from tracked activity; concepts
/// the tracker does not model yet (late arrival, on-site vs remote) are intentionally not invented.
/// </summary>
public class AttendanceReportDto
{
    /// <summary>Weekdays covered by the range (UTC), oldest first.</summary>
    public List<DateTime> Days { get; set; } = new();

    public List<AttendanceRowDto> Rows { get; set; } = new();
}
