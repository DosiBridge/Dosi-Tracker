using System;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Permissions;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Mvc;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Controllers;

/// <summary>Deleting tracked data of the current workspace (<see cref="IActivityErasureAppService"/>). Deletions are
/// POSTs with a JSON body, so the reason never travels in a URL (request logs keep URLs).</summary>
[Route("api/app/activity")]
[Authorize]
public class ActivityErasureController : TrackerController, IActivityErasureAppService
{
    private readonly IActivityErasureAppService _erasureAppService;

    public ActivityErasureController(IActivityErasureAppService erasureAppService)
    {
        _erasureAppService = erasureAppService;
    }

    [HttpPost("{id:guid}/delete")]
    [Authorize(TrackerPermissions.Activities.Delete)]
    public Task<ActivityErasureDto> DeleteBlockAsync(Guid id, [FromBody] DeleteTrackedDataDto input)
    {
        return _erasureAppService.DeleteBlockAsync(id, input);
    }

    [HttpPost("capture/{captureId:guid}/delete")]
    public Task<ActivityErasureDto> DeleteCaptureAsync(Guid captureId, [FromBody] DeleteCaptureDto input)
    {
        return _erasureAppService.DeleteCaptureAsync(captureId, input);
    }

    [HttpGet("erasures")]
    public Task<PagedResultDto<ActivityErasureDto>> GetErasuresAsync([FromQuery] PagedResultRequestDto input)
    {
        return _erasureAppService.GetErasuresAsync(input);
    }
}
