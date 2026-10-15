using Dosi.Tracker.Workspaces;
using Volo.Abp.Domain.Services;

namespace Dosi.Tracker;

public abstract class TrackerDomainService : DomainService
{
    protected ICurrentWorkspace CurrentWorkspace => LazyServiceProvider.LazyGetRequiredService<ICurrentWorkspace>();
}
