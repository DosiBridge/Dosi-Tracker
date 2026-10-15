using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Workspaces;
using Volo.Abp.Domain.Repositories;

namespace Dosi.Tracker.Identity;

/// <summary>
/// Keeps the workspace role in line with project memberships: an Admin/Manager project membership makes a member a
/// workspace manager, and the manager role is taken away again once they hold no such membership on a project that
/// still exists. An owner is never demoted (nor promoted) here.
/// </summary>
public class ManagerRoleSynchronizer : TrackerDomainService
{
    private readonly IRepository<WorkspaceMember, Guid> _workspaceMemberRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;

    public ManagerRoleSynchronizer(
        IRepository<WorkspaceMember, Guid> workspaceMemberRepository,
        IRepository<ProjectMember, Guid> memberRepository,
        IRepository<Project, Guid> projectRepository)
    {
        _workspaceMemberRepository = workspaceMemberRepository;
        _memberRepository = memberRepository;
        _projectRepository = projectRepository;
    }

    /// <summary>Makes the user a manager of the current workspace (an active plain member only).</summary>
    public virtual async Task GrantAsync(Guid userId)
    {
        var member = await FindActiveAsync(userId);
        if (member == null || member.Role != WorkspaceRoles.Member)
        {
            return;
        }

        member.SetRole(WorkspaceRoles.Manager);
        await _workspaceMemberRepository.UpdateAsync(member, autoSave: true);
    }

    /// <summary>Makes a manager a plain member again unless they still hold an Admin/Manager project membership.
    /// Call it after the membership change has been saved: the check reads the database.</summary>
    public virtual async Task RevokeIfUnusedAsync(Guid userId)
    {
        var member = await FindActiveAsync(userId);
        if (member == null || member.Role != WorkspaceRoles.Manager || await HoldsManagerMembershipAsync(userId))
        {
            return;
        }

        member.SetRole(WorkspaceRoles.Member);
        await _workspaceMemberRepository.UpdateAsync(member, autoSave: true);
    }

    /// <summary>Whether the user is Admin/Manager on at least one project of the current workspace that still exists.</summary>
    public virtual async Task<bool> HoldsManagerMembershipAsync(Guid userId)
    {
        var memberQuery = await _memberRepository.GetQueryableAsync();
        var projectQuery = await _projectRepository.GetQueryableAsync();

        var roles = await AsyncExecuter.ToListAsync(
            from member in memberQuery
            join project in projectQuery on member.ProjectId equals project.Id
            where member.UserId == userId
            select member.Role);

        return roles.Any(TrackerRoles.IsManagerProjectRole);
    }

    private async Task<WorkspaceMember?> FindActiveAsync(Guid userId)
    {
        var workspaceId = CurrentWorkspace.Id;
        if (workspaceId == null)
        {
            return null;
        }

        return await _workspaceMemberRepository.FirstOrDefaultAsync(m =>
            m.WorkspaceId == workspaceId && m.UserId == userId && m.Status == WorkspaceMemberStatus.Active);
    }
}
