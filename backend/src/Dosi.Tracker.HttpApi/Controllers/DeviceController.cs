using System;
using System.Threading.Tasks;
using Dosi.Tracker.Devices;
using Dosi.Tracker.Permissions;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Mvc;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Controllers;

/// <summary>Devices of the current workspace (<see cref="IDeviceAppService"/>): served by this controller so the routes
/// stay the agent contract's (docs/agent-api-contract.md).</summary>
[Route("api/app/device")]
[Authorize]
public class DeviceController : TrackerController, IDeviceAppService
{
    private readonly IDeviceAppService _deviceAppService;

    public DeviceController(IDeviceAppService deviceAppService)
    {
        _deviceAppService = deviceAppService;
    }

    [HttpPost("revoke")]
    public Task<DeviceRevocationDto> RevokeAsync([FromBody] RevokeDeviceInput input)
    {
        return _deviceAppService.RevokeAsync(input);
    }

    [HttpGet("my-revocations")]
    public Task<ListResultDto<DeviceRevocationDto>> GetMyRevocationsAsync()
    {
        return _deviceAppService.GetMyRevocationsAsync();
    }

    [HttpGet("{deviceId}/status")]
    public Task<DeviceStatusDto> GetStatusAsync(Guid deviceId)
    {
        return _deviceAppService.GetStatusAsync(deviceId);
    }

    [HttpGet]
    public Task<ListResultDto<WorkspaceDeviceDto>> GetListAsync()
    {
        return _deviceAppService.GetListAsync();
    }

    [HttpPost("revoke-member")]
    [Authorize(TrackerPermissions.Devices.Manage)]
    public Task<DeviceRevocationDto> RevokeMemberDeviceAsync([FromBody] RevokeMemberDeviceInput input)
    {
        return _deviceAppService.RevokeMemberDeviceAsync(input);
    }

    [HttpPost("restore")]
    [Authorize(TrackerPermissions.Devices.Manage)]
    public Task<DeviceStatusDto> RestoreAsync([FromBody] RestoreDeviceInput input)
    {
        return _deviceAppService.RestoreAsync(input);
    }
}
