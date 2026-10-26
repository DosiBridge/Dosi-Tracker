using System;
using System.Threading.Tasks;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Teams;
using Microsoft.AspNetCore.Authorization;
using Volo.Abp;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Users;

namespace Dosi.Tracker.Projects;

/// <summary>
/// Who may read and change a project's tasks, shared by the task endpoints and by AI task drafts, so drafts are only
/// made for someone who could save them.
/// </summary>
public class ProjectTaskRules : ITransientDependency
{
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IAuthorizationService _authorizationService;
    private readonly ICurrentUser _currentUser;

    public ProjectTaskRules(
        IRepository<ProjectMember, Guid> memberRepository,
        IAuthorizationService authorizationService,
        ICurrentUser currentUser)
    {
        _memberRepository = memberRepository;
        _authorizationService = authorizationService;
        _currentUser = currentUser;
    }

    /// <summary>Project editors (Tracker.Projects.Edit) and team viewers (Tracker.Activities.ViewAll) read every
    /// project's tasks; anyone else only those of projects they are a member of, in any role. Others get 404, so a
    /// project's existence is not revealed.</summary>
    public virtual async Task EnsureCanReadProjectAsync(Project project)
    {
        if (await _authorizationService.IsGrantedAsync(TrackerPermissions.Projects.Edit) ||
            await _authorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll))
        {
            return;
        }

        var userId = _currentUser.GetId();
        var isMember = await _memberRepository.AnyAsync(m => m.ProjectId == project.Id && m.UserId == userId);
        if (!isMember)
        {
            throw new EntityNotFoundException(typeof(Project), project.Id);
        }
    }

    /// <summary>An archived project's tasks are frozen (<c>Tracker:ProjectArchived</c>). Otherwise project editors may
    /// change any project's tasks, and other members of the project those of that project, except its clients
    /// (<c>Tracker:RoleNotAllowed</c>); anyone else is refused as not a member (<c>Tracker:NotProjectMember</c>).</summary>
    public virtual async Task EnsureCanMutateProjectTasksAsync(Project project)
    {
        if (project.IsArchived)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ProjectArchived);
        }

        if (await _authorizationService.IsGrantedAsync(TrackerPermissions.Projects.Edit))
        {
            return;
        }

        var userId = _currentUser.GetId();
        var member = await _memberRepository.FirstOrDefaultAsync(m => m.ProjectId == project.Id && m.UserId == userId);
        if (member == null)
        {
            throw new BusinessException(TrackerDomainErrorCodes.NotProjectMember);
        }

        if (ProjectMemberRoles.IsClient(member.Role))
        {
            throw new BusinessException(TrackerDomainErrorCodes.RoleNotAllowed);
        }
    }
}
