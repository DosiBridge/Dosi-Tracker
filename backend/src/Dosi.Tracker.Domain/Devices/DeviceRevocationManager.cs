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
/// <para>
/// A revocation is kept per (workspace, member, device id). It refuses the device in that workspace (uploads,
/// heartbeats and every other call made in it answer 403 <c>Tracker:DeviceRevoked</c>) and revokes the device's
/// sign-in sessions everywhere (<see cref="DeviceSessionManager"/>), so the install cannot even refresh its tokens.
/// A new device id does not undo it silently: the install has to sign in again with the member's password, and then
/// shows up as a new device of that member. Stopping a person, rather than an install, is removing them from the
/// workspace.
/// </para>
/// </summary>
public class DeviceRevocationManager : DomainService
{
    private readonly IRepository<DeviceRevocation, Guid> _revocationRepository;
    private readonly IRepository<AgentPresence, Guid> _presenceRepository;
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly DeviceSessionManager _sessionManager;

    public DeviceRevocationManager(
        IRepository<DeviceRevocation, Guid> revocationRepository,
        IRepository<AgentPresence, Guid> presenceRepository,
        IRepository<Activity, Guid> activityRepository,
        DeviceSessionManager sessionManager)
    {
        _revocationRepository = revocationRepository;
        _presenceRepository = presenceRepository;
        _activityRepository = activityRepository;
        _sessionManager = sessionManager;
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

    /// <summary>
    /// The device an upload or heartbeat is recorded for, after refusing a revoked one. <paramref name="tokenDeviceId"/>
    /// is the device the caller's token is bound to, <paramref name="requestDeviceId"/> the one the request body names.
    /// Both are checked, so neither leaving the body's id out nor inventing a fresh one gets a revoked install's data in;
    /// the body's id is the one recorded (the install that recorded a queued block), else the token's. A request naming
    /// no device at all is refused (400 <c>Tracker:DeviceIdRequired</c>): revocation could not apply to it.
    /// </summary>
    public async Task<Guid> EnsureCanIngestAsync(Guid workspaceId, Guid userId, Guid? tokenDeviceId, Guid? requestDeviceId)
    {
        var token = tokenDeviceId is { } t && t != Guid.Empty ? t : (Guid?)null;
        var request = requestDeviceId is { } r && r != Guid.Empty ? r : (Guid?)null;
        if (token == null && request == null)
        {
            throw new BusinessException(TrackerDomainErrorCodes.DeviceIdRequired);
        }

        foreach (var deviceId in new[] { token, request })
        {
            if (deviceId is { } id && await IsDeviceRevokedAsync(workspaceId, userId, id))
            {
                throw new BusinessException(TrackerDomainErrorCodes.DeviceRevoked)
                    .WithData("deviceId", id);
            }
        }

        return (request ?? token)!.Value;
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

    /// <summary>Revokes a device of <paramref name="userId"/> in the workspace and revokes its sign-in sessions. Revoking
    /// an already revoked device returns the existing record and revokes its sessions again (a sign-in made since).</summary>
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
            await _sessionManager.RevokeAsync(userId, deviceId);
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
        await _sessionManager.RevokeAsync(userId, deviceId);
        return revocation;
    }

    /// <summary>Lifts the revocation of a device in the workspace (404 <c>Tracker:DeviceNotFound</c> when there is none).
    /// The sessions revoked with it stay revoked: the install signs in again.</summary>
    public async Task RestoreDeviceAsync(Guid workspaceId, Guid userId, Guid deviceId)
    {
        var existing = await _revocationRepository.FirstOrDefaultAsync(r =>
            r.WorkspaceId == workspaceId &&
            r.UserId == userId &&
            r.DeviceId == deviceId);

        if (existing == null)
        {
            throw new BusinessException(TrackerDomainErrorCodes.DeviceNotFound)
                .WithData("deviceId", deviceId);
        }

        await _revocationRepository.DeleteAsync(existing, autoSave: true);
    }
}
