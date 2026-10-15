using System;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Volo.Abp.Data;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Entities.Events;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.EventBus;
using Volo.Abp.Identity;
using Volo.Abp.Uow;

namespace Dosi.Tracker.Identity;

/// <summary>
/// A deleted account leaves every workspace: its workspace memberships end and its project memberships (which occupy,
/// and bill, seats) are removed. The member's rate history is kept: payroll for time they already tracked needs it.
/// </summary>
public class UserMembershipCleanupHandler : ILocalEventHandler<EntityDeletedEventData<IdentityUser>>, ITransientDependency
{
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<WorkspaceMember, Guid> _workspaceMemberRepository;
    private readonly IDataFilter _dataFilter;

    public ILogger<UserMembershipCleanupHandler> Logger { get; set; }

    public UserMembershipCleanupHandler(
        IRepository<ProjectMember, Guid> memberRepository,
        IRepository<WorkspaceMember, Guid> workspaceMemberRepository,
        IDataFilter dataFilter)
    {
        _memberRepository = memberRepository;
        _workspaceMemberRepository = workspaceMemberRepository;
        _dataFilter = dataFilter;
        Logger = NullLogger<UserMembershipCleanupHandler>.Instance;
    }

    [UnitOfWork]
    public virtual async Task HandleEventAsync(EntityDeletedEventData<IdentityUser> eventData)
    {
        var userId = eventData.Entity.Id;

        using (_dataFilter.Disable<IWorkspaceScoped>())
        {
            var memberships = await _memberRepository.GetListAsync(m => m.UserId == userId);
            if (memberships.Count > 0)
            {
                await _memberRepository.DeleteManyAsync(memberships);
            }

            var workspaceMemberships = await _workspaceMemberRepository.GetListAsync(m =>
                m.UserId == userId && m.Status == WorkspaceMemberStatus.Active);
            foreach (var membership in workspaceMemberships)
            {
                membership.Remove();
            }

            await _workspaceMemberRepository.UpdateManyAsync(workspaceMemberships);

            Logger.LogInformation("Deleted user {UserId} left {Workspaces} workspace(s) and {Projects} project membership(s).",
                userId, workspaceMemberships.Count, memberships.Count);
        }
    }
}
