using Dosi.Tracker.Localization;
using Dosi.Tracker.Workspaces;
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

    protected ICurrentWorkspace CurrentWorkspace => LazyServiceProvider.LazyGetRequiredService<ICurrentWorkspace>();
}
