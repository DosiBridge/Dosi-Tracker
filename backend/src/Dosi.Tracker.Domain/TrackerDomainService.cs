using Dosi.Tracker.Workspaces;
using Volo.Abp.Domain.Services;
using Volo.Abp.Uow;

namespace Dosi.Tracker;

public abstract class TrackerDomainService : DomainService
{
    protected ICurrentWorkspace CurrentWorkspace => LazyServiceProvider.LazyGetRequiredService<ICurrentWorkspace>();
    protected IUnitOfWorkManager UnitOfWorkManager => LazyServiceProvider.LazyGetRequiredService<IUnitOfWorkManager>();
}
