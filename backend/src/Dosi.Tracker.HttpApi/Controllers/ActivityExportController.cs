using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Mvc;

namespace Dosi.Tracker.Controllers;

/// <summary>
/// Authenticated workspace-scoped activity export endpoints (REQ-DAT-02).
/// Enforces workspace context via WorkspaceRequiredFilter and returns downloadable CSV or JSON stream.
/// </summary>
[Route("api/app/activity")]
[Authorize]
public class ActivityExportController : TrackerController
{
    private readonly IActivityAppService _activityAppService;

    public ActivityExportController(IActivityAppService activityAppService)
    {
        _activityAppService = activityAppService;
    }

    [HttpGet("export")]
    public async Task<IActionResult> ExportCsvAsync([FromQuery] ExportActivitiesInput input)
    {
        var result = await _activityAppService.ExportCsvAsync(input);
        Response.Headers["X-Total-Count"] = result.TotalCount.ToString();
        Response.Headers["X-Exported-Count"] = result.ExportedCount.ToString();
        return File(result.Bytes, result.ContentType, result.FileName);
    }

    [HttpGet("export/json")]
    public async Task<IActionResult> ExportJsonAsync([FromQuery] ExportActivitiesInput input)
    {
        var result = await _activityAppService.ExportJsonAsync(input);
        Response.Headers["X-Total-Count"] = result.TotalCount.ToString();
        Response.Headers["X-Exported-Count"] = result.ExportedCount.ToString();
        Response.Headers["Cache-Control"] = "no-store, private";
        Response.Headers["Pragma"] = "no-cache";
        return File(result.Bytes, result.ContentType, result.FileName);
    }
}
