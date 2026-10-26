using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Notifications;
using Dosi.Tracker.Workspaces;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;

namespace Dosi.Tracker.AiAssist;

/// <summary>Sends an in-app notification to every active member of the current workspace whose account is active —
/// owners, managers, members and clients alike: what AI assist may send concerns everyone's data.</summary>
public class WorkspaceMemberNotifier : TrackerDomainService
{
    /// <summary>Longest notification text the store keeps.</summary>
    public const int MaxMessageLength = 512;

    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<IdentityUser, Guid> _userRepository;
    private readonly IRepository<Notification, Guid> _notificationRepository;

    public WorkspaceMemberNotifier(
        IRepository<WorkspaceMember, Guid> memberRepository,
        IRepository<IdentityUser, Guid> userRepository,
        IRepository<Notification, Guid> notificationRepository)
    {
        _memberRepository = memberRepository;
        _userRepository = userRepository;
        _notificationRepository = notificationRepository;
    }

    /// <summary>Returns how many members were notified.</summary>
    public virtual async Task<int> NotifyAsync(string message)
    {
        var workspaceId = CurrentWorkspace.GetId();
        var text = message.Length > MaxMessageLength ? message[..MaxMessageLength] : message;

        var members = await _memberRepository.GetQueryableAsync();
        var users = await _userRepository.GetQueryableAsync();
        var memberIds = await AsyncExecuter.ToListAsync(
            (from m in members
             join u in users on m.UserId equals u.Id
             where m.WorkspaceId == workspaceId && m.Status == WorkspaceMemberStatus.Active && u.IsActive
             select m.UserId).Distinct());

        foreach (var userId in memberIds)
        {
            await _notificationRepository.InsertAsync(new Notification(GuidGenerator.Create(), workspaceId, userId, text));
        }

        return memberIds.Count;
    }
}
