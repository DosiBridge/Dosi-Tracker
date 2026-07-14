using Dosi.Tracker.Localization;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker;

/* Inherit your application services from this class.
 */
public abstract class TrackerAppService : ApplicationService
{
    protected TrackerAppService()
    {
        LocalizationResource = typeof(TrackerResource);
    }
}
