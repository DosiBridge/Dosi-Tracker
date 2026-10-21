using System;
using System.Threading.Tasks;
using Dosi.Tracker.Devices;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Mvc;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Controllers;

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
}
