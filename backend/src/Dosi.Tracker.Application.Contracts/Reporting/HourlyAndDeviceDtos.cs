using System;

namespace Dosi.Tracker.Reporting;

/// <summary>Tracked time in one clock hour of one weekday, summed over the range (the work-rhythm heatmap).</summary>
public class HourlyReportRowDto
{
    /// <summary>0 = Sunday … 6 = Saturday, in the report's time zone (or the member's clock with the "member" basis).</summary>
    public int DayOfWeek { get; set; }

    /// <summary>0..23, the local clock hour.</summary>
    public int Hour { get; set; }

    public double TrackedMinutes { get; set; }

    /// <summary>The measured active share of <see cref="TrackedMinutes"/>: each block's minutes in this hour
    /// times its activity %. Blocks that did not measure activity add tracked minutes only.</summary>
    public double ActiveMinutes { get; set; }
}

/// <summary>One device a member tracked from in the range (blocks of older agents, which report no device, share
/// one row per member with a null <see cref="DeviceId"/>).</summary>
public class DeviceReportRowDto
{
    public Guid UserId { get; set; }

    /// <summary>The member's user name; empty when the account no longer exists.</summary>
    public string UserName { get; set; } = string.Empty;

    public Guid? DeviceId { get; set; }

    // Taken from the device's latest block in the range, so an agent upgrade shows its newest version.
    public string? DeviceName { get; set; }
    public string? Platform { get; set; }
    public string? OsVersion { get; set; }
    public string? AgentVersion { get; set; }

    /// <summary>UTC instant the device's latest block in the range ended.</summary>
    public DateTime LastActivityAt { get; set; }

    public double TrackedMinutes { get; set; }
}
