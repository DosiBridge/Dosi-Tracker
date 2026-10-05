using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Microsoft.AspNetCore.Identity;
using Microsoft.Extensions.Logging;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Domain.Services;
using Volo.Abp.Identity;

namespace Dosi.Tracker.Identity;

/// <summary>
/// Keeps the workspace-wide Identity <see cref="TrackerRoles.Manager"/> role in line with project memberships: an
/// Admin/Manager membership confers it, and it is taken away again once the user holds no such membership on a
/// project that still exists. Without the revocation a removed team lead would keep seeing every member's activity,
/// screenshots and webcam frames.
/// </summary>
public class ManagerRoleSynchronizer : DomainService
{
    private readonly ManagerRoleProvisioner _provisioner;
    private readonly IdentityUserManager _userManager;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<Project, Guid> _projectRepository;

    public ManagerRoleSynchronizer(
        ManagerRoleProvisioner provisioner,
        IdentityUserManager userManager,
        IRepository<ProjectMember, Guid> memberRepository,
        IRepository<Project, Guid> projectRepository)
    {
        _provisioner = provisioner;
        _userManager = userManager;
        _memberRepository = memberRepository;
        _projectRepository = projectRepository;
    }

    /// <summary>Gives the user the "manager" role (created on demand for workspaces that predate it). A membership for a
    /// user id that has no Identity account is left as a plain membership.</summary>
    public virtual async Task GrantAsync(Guid userId)
    {
        var user = await _userManager.FindByIdAsync(userId.ToString());
        if (user == null)
        {
            Logger.LogWarning("Cannot grant the '{Role}' role: user {UserId} does not exist.", TrackerRoles.Manager, userId);
            return;
        }

        await _provisioner.EnsureAsync();

        if (!await _userManager.IsInRoleAsync(user, TrackerRoles.Manager))
        {
            (await _userManager.AddToRoleAsync(user, TrackerRoles.Manager)).CheckErrors();
        }
    }

    /// <summary>Drops the "manager" role unless the user is still Admin/Manager on a project that has not been deleted.
    /// Call it after the membership change has been saved: the check reads the database.</summary>
    public virtual async Task RevokeIfUnusedAsync(Guid userId)
    {
        if (await HoldsManagerMembershipAsync(userId))
        {
            return;
        }

        var user = await _userManager.FindByIdAsync(userId.ToString());
        if (user != null && await _userManager.IsInRoleAsync(user, TrackerRoles.Manager))
        {
            (await _userManager.RemoveFromRoleAsync(user, TrackerRoles.Manager)).CheckErrors();
        }
    }

    /// <summary>Whether the user is Admin/Manager on at least one project that still exists (soft-deleted projects, and
    /// memberships left behind by them, do not count).</summary>
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
}
