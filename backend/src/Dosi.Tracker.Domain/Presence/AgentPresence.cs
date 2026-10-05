using System;
using Volo.Abp.Domain.Entities.Auditing;
using Volo.Abp.MultiTenancy;

namespace Dosi.Tracker.Presence;

/// <summary>
/// The live state of one desktop agent install (a user on a device), refreshed by the agent's heartbeats.
/// There is at most one row per (tenant, user, device); <see cref="LastSeenAt"/> tells whether it is online.
/// </summary>
public class AgentPresence : CreationAuditedEntity<Guid>, IMultiTenant
{
    public Guid? TenantId { get; set; }
    public Guid UserId { get; set; }

    /// <summary>Random id generated once per agent install.</summary>
    public Guid DeviceId { get; set; }

    public string? DeviceName { get; set; }
    public string? Platform { get; set; } // "windows" | "macos"
    public string? OsVersion { get; set; }
    public string? AgentVersion { get; set; }

    /// <summary>One of <c>AgentPresenceConsts.Statuses</c> ("tracking", "paused", "idle", …).</summary>
    public string Status { get; set; }

    /// <summary>The project being tracked (or last chosen), if any.</summary>
    public Guid? ProjectId { get; set; }

    /// <summary>When the agent entered <see cref="Status"/>, by the agent's account.</summary>
    public DateTime? StatusSince { get; set; }

    /// <summary>Server time of the latest heartbeat.</summary>
    public DateTime LastSeenAt { get; set; }

    /// <summary>The foreground application; only kept when the project allows the active window.</summary>
    public string? CurrentApp { get; set; }

    public string? TimeZone { get; set; } // IANA id of the device, e.g. "Asia/Dhaka"
    public int? UtcOffsetMinutes { get; set; }

    protected AgentPresence()
    {
    }

    public AgentPresence(Guid id, Guid? tenantId, Guid userId, Guid deviceId, string status, DateTime lastSeenAt)
    {
        Id = id;
        TenantId = tenantId;
        UserId = userId;
        DeviceId = deviceId;
        Status = status;
        LastSeenAt = lastSeenAt;
    }
}
