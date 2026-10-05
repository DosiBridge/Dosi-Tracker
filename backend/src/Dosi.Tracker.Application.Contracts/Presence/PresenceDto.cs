using System;

namespace Dosi.Tracker.Presence;

/// <summary>The live state of one member's desktop agent install ("who is working now").</summary>
public class PresenceDto
{
    public Guid UserId { get; set; }

    public string UserName { get; set; } = string.Empty;

    /// <summary>The member's full name ("Name Surname"), when set.</summary>
    public string? Name { get; set; }

    public Guid DeviceId { get; set; }

    public string? DeviceName { get; set; }

    /// <summary>"windows" or "macos".</summary>
    public string? Platform { get; set; }

    public string? OsVersion { get; set; }

    public string? AgentVersion { get; set; }

    /// <summary>The last reported status: one of <see cref="AgentPresenceConsts.Statuses"/>.</summary>
    public string Status { get; set; } = string.Empty;

    public Guid? ProjectId { get; set; }

    public string? ProjectTitle { get; set; }

    /// <summary>When the agent entered <see cref="Status"/>, on the server's clock.</summary>
    public DateTime? StatusSince { get; set; }

    /// <summary>Server time of the latest heartbeat.</summary>
    public DateTime LastSeenAt { get; set; }

    /// <summary>True while the latest heartbeat is at most <see cref="AgentPresenceConsts.OnlineWindowSeconds"/> old.</summary>
    public bool Online { get; set; }

    /// <summary>The foreground application, only when the project allows the active window.</summary>
    public string? CurrentApp { get; set; }

    public string? TimeZone { get; set; }

    public int? UtcOffsetMinutes { get; set; }
}
