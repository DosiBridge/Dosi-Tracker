using Dosi.Tracker.Workspaces;
using Volo.Abp.Data;
using Volo.Abp.Domain.Services;
using Volo.Abp.Uow;
using Volo.Abp.Users;

namespace Dosi.Tracker;

public abstract class TrackerDomainService : DomainService
{
    protected ICurrentWorkspace CurrentWorkspace => LazyServiceProvider.LazyGetRequiredService<ICurrentWorkspace>();
    protected IUnitOfWorkManager UnitOfWorkManager => LazyServiceProvider.LazyGetRequiredService<IUnitOfWorkManager>();
    protected IDataFilter DataFilter => LazyServiceProvider.LazyGetRequiredService<IDataFilter>();
    protected ICurrentUser CurrentUser => LazyServiceProvider.LazyGetRequiredService<ICurrentUser>();
}
