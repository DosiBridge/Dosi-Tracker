using System;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Entities.Events;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.EventBus;
using Volo.Abp.Identity;
using Volo.Abp.MultiTenancy;
using Volo.Abp.Uow;

namespace Dosi.Tracker.Identity;

/// <summary>
/// Removes the project memberships of a deleted Identity user (the Identity endpoints soft-delete users, which also
/// raises this event). A membership is what occupies — and bills — a seat, so leaving it behind would keep charging
/// for someone who is gone and could fill the plan's seat limit. The member's rate history is kept: payroll for time
/// they already tracked still needs it.
/// </summary>
public class UserMembershipCleanupHandler : ILocalEventHandler<EntityDeletedEventData<IdentityUser>>, ITransientDependency
{
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly ICurrentTenant _currentTenant;

    public ILogger<UserMembershipCleanupHandler> Logger { get; set; }

    public UserMembershipCleanupHandler(IRepository<ProjectMember, Guid> memberRepository, ICurrentTenant currentTenant)
    {
        _memberRepository = memberRepository;
        _currentTenant = currentTenant;
        Logger = NullLogger<UserMembershipCleanupHandler>.Instance;
    }

    [UnitOfWork]
    public virtual async Task HandleEventAsync(EntityDeletedEventData<IdentityUser> eventData)
    {
        var user = eventData.Entity;

        // The user's own workspace, whatever the caller's context (e.g. a host operation).
        using (_currentTenant.Change(user.TenantId))
        {
            var memberships = await _memberRepository.GetListAsync(m => m.UserId == user.Id);
            if (memberships.Count == 0)
            {
                return;
            }

            await _memberRepository.DeleteManyAsync(memberships);
            Logger.LogInformation("Removed {Count} project membership(s) of deleted user {UserId}.", memberships.Count, user.Id);
        }
    }
}
