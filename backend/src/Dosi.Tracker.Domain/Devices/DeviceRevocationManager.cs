using System;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Presence;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Domain.Services;

namespace Dosi.Tracker.Devices;

/// <summary>
/// Domain service coordinating verification of observed status, idempotency, and revocation persistence.
/// </summary>
public class DeviceRevocationManager : DomainService
{
    private readonly IRepository<DeviceRevocation, Guid> _revocationRepository;
    private readonly IRepository<AgentPresence, Guid> _presenceRepository;
    private readonly IRepository<Activity, Guid> _activityRepository;

    public DeviceRevocationManager(
        IRepository<DeviceRevocation, Guid> revocationRepository,
        IRepository<AgentPresence, Guid> presenceRepository,
        IRepository<Activity, Guid> activityRepository)
    {
        _revocationRepository = revocationRepository;
        _presenceRepository = presenceRepository;
        _activityRepository = activityRepository;
    }

    public async Task<bool> IsDeviceRevokedAsync(Guid workspaceId, Guid userId, Guid deviceId)
    {
        if (deviceId == Guid.Empty)
        {
            return false;
        }

        return await _revocationRepository.AnyAsync(r =>
            r.WorkspaceId == workspaceId &&
            r.UserId == userId &&
            r.DeviceId == deviceId);
    }

    public async Task<bool> IsDeviceObservedForUserAsync(Guid workspaceId, Guid userId, Guid deviceId)
    {
        if (deviceId == Guid.Empty)
        {
            return false;
        }

        var inPresence = await _presenceRepository.AnyAsync(p =>
            p.WorkspaceId == workspaceId &&
            p.UserId == userId &&
            p.DeviceId == deviceId);

        if (inPresence)
        {
            return true;
        }

        return await _activityRepository.AnyAsync(a =>
            a.WorkspaceId == workspaceId &&
            a.UserId == userId &&
            a.DeviceId == deviceId);
    }

    public async Task<DeviceRevocation> RevokeDeviceAsync(Guid workspaceId, Guid userId, Guid deviceId, string? reason = null)
    {
        if (deviceId == Guid.Empty)
        {
            throw new BusinessException(TrackerDomainErrorCodes.DeviceNotObserved)
                .WithData("deviceId", deviceId);
        }

        var existing = await _revocationRepository.FirstOrDefaultAsync(r =>
            r.WorkspaceId == workspaceId &&
            r.UserId == userId &&
            r.DeviceId == deviceId);

        if (existing != null)
        {
            // Idempotent: already revoked, return existing
            return existing;
        }

        var isObserved = await IsDeviceObservedForUserAsync(workspaceId, userId, deviceId);
        if (!isObserved)
        {
            // Target device was never observed for this user in this workspace
            throw new BusinessException(TrackerDomainErrorCodes.DeviceNotObserved)
                .WithData("deviceId", deviceId);
        }

        var revocation = new DeviceRevocation(
            GuidGenerator.Create(),
            workspaceId,
            userId,
            deviceId,
            Clock.Now,
            reason);

        await _revocationRepository.InsertAsync(revocation, autoSave: true);
        return revocation;
    }
}
