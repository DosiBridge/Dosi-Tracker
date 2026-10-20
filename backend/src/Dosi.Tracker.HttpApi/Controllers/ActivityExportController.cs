using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Mvc;

namespace Dosi.Tracker.Controllers;

/// <summary>
/// Authenticated workspace-scoped activity CSV export endpoint (REQ-DAT-02).
/// Enforces workspace context via WorkspaceRequiredFilter and returns downloadable CSV stream.
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
}
