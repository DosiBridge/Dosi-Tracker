using System;
using Dosi.Tracker.Workspaces;
using Volo.Abp.Domain.Entities.Auditing;

namespace Dosi.Tracker.Devices;

/// <summary>
/// A persistent record that an observed logical device id has been revoked for a user in a workspace.
/// Keyed by (WorkspaceId, UserId, DeviceId) so presence pruning or recreation cannot erase it.
/// </summary>
public class DeviceRevocation : CreationAuditedEntity<Guid>, IWorkspaceScoped
{
    public Guid WorkspaceId { get; set; }
    public Guid UserId { get; set; }
    public Guid DeviceId { get; set; }
    public DateTime RevokedAt { get; set; }
    public string? Reason { get; set; }

    protected DeviceRevocation()
    {
    }

    public DeviceRevocation(Guid id, Guid workspaceId, Guid userId, Guid deviceId, DateTime revokedAt, string? reason = null)
        : base(id)
    {
        if (deviceId == Guid.Empty)
        {
            throw new ArgumentException("DeviceId cannot be empty.", nameof(deviceId));
        }

        WorkspaceId = workspaceId;
        UserId = userId;
        DeviceId = deviceId;
        RevokedAt = revokedAt;
        Reason = reason;
    }
}
