using System;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Mvc;

namespace Dosi.Tracker.Controllers;

/// <summary>
/// Streams capture images as binary content (the auto-API would base64-wrap them in JSON).
/// Authorization/ownership is enforced by <see cref="IActivityAppService.GetScreenshotContentAsync"/>.
/// Private capture data is strictly non-storable (Cache-Control: no-store, private).
/// </summary>
[Route("api/app/activity/screenshot")]
[Authorize]
[ResponseCache(NoStore = true, Location = ResponseCacheLocation.None)]
public class ScreenshotContentController : TrackerController
{
    private readonly IActivityAppService _activityAppService;

    public ScreenshotContentController(IActivityAppService activityAppService)
    {
        _activityAppService = activityAppService;
    }

    [HttpGet("{id}/content")]
    public async Task<IActionResult> GetContentAsync(Guid id)
    {
        Response.Headers.CacheControl = "no-store, private";
        Response.Headers.Pragma = "no-cache";

        var content = await _activityAppService.GetScreenshotContentAsync(id);
        return File(content.Bytes, content.ContentType, content.FileName);
    }
}
