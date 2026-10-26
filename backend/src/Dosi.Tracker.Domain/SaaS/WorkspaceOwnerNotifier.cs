using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Notifications;
using Dosi.Tracker.Workspaces;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;

namespace Dosi.Tracker.SaaS;

/// <summary>Sends an in-app notification to every active owner of the current workspace — the people who can act on
/// the subscription and its invoices — and finds their email addresses for the notices that are emailed too.</summary>
public class WorkspaceOwnerNotifier : TrackerDomainService
{
    /// <summary>Longest notification text the store keeps.</summary>
    public const int MaxMessageLength = 512;

    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IRepository<IdentityUser, Guid> _userRepository;
    private readonly IRepository<Notification, Guid> _notificationRepository;
    private readonly IRepository<Workspace, Guid> _workspaceRepository;

    public WorkspaceOwnerNotifier(
        IRepository<WorkspaceMember, Guid> memberRepository,
        IRepository<IdentityUser, Guid> userRepository,
        IRepository<Notification, Guid> notificationRepository,
        IRepository<Workspace, Guid> workspaceRepository)
    {
        _memberRepository = memberRepository;
        _userRepository = userRepository;
        _notificationRepository = notificationRepository;
        _workspaceRepository = workspaceRepository;
    }

    /// <summary>Returns how many owners were notified.</summary>
    public virtual async Task<int> NotifyAsync(string message)
    {
        var workspaceId = CurrentWorkspace.GetId();
        var text = message.Length > MaxMessageLength ? message[..MaxMessageLength] : message;

        var ownerIds = (await GetOwnersAsync(workspaceId)).Select(o => o.UserId).Distinct().ToList();
        foreach (var ownerId in ownerIds)
        {
            await _notificationRepository.InsertAsync(new Notification(GuidGenerator.Create(), workspaceId, ownerId, text));
        }

        return ownerIds.Count;
    }

    /// <summary>The current workspace's name and the email addresses of its active owners (with an active account).</summary>
    public virtual async Task<(string WorkspaceName, List<string> Emails)> GetOwnerEmailsAsync()
    {
        var workspaceId = CurrentWorkspace.GetId();
        var workspace = await _workspaceRepository.GetAsync(workspaceId);
        var emails = (await GetOwnersAsync(workspaceId))
            .Select(o => o.Email)
            .Where(email => !string.IsNullOrWhiteSpace(email))
            .Distinct(StringComparer.OrdinalIgnoreCase)
            .ToList();

        return (workspace.Name, emails);
    }

    private async Task<List<(Guid UserId, string Email)>> GetOwnersAsync(Guid workspaceId)
    {
        var members = await _memberRepository.GetQueryableAsync();
        var users = await _userRepository.GetQueryableAsync();
        var owners = await AsyncExecuter.ToListAsync(
            from m in members
            join u in users on m.UserId equals u.Id
            where m.WorkspaceId == workspaceId && m.Role == WorkspaceRoles.Owner &&
                  m.Status == WorkspaceMemberStatus.Active && u.IsActive
            orderby m.JoinedAt
            select new { m.UserId, u.Email });

        return owners.Select(o => (o.UserId, o.Email)).ToList();
    }
}
