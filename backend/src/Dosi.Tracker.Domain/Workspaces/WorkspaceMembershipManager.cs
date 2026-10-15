using System;
using System.Linq;
using System.Threading.Tasks;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;

namespace Dosi.Tracker.Workspaces;

/// <summary>Adds people to workspaces, changes their role and removes them. Removal only ends the membership: the
/// person's account (global) is never touched.</summary>
public class WorkspaceMembershipManager : TrackerDomainService
{
    private readonly IRepository<WorkspaceMember, Guid> _memberRepository;

    public WorkspaceMembershipManager(IRepository<WorkspaceMember, Guid> memberRepository)
    {
        _memberRepository = memberRepository;
    }

    public virtual Task<WorkspaceMember?> FindAsync(Guid workspaceId, Guid userId)
    {
        return _memberRepository.FirstOrDefaultAsync(m => m.WorkspaceId == workspaceId && m.UserId == userId);
    }

    public virtual async Task<WorkspaceMember?> FindActiveAsync(Guid workspaceId, Guid userId)
    {
        var member = await FindAsync(workspaceId, userId);
        return member is { IsActive: true } ? member : null;
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

        if (member.Role == WorkspaceRoles.Owner && await CountOwnersAsync(member.WorkspaceId) <= 1)
        {
            throw new BusinessException(TrackerDomainErrorCodes.LastOwnerCannotLeave);
        }

        member.SetRole(role);
        await _memberRepository.UpdateAsync(member, autoSave: true);
    }

    /// <summary>Ends the membership. The last owner cannot be removed.</summary>
    public virtual async Task RemoveAsync(WorkspaceMember member)
    {
        if (member.IsOwner && await CountOwnersAsync(member.WorkspaceId) <= 1)
        {
            throw new BusinessException(TrackerDomainErrorCodes.LastOwnerCannotLeave);
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
