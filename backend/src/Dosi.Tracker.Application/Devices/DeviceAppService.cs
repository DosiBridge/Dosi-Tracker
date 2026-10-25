using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Presence;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.Authorization;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Users;

namespace Dosi.Tracker.Devices;

[Authorize]
[Volo.Abp.RemoteService(IsEnabled = false)]
public class DeviceAppService : TrackerAppService, IDeviceAppService
{
    private readonly DeviceRevocationManager _revocationManager;
    private readonly IRepository<DeviceRevocation, Guid> _revocationRepository;
    private readonly IRepository<AgentPresence, Guid> _presenceRepository;
    private readonly WorkspaceMembershipManager _membershipManager;
    private readonly IIdentityUserRepository _userRepository;

    public DeviceAppService(
        DeviceRevocationManager revocationManager,
        IRepository<DeviceRevocation, Guid> revocationRepository,
        IRepository<AgentPresence, Guid> presenceRepository,
        WorkspaceMembershipManager membershipManager,
        IIdentityUserRepository userRepository)
    {
        _revocationManager = revocationManager;
        _revocationRepository = revocationRepository;
        _presenceRepository = presenceRepository;
        _membershipManager = membershipManager;
        _userRepository = userRepository;
    }

    public async Task<DeviceRevocationDto> RevokeAsync(RevokeDeviceInput input)
    {
        var workspaceId = CurrentWorkspace.GetId();
        var userId = CurrentUser.GetId();

        var revocation = await _revocationManager.RevokeDeviceAsync(
            workspaceId,
            userId,
            input.DeviceId,
            input.Reason);

        return MapToDto(revocation);
    }

    [Authorize(TrackerPermissions.Devices.Manage)]
    public async Task<DeviceRevocationDto> RevokeMemberDeviceAsync(RevokeMemberDeviceInput input)
    {
        var workspaceId = CurrentWorkspace.GetId();
        await EnsureMayManageDevicesOfAsync(workspaceId, input.UserId);

        // A device never seen for that member in this workspace (another workspace's member, a guessed id) is
        // refused as not observed, whoever the member is: nothing about other workspaces is disclosed.
        var revocation = await _revocationManager.RevokeDeviceAsync(
            workspaceId,
            input.UserId,
            input.DeviceId,
            input.Reason);

        return MapToDto(revocation);
    }

    [Authorize(TrackerPermissions.Devices.Manage)]
    public async Task<DeviceStatusDto> RestoreAsync(RestoreDeviceInput input)
    {
        var workspaceId = CurrentWorkspace.GetId();
        await EnsureMayManageDevicesOfAsync(workspaceId, input.UserId);

        await _revocationManager.RestoreDeviceAsync(workspaceId, input.UserId, input.DeviceId);

        return new DeviceStatusDto { DeviceId = input.DeviceId, IsRevoked = false };
    }

    public async Task<ListResultDto<WorkspaceDeviceDto>> GetListAsync()
    {
        var workspaceId = CurrentWorkspace.GetId();
        Guid? onlyUserId = await AuthorizationService.IsGrantedAsync(TrackerPermissions.Devices.Manage)
            ? null
            : CurrentUser.GetId();

        var presenceQuery = (await _presenceRepository.GetQueryableAsync()).Where(p => p.WorkspaceId == workspaceId);
        var revocationQuery = (await _revocationRepository.GetQueryableAsync()).Where(r => r.WorkspaceId == workspaceId);
        if (onlyUserId is { } ownId)
        {
            presenceQuery = presenceQuery.Where(p => p.UserId == ownId);
            revocationQuery = revocationQuery.Where(r => r.UserId == ownId);
        }

        var presences = await AsyncExecuter.ToListAsync(presenceQuery
            .OrderByDescending(p => p.LastSeenAt)
            .Take(DeviceConsts.MaxDevicesListing));
        var revocations = await AsyncExecuter.ToListAsync(revocationQuery
            .OrderByDescending(r => r.RevokedAt)
            .Take(DeviceConsts.MaxDevicesListing));

        var revocationsByDevice = revocations.ToDictionary(r => (r.UserId, r.DeviceId));
        var devices = presences
            .Select(p => new WorkspaceDeviceDto
            {
                UserId = p.UserId,
                DeviceId = p.DeviceId,
                DeviceName = p.DeviceName,
                Platform = p.Platform,
                OsVersion = p.OsVersion,
                AgentVersion = p.AgentVersion,
                LastSeenAt = p.LastSeenAt
            })
            .ToList();

        var listed = devices.Select(d => (d.UserId, d.DeviceId)).ToHashSet();
        devices.AddRange(revocations
            .Where(r => !listed.Contains((r.UserId, r.DeviceId)))
            .Select(r => new WorkspaceDeviceDto { UserId = r.UserId, DeviceId = r.DeviceId }));

        var users = (await _userRepository.GetListByIdsAsync(devices.Select(d => d.UserId).Distinct()))
            .ToDictionary(u => u.Id);

        foreach (var device in devices)
        {
            if (users.TryGetValue(device.UserId, out var user))
            {
                device.UserName = user.UserName;
                var fullName = string.Join(" ",
                    new[] { user.Name, user.Surname }.Where(s => !string.IsNullOrWhiteSpace(s)).Select(s => s!.Trim()));
                device.Name = fullName.Length == 0 ? null : fullName;
            }

            if (revocationsByDevice.TryGetValue((device.UserId, device.DeviceId), out var revocation))
            {
                device.IsRevoked = true;
                device.RevokedAt = revocation.RevokedAt;
                device.RevokedByUserId = revocation.CreatorId;
                device.Reason = revocation.Reason;
            }
        }

        var items = devices
            .OrderBy(d => d.IsRevoked)
            .ThenByDescending(d => d.LastSeenAt ?? d.RevokedAt)
            .ThenBy(d => d.DeviceId)
            .Take(DeviceConsts.MaxDevicesListing)
            .ToList();

        return new ListResultDto<WorkspaceDeviceDto>(items);
    }

    public async Task<ListResultDto<DeviceRevocationDto>> GetMyRevocationsAsync()
    {
        var workspaceId = CurrentWorkspace.GetId();
        var userId = CurrentUser.GetId();

        var query = (await _revocationRepository.GetQueryableAsync())
            .Where(r => r.WorkspaceId == workspaceId && r.UserId == userId)
            .OrderByDescending(r => r.RevokedAt)
            .Take(DeviceConsts.MaxRevocationsListing);

        var list = await AsyncExecuter.ToListAsync(query);
        var dtos = list.Select(MapToDto).ToList();

        return new ListResultDto<DeviceRevocationDto>(dtos);
    }

    public async Task<DeviceStatusDto> GetStatusAsync(Guid deviceId)
    {
        var workspaceId = CurrentWorkspace.GetId();
        var userId = CurrentUser.GetId();

        var revocation = await _revocationRepository.FirstOrDefaultAsync(r =>
            r.WorkspaceId == workspaceId &&
            r.UserId == userId &&
            r.DeviceId == deviceId);

        return new DeviceStatusDto
        {
            DeviceId = deviceId,
            IsRevoked = revocation != null,
            RevokedAt = revocation?.RevokedAt,
            Reason = revocation?.Reason
        };
    }

    /// <summary>A manager may manage everyone's devices but an owner's, like the rest of an owner's account; an owner
    /// may manage anyone's (their own included).</summary>
    private async Task EnsureMayManageDevicesOfAsync(Guid workspaceId, Guid targetUserId)
    {
        var target = await _membershipManager.FindAsync(workspaceId, targetUserId);
        if (target is not { IsOwner: true })
        {
            return;
        }

        var caller = await _membershipManager.FindActiveAsync(workspaceId, CurrentUser.GetId());
        if (caller is not { IsOwner: true })
        {
            throw new BusinessException(TrackerDomainErrorCodes.RoleNotAllowed)
                .WithData("userId", targetUserId);
        }
    }

    private static DeviceRevocationDto MapToDto(DeviceRevocation entity)
    {
        return new DeviceRevocationDto
        {
            Id = entity.Id,
            WorkspaceId = entity.WorkspaceId,
            UserId = entity.UserId,
            DeviceId = entity.DeviceId,
            RevokedAt = entity.RevokedAt,
            Reason = entity.Reason,
            RevokedByUserId = entity.CreatorId
        };
    }
}
