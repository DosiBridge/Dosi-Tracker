using Dosi.Tracker.Localization;
using Volo.Abp.AspNetCore.Mvc;

namespace Dosi.Tracker.Controllers;

/* Inherit your controllers from this class.
 */
public abstract class TrackerController : AbpControllerBase
{
    protected TrackerController()
    {
        LocalizationResource = typeof(TrackerResource);
    }
}
