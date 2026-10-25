using System;
using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Volo.Abp;
using Volo.Abp.Authorization;
using Volo.Abp.DistributedLocking;
using Volo.Abp.Domain.Repositories;
using Microsoft.Extensions.Logging;
using Volo.Abp.Uow;

namespace Dosi.Tracker.Workspaces;

/// <summary>Creates workspaces (the workspace, its owner's membership and its subscription), archives, restores and
/// deletes them.</summary>
public class WorkspaceManager : TrackerDomainService
{
    /// <summary>Prefix of the lock that serializes creations of the same workspace name.</summary>
    public const string NameLockPrefix = "Tracker:WorkspaceName:";

    private static readonly TimeSpan NameLockTimeout = TimeSpan.FromSeconds(30);

    private readonly IRepository<Workspace, Guid> _workspaceRepository;
    private readonly IRepository<Subscription, Guid> _subscriptionRepository;
    private readonly WorkspaceMembershipManager _memberships;
    private readonly SubscriptionManager _subscriptionManager;
    private readonly IAbpDistributedLock _distributedLock;

    public WorkspaceManager(
        IRepository<Workspace, Guid> workspaceRepository,
        IRepository<Subscription, Guid> subscriptionRepository,
        WorkspaceMembershipManager memberships,
        SubscriptionManager subscriptionManager,
        IAbpDistributedLock distributedLock)
    {
        _workspaceRepository = workspaceRepository;
        _subscriptionRepository = subscriptionRepository;
        _memberships = memberships;
        _subscriptionManager = subscriptionManager;
        _distributedLock = distributedLock;
    }

    /// <summary>Serializes creations of this name until the returned handle is disposed (hold it until the creating
    /// unit of work is committed).</summary>
    public virtual async Task<IAsyncDisposable> LockNameAsync(string name)
    {
        var handle = await _distributedLock.TryAcquireAsync(NameLockPrefix + Workspace.NormalizeName(name), NameLockTimeout);
        return handle ?? throw DuplicateName(name);
    }

    public virtual async Task<bool> IsNameTakenAsync(string name)
    {
        var normalized = Workspace.NormalizeName(name);
        return await _workspaceRepository.AnyAsync(w => w.NormalizedName == normalized);
    }

    /// <summary>Creates the workspace with <paramref name="ownerUserId"/> as its owner and a subscription on
    /// <paramref name="plan"/>. Refuses a name already in use (<c>Tracker:DuplicateWorkspaceName</c>).</summary>
    public virtual async Task<Workspace> CreateAsync(string name, Guid ownerUserId, Plan plan)
    {
        if (await IsNameTakenAsync(name))
        {
            throw DuplicateName(name);
        }

        var workspace = new Workspace(GuidGenerator.Create(), name);
        try
        {
            await _workspaceRepository.InsertAsync(workspace, autoSave: true);
        }
        catch (Exception ex) when (ex is not BusinessException)
        {
            // A concurrent creation committed the name first: the unique index refused this one.
            if (await IsNameTakenInNewUnitOfWorkAsync(name))
            {
                throw DuplicateName(name);
            }

            throw;
        }

        await _memberships.AddAsync(workspace.Id, ownerUserId, WorkspaceRoles.Owner);
        await _subscriptionRepository.InsertAsync(_subscriptionManager.CreateForNewWorkspace(workspace.Id, plan), autoSave: true);
        return workspace;
    }

    private async Task<bool> IsNameTakenInNewUnitOfWorkAsync(string name)
    {
        // The failed insert leaves the current unit of work's database context unusable.
        using var uow = LazyServiceProvider.LazyGetRequiredService<IUnitOfWorkManager>().Begin(requiresNew: true);
        var taken = await IsNameTakenAsync(name);
        await uow.CompleteAsync();
        return taken;
    }

    public static BusinessException DuplicateName(string name)
    {
        return new BusinessException(TrackerDomainErrorCodes.DuplicateWorkspaceName).WithData("name", name.Trim());
    }

    /// <summary>
    /// Archives the workspace. Reversible and idempotent.
    /// Strictly requires the operator to be an active owner of the target workspace.
    /// </summary>
    public virtual async Task<Workspace> ArchiveAsync(Guid workspaceId, Guid operatorUserId)
    {
        var workspace = await _workspaceRepository.GetAsync(workspaceId);
        var member = await _memberships.FindActiveAsync(workspaceId, operatorUserId);
        if (member == null || !member.IsOwner)
        {
            throw new AbpAuthorizationException("Only an active owner of the workspace can archive it.");
        }

        if (!workspace.IsArchived)
        {
            workspace.Archive(operatorUserId, Clock.Now);
            await _workspaceRepository.UpdateAsync(workspace, autoSave: true);
        }

        return workspace;
    }

    /// <summary>
    /// Restores an archived workspace to active. Reversible and idempotent.
    /// Strictly requires the operator to be an active owner of the target workspace.
    /// </summary>
    public virtual async Task<Workspace> RestoreAsync(Guid workspaceId, Guid operatorUserId)
    {
        var workspace = await _workspaceRepository.GetAsync(workspaceId);
        var member = await _memberships.FindActiveAsync(workspaceId, operatorUserId);
        if (member == null || !member.IsOwner)
        {
            throw new AbpAuthorizationException("Only an active owner of the workspace can restore it.");
        }

        if (workspace.IsArchived)
        {
            workspace.Restore();
            await _workspaceRepository.UpdateAsync(workspace, autoSave: true);
        }

        return workspace;
    }

    /// <summary>
    /// Deletes the workspace (owners only), confirmed by typing its name (<paramref name="confirmationName"/>, compared
    /// trimmed and case-insensitively; otherwise <c>Tracker:WorkspaceDeleteConfirmationMismatch</c>). The workspace is
    /// soft-deleted: at once nobody can open it any more (its members and agents are refused as non-members), its
    /// pending invitations are withdrawn and its subscription is canceled, so it is never invoiced again. Its data is
    /// purged by the erasure sweep once the grace period is over (<see cref="Erasure.DataErasureOptions"/>). Returns the
    /// deleted workspace. Saved with the caller's unit of work.
    /// </summary>
    public virtual async Task<Workspace> DeleteAsync(Guid workspaceId, Guid operatorUserId, string? confirmationName)
    {
        var workspace = await _workspaceRepository.GetAsync(workspaceId);
        var member = await _memberships.FindActiveAsync(workspaceId, operatorUserId);
        if (member == null || !member.IsOwner)
        {
            throw new AbpAuthorizationException("Only an active owner of the workspace can delete it.");
        }

        if (!string.Equals(confirmationName?.Trim(), workspace.Name, StringComparison.OrdinalIgnoreCase))
        {
            throw new BusinessException(TrackerDomainErrorCodes.WorkspaceDeleteConfirmationMismatch);
        }

        var invitationRepository = LazyServiceProvider.LazyGetRequiredService<IRepository<WorkspaceInvitation, Guid>>();
        var pending = await invitationRepository.GetListAsync(i => i.WorkspaceId == workspaceId && i.AcceptedAt == null);
        if (pending.Count > 0)
        {
            await invitationRepository.DeleteManyAsync(pending, autoSave: true);
        }

        using (CurrentWorkspace.Change(workspaceId))
        {
            var subscription = await _subscriptionRepository.FirstOrDefaultAsync(s => s.WorkspaceId == workspaceId);
            if (subscription != null && subscription.Status != Subscription.CanceledStatus)
            {
                subscription.Cancel();
                await _subscriptionRepository.UpdateAsync(subscription, autoSave: true);
            }
        }

        await _workspaceRepository.DeleteAsync(workspace, autoSave: true);
        Logger.LogInformation("Workspace {WorkspaceId} was deleted by its owner {UserId}; its data will be purged.",
            workspaceId, operatorUserId);
        return workspace;
    }

    /// <summary>
    /// Checks whether the specified workspace is archived.
    /// </summary>
    public virtual async Task<bool> IsArchivedAsync(Guid workspaceId)
    {
        var workspace = await _workspaceRepository.FindAsync(workspaceId);
        return workspace?.IsArchived ?? false;
    }
}
