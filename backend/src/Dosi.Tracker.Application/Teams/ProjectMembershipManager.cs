using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Projects;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Dosi.Tracker.Workspaces;
using Volo.Abp.Uow;

namespace Dosi.Tracker.Teams;

/// <summary>
/// Creates and removes project memberships together with what hangs off them: the hourly-rate history that payroll
/// prices tracked time with, and the workspace manager role that Admin/Manager memberships confer. Shared by the
/// project and team services so every path (project creation, add, invite, update, removal, project deletion,
/// removal from the workspace) keeps them consistent. Callers check permissions and business rules first.
/// </summary>
public class ProjectMembershipManager : TrackerDomainService
{
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<ProjectMemberRate, Guid> _rateRepository;
    private readonly ManagerRoleSynchronizer _managerRoles;
    private readonly IUnitOfWorkManager _unitOfWorkManager;

    public ProjectMembershipManager(
        IRepository<ProjectMember, Guid> memberRepository,
        IRepository<ProjectMemberRate, Guid> rateRepository,
        ManagerRoleSynchronizer managerRoles,
        IUnitOfWorkManager unitOfWorkManager)
    {
        _memberRepository = memberRepository;
        _rateRepository = rateRepository;
        _managerRoles = managerRoles;
        _unitOfWorkManager = unitOfWorkManager;
    }

    /// <summary>Adds the membership, records its rate and, for Admin/Manager, grants the Identity "manager" role
    /// (unless <paramref name="grantManagerRole"/> is false: a project's creator already manages projects).</summary>
    public virtual async Task<ProjectMember> EnrollAsync(
        Guid projectId,
        Guid userId,
        string role,
        decimal hourlyRate,
        bool grantManagerRole = true)
    {
        var member = new ProjectMember(GuidGenerator.Create(), CurrentWorkspace.GetId(), projectId, userId, role, hourlyRate);
        await InsertMembershipAsync(member);
        await RecordRateAsync(projectId, userId, hourlyRate);

        if (grantManagerRole && TrackerRoles.IsManagerProjectRole(role))
        {
            await _managerRoles.GrantAsync(userId);
        }

        return member;
    }

    /// <summary>
    /// Changes a membership's role and rate. A new rate applies from now on (time tracked earlier keeps the earlier
    /// rate), and the "manager" role follows the new role.
    /// </summary>
    public virtual async Task ChangeAsync(ProjectMember member, string role, decimal hourlyRate)
    {
        var wasManagerRole = TrackerRoles.IsManagerProjectRole(member.Role);
        var rateChanged = member.HourlyRate != hourlyRate;

        member.Role = role;
        member.HourlyRate = hourlyRate;
        await _memberRepository.UpdateAsync(member, autoSave: true);

        if (rateChanged)
        {
            await RecordRateAsync(member.ProjectId, member.UserId, hourlyRate);
        }

        if (TrackerRoles.IsManagerProjectRole(role))
        {
            await _managerRoles.GrantAsync(member.UserId);
        }
        else if (wasManagerRole)
        {
            await _managerRoles.RevokeIfUnusedAsync(member.UserId);
        }
    }

    /// <summary>
    /// Appends a rate-history entry effective now, unless the member's latest entry on the project already has this rate
    /// (for example when someone is re-added at their old rate). Entries are never changed or deleted.
    /// </summary>
    public virtual async Task RecordRateAsync(Guid projectId, Guid userId, decimal hourlyRate)
    {
        if (await FindLatestRateAsync(projectId, userId) == hourlyRate)
        {
            return;
        }

        await _rateRepository.InsertAsync(
            new ProjectMemberRate(GuidGenerator.Create(), CurrentWorkspace.GetId(), projectId, userId, hourlyRate, Clock.Now),
            autoSave: true);
    }

    /// <summary>The member's latest recorded rate on the project — the one payroll prices new time with, kept after the
    /// member left the project — or null when they never had one.</summary>
    public virtual async Task<decimal?> FindLatestRateAsync(Guid projectId, Guid userId)
    {
        var rates = await _rateRepository.GetQueryableAsync();
        return await AsyncExecuter.FirstOrDefaultAsync(rates
            .Where(r => r.ProjectId == projectId && r.UserId == userId)
            .OrderByDescending(r => r.EffectiveFrom)
            .ThenByDescending(r => r.CreationTime)
            .Select(r => (decimal?)r.HourlyRate));
    }

    /// <summary>Removes one membership; the "manager" role goes with the user's last Admin/Manager membership.</summary>
    public virtual Task RemoveAsync(ProjectMember member)
    {
        return RemoveManyAsync(new List<ProjectMember> { member });
    }

    /// <summary>Removes every membership of a project (it is being deleted). Returns how many were removed.</summary>
    public virtual async Task<int> RemoveAllOfProjectAsync(Guid projectId)
    {
        var memberships = await _memberRepository.GetListAsync(m => m.ProjectId == projectId);
        await RemoveManyAsync(memberships);
        return memberships.Count;
    }

    /// <summary>Removes every membership of a user (they leave the workspace), and with them the "manager" role however
    /// it was given — through a membership, or directly in Identity: someone who left keeps no workspace-wide access
    /// that a later re-invite would bring back. Returns how many memberships were removed.</summary>
    public virtual async Task<int> RemoveAllOfUserAsync(Guid userId)
    {
        var memberships = await _memberRepository.GetListAsync(m => m.UserId == userId);
        if (memberships.Count > 0)
        {
            await _memberRepository.DeleteManyAsync(memberships, autoSave: true);
        }

        // No membership is left, so this always takes the role away.
        await _managerRoles.RevokeIfUnusedAsync(userId);
        return memberships.Count;
    }

    /// <summary>
    /// Saves a new membership. When two adds of the same person race past the callers' duplicate check, the unique
    /// (workspace, project, user) index lets the first one in and refuses the other, which is then answered like any
    /// duplicate (<c>Tracker:DuplicateProjectMember</c>, 409) instead of a server error.
    /// </summary>
    private async Task InsertMembershipAsync(ProjectMember member)
    {
        try
        {
            await _memberRepository.InsertAsync(member, autoSave: true);
        }
        catch (Exception exception) when (DatabaseErrors.MayBeUniqueViolation(exception))
        {
            if (!await IsMemberAsync(member.ProjectId, member.UserId))
            {
                throw;
            }

            throw new BusinessException(TrackerDomainErrorCodes.DuplicateProjectMember)
                .WithData("projectId", member.ProjectId)
                .WithData("userId", member.UserId);
        }
    }

    /// <summary>Whether the membership exists, read in a unit of work of its own: a failed insert leaves the current
    /// one's database context (and transaction) unusable.</summary>
    private async Task<bool> IsMemberAsync(Guid projectId, Guid userId)
    {
        using var uow = _unitOfWorkManager.Begin(requiresNew: true);
        var isMember = await _memberRepository.AnyAsync(m => m.ProjectId == projectId && m.UserId == userId);
        await uow.CompleteAsync();
        return isMember;
    }

    private async Task RemoveManyAsync(List<ProjectMember> memberships)
    {
        if (memberships.Count == 0)
        {
            return;
        }

        // Saved before the role check below, which reads the database.
        await _memberRepository.DeleteManyAsync(memberships, autoSave: true);

        var formerManagers = memberships
            .Where(m => TrackerRoles.IsManagerProjectRole(m.Role))
            .Select(m => m.UserId)
            .Distinct();

        foreach (var userId in formerManagers)
        {
            await _managerRoles.RevokeIfUnusedAsync(userId);
        }
    }
}
