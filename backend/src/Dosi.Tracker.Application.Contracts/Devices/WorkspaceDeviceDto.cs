using System;

namespace Dosi.Tracker.Devices;

/// <summary>
/// A desktop agent install of a member in the current workspace: one that reported a heartbeat (its descriptors
/// are the latest heartbeat's), or one that is revoked. Members see their own devices; owners and managers
/// (<c>Tracker.Devices.Manage</c>) see everyone's.
/// </summary>
public class WorkspaceDeviceDto
{
    public Guid UserId { get; set; }
    public string? UserName { get; set; }

    /// <summary>The member's full name, when the account has one.</summary>
    public string? Name { get; set; }

    public Guid DeviceId { get; set; }
    public string? DeviceName { get; set; }
    public string? Platform { get; set; }
    public string? OsVersion { get; set; }
    public string? AgentVersion { get; set; }

    /// <summary>Server time of the device's latest heartbeat; null when presence no longer lists it (only its
    /// revocation remains).</summary>
    public DateTime? LastSeenAt { get; set; }

    public bool IsRevoked { get; set; }
    public DateTime? RevokedAt { get; set; }
    public Guid? RevokedByUserId { get; set; }
    public string? Reason { get; set; }
}
