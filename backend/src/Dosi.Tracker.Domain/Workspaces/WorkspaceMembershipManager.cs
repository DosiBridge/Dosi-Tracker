using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using Volo.Abp;
using Volo.Abp.DistributedLocking;
using Volo.Abp.Domain.Repositories;

namespace Dosi.Tracker.Workspaces;

/// <summary>Adds people to workspaces, changes their role and removes them. Removal only ends the membership: the
/// person's account (global) is never touched.</summary>
public class WorkspaceMembershipManager : TrackerDomainService
{
    /// <summary>Timeout for acquiring the lock serializing workspace ownership transitions.</summary>
    public static readonly TimeSpan OwnerLockTimeout = TimeSpan.FromSeconds(30);

    /// <summary>Name of the distributed lock that serializes owner removals and demotions of a workspace.</summary>
    public static string OwnerLockName(Guid workspaceId) => $"Tracker:WorkspaceOwners:{workspaceId:N}";

    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;
    private readonly IAbpDistributedLock _distributedLock;
    private static string OwnerLockItemKey(Guid workspaceId) => $"Tracker:WorkspaceOwnersLock:{workspaceId:N}";

    public WorkspaceMembershipManager(
        IRepository<WorkspaceMember, Guid> memberRepository,
        IAbpDistributedLock distributedLock)
    {
        _memberRepository = memberRepository;
        _distributedLock = distributedLock;
    }

    /// <summary>
    /// Serializes owner transitions (removals and demotions) for a specific workspace.
    /// Re-entrant within the ambient unit of work so that callers wrapping a unit of work
    /// (such as LeaveAsync or UpdateWorkspaceRoleAsync) can hold the lock until commit,
    /// while domain methods (RemoveAsync, ChangeRoleAsync) also safely ensure the lock is held.
    /// </summary>
    public virtual async Task<IAsyncDisposable> LockOwnersAsync(Guid workspaceId)
    {
        var currentUow = UnitOfWorkManager?.Current;
        var itemKey = OwnerLockItemKey(workspaceId);

        if (currentUow != null && currentUow.Items.ContainsKey(itemKey))
        {
            return new ReentrantLockHandle(() => ValueTask.CompletedTask);
        }

        var handle = await _distributedLock.TryAcquireAsync(OwnerLockName(workspaceId), OwnerLockTimeout);
        if (handle == null)
        {
            throw new AbpException(
                "Another change of this workspace's ownership is still being processed; try again in a moment.");
        }

        if (currentUow != null)
        {
            currentUow.Items[itemKey] = handle;
        }

        return new ReentrantLockHandle(async () =>
        {
            try
            {
                await handle.DisposeAsync();
            }
            finally
            {
                if (currentUow != null)
                {
                    currentUow.Items.Remove(itemKey);
                }
            }
        });
    }

    private sealed class ReentrantLockHandle : IAsyncDisposable
    {
        private readonly Func<ValueTask> _onDispose;
        private int _disposed;

        public ReentrantLockHandle(Func<ValueTask> onDispose)
        {
            _onDispose = onDispose;
        }

        public async ValueTask DisposeAsync()
        {
            if (Interlocked.Exchange(ref _disposed, 1) == 0)
            {
                await _onDispose();
            }
        }
    }

    public virtual Task<WorkspaceMember?> FindAsync(Guid workspaceId, Guid userId)
    {
        return _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == workspaceId && m.UserId == userId);
    }

    /// <summary>The user's active membership of the workspace; null when there is none or the workspace was deleted
    /// (its memberships are kept until its data is purged, but grant nothing).</summary>
    public virtual async Task<WorkspaceMember?> FindActiveAsync(Guid workspaceId, Guid userId)
    {
        var member = await FindAsync(workspaceId, userId);
        if (member is not { IsActive: true })
        {
            return null;
        }

        var workspaces = LazyServiceProvider.LazyGetRequiredService<IRepository<Workspace, Guid>>();
        return await workspaces.AnyAsync(w => w.Id == workspaceId) ? member : null;
    }

    public virtual async Task<bool> IsActiveMemberAsync(Guid workspaceId, Guid userId)
    {
        return await FindActiveAsync(workspaceId, userId) != null;
    }

    /// <summary>Makes the user an active member with <paramref name="role"/>: a new membership, a removed one renewed,
    /// or an active one raised to the role when it is higher (a role is never lowered here).</summary>
    public virtual async Task<WorkspaceMember> AddAsync(Guid workspaceId, Guid userId, string role)
    {
        var member = await FindAsync(workspaceId, userId);
        if (member == null)
        {
            member = new WorkspaceMember(GuidGenerator.Create(), workspaceId, userId, role, Clock.Now);
            await _memberRepository.InsertAsync(member, autoSave: true);
            return member;
        }

        if (!member.IsActive)
        {
            member.Rejoin(role, Clock.Now);
        }
        else if (Rank(role) > Rank(member.Role))
        {
            member.SetRole(role);
        }
        else
        {
            return member;
        }

        await _memberRepository.UpdateAsync(member, autoSave: true);
        return member;
    }

    public virtual async Task ChangeRoleAsync(WorkspaceMember member, string role)
    {
        if (member.Role == role)
        {
            return;
        }

        var isOwnerDemotion = member.Role == WorkspaceRoles.Owner && role != WorkspaceRoles.Owner;
        if (isOwnerDemotion)
        {
            await using var ownerLock = await LockOwnersAsync(member.WorkspaceId);

            if (await CountOwnersAsync(member.WorkspaceId) <= 1)
            {
                throw new BusinessException(TrackerDomainErrorCodes.LastOwnerCannotLeave);
            }

            member.SetRole(role);
            await _memberRepository.UpdateAsync(member, autoSave: true);
            return;
        }

        member.SetRole(role);
        await _memberRepository.UpdateAsync(member, autoSave: true);
    }

    /// <summary>Ends the membership. The last owner cannot be removed.</summary>
    public virtual async Task RemoveAsync(WorkspaceMember member)
    {
        if (member.IsOwner)
        {
            await using var ownerLock = await LockOwnersAsync(member.WorkspaceId);

            if (await CountOwnersAsync(member.WorkspaceId) <= 1)
            {
                throw new BusinessException(TrackerDomainErrorCodes.LastOwnerCannotLeave);
            }

            member.Remove();
            await _memberRepository.UpdateAsync(member, autoSave: true);
            return;
        }

        member.Remove();
        await _memberRepository.UpdateAsync(member, autoSave: true);
    }

    public virtual async Task<int> CountOwnersAsync(Guid workspaceId)
    {
        return await _memberRepository.CountAsync(m =>
            m.WorkspaceId == workspaceId && m.Role == WorkspaceRoles.Owner && m.Status == WorkspaceMemberStatus.Active);
    }

    public static int Rank(string? role) => role switch
    {
        WorkspaceRoles.Owner => 3,
        WorkspaceRoles.Manager => 2,
        WorkspaceRoles.Member => 1,
        _ => 0
    };
}
