using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.Authorization;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Users;

namespace Dosi.Tracker.Devices;

[Authorize]
[Volo.Abp.RemoteService(IsEnabled = false)]
public class DeviceAppService : TrackerAppService, IDeviceAppService
{
    private readonly DeviceRevocationManager _revocationManager;
    private readonly IRepository<DeviceRevocation, Guid> _revocationRepository;

    public DeviceAppService(
        DeviceRevocationManager revocationManager,
        IRepository<DeviceRevocation, Guid> revocationRepository)
    {
        _revocationManager = revocationManager;
        _revocationRepository = revocationRepository;
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

    private static DeviceRevocationDto MapToDto(DeviceRevocation entity)
    {
        return new DeviceRevocationDto
        {
            Id = entity.Id,
            WorkspaceId = entity.WorkspaceId,
            UserId = entity.UserId,
            DeviceId = entity.DeviceId,
            RevokedAt = entity.RevokedAt,
            Reason = entity.Reason
        };
    }
}
