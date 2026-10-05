using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Notifications;
using Microsoft.AspNetCore.Identity;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Domain.Services;
using Volo.Abp.Identity;

namespace Dosi.Tracker.SaaS;

/// <summary>Sends an in-app notification to every active owner (Identity "admin" role) of the current workspace:
/// the people who can act on the subscription and its invoices.</summary>
public class WorkspaceOwnerNotifier : DomainService
{
    /// <summary>Longest notification text the store keeps.</summary>
    public const int MaxMessageLength = 512;

    private readonly IIdentityUserRepository _userRepository;
    private readonly IRepository<Notification, Guid> _notificationRepository;
    private readonly ILookupNormalizer _lookupNormalizer;

    public WorkspaceOwnerNotifier(
        IIdentityUserRepository userRepository,
        IRepository<Notification, Guid> notificationRepository,
        ILookupNormalizer lookupNormalizer)
    {
        _userRepository = userRepository;
        _notificationRepository = notificationRepository;
        _lookupNormalizer = lookupNormalizer;
    }

    /// <summary>Returns how many owners were notified.</summary>
    public virtual async Task<int> NotifyAsync(string message)
    {
        var text = message.Length > MaxMessageLength ? message[..MaxMessageLength] : message;
        var owners = await _userRepository.GetListByNormalizedRoleNameAsync(_lookupNormalizer.NormalizeName(TrackerRoles.Admin));

        var notified = 0;
        foreach (var owner in owners.Where(o => o.IsActive))
        {
            await _notificationRepository.InsertAsync(new Notification(GuidGenerator.Create(), CurrentTenant.Id, owner.Id, text));
            notified++;
        }

        return notified;
    }
}
