using System;
using System.Threading.Tasks;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Devices;

/// <summary>
/// Authenticated self-service device revocation for the caller's own observed devices in the current authorized workspace.
/// </summary>
public interface IDeviceAppService : IApplicationService
{
    Task<DeviceRevocationDto> RevokeAsync(RevokeDeviceInput input);
    Task<ListResultDto<DeviceRevocationDto>> GetMyRevocationsAsync();
    Task<DeviceStatusDto> GetStatusAsync(Guid deviceId);
}
