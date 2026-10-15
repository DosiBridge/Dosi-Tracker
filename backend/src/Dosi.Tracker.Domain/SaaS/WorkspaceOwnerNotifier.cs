using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Notifications;
using Dosi.Tracker.Workspaces;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;

namespace Dosi.Tracker.SaaS;

/// <summary>Sends an in-app notification to every active owner of the current workspace: the people who can act on
/// the subscription and its invoices.</summary>
public class WorkspaceOwnerNotifier : TrackerDomainService
{
    /// <summary>Longest notification text the store keeps.</summary>
    public const int MaxMessageLength = 512;

    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<IdentityUser, Guid> _userRepository;
    private readonly IRepository<Notification, Guid> _notificationRepository;

    public WorkspaceOwnerNotifier(
        IRepository<WorkspaceMember, Guid> memberRepository,
        IRepository<IdentityUser, Guid> userRepository,
        IRepository<Notification, Guid> notificationRepository)
    {
        _memberRepository = memberRepository;
        _userRepository = userRepository;
        _notificationRepository = notificationRepository;
    }

    /// <summary>Returns how many owners were notified.</summary>
    public virtual async Task<int> NotifyAsync(string message)
    {
        var workspaceId = CurrentWorkspace.GetId();
        var text = message.Length > MaxMessageLength ? message[..MaxMessageLength] : message;

        var members = await _memberRepository.GetQueryableAsync();
        var users = await _userRepository.GetQueryableAsync();
        var ownerIds = await AsyncExecuter.ToListAsync(
            from m in members
            join u in users on m.UserId equals u.Id
            where m.WorkspaceId == workspaceId && m.Role == WorkspaceRoles.Owner &&
                  m.Status == WorkspaceMemberStatus.Active && u.IsActive
            orderby m.JoinedAt
            select m.UserId);

        foreach (var ownerId in ownerIds.Distinct())
        {
            await _notificationRepository.InsertAsync(new Notification(GuidGenerator.Create(), workspaceId, ownerId, text));
        }

        return ownerIds.Distinct().Count();
    }
}
