using System;
using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Volo.Abp;
using Volo.Abp.Authorization;
using Volo.Abp.DistributedLocking;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Uow;

namespace Dosi.Tracker.Workspaces;

/// <summary>Creates workspaces: the workspace, its owner's membership and its subscription.</summary>
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
    /// Checks whether the specified workspace is archived.
    /// </summary>
    public virtual async Task<bool> IsArchivedAsync(Guid workspaceId)
    {
        var workspace = await _workspaceRepository.FindAsync(workspaceId);
        return workspace?.IsArchived ?? false;
    }
}
