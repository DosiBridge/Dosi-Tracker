using System;
using System.Threading.Tasks;
using Dosi.Tracker.Identity;
using Dosi.Tracker.Notifications;
using Dosi.Tracker.Projects;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Workspaces;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;

namespace Dosi.Tracker.Teams;

/// <summary>Brings a person into a workspace (and optionally one of its projects): used when the invitee accepts an
/// invitation, and when an existing member of the workspace is invited to one of its projects. Never called for
/// someone who has not accepted. Runs in the target workspace; a caller that may take a seat holds
/// <see cref="SeatCounter.LockAsync"/>.</summary>
public class WorkspaceJoiner : TrackerDomainService
{
    private readonly WorkspaceMembershipManager _workspaceMemberships;
    private readonly ProjectMembershipManager _projectMemberships;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _projectMemberRepository;
    private readonly IRepository<Notification, Guid> _notificationRepository;
    private readonly SeatCounter _seatCounter;

    public WorkspaceJoiner(
        WorkspaceMembershipManager workspaceMemberships,
        ProjectMembershipManager projectMemberships,
        IRepository<Project, Guid> projectRepository,
        IRepository<ProjectMember, Guid> projectMemberRepository,
        IRepository<Notification, Guid> notificationRepository,
        SeatCounter seatCounter)
    {
        _workspaceMemberships = workspaceMemberships;
        _projectMemberships = projectMemberships;
        _projectRepository = projectRepository;
        _projectMemberRepository = projectMemberRepository;
        _notificationRepository = notificationRepository;
        _seatCounter = seatCounter;
    }

    /// <summary>The workspace role a person gets: the requested one, raised to manager by an Admin/Manager project role.</summary>
    public static string EffectiveRole(string workspaceRole, Guid? projectId, string? projectRole)
    {
        return workspaceRole == WorkspaceRoles.Member && projectId.HasValue && TrackerRoles.IsManagerProjectRole(projectRole)
            ? WorkspaceRoles.Manager
            : workspaceRole;
    }

    public virtual async Task<(WorkspaceMember Member, ProjectMember? ProjectMember)> JoinAsync(
        Guid userId,
        string workspaceRole,
        Guid? projectId,
        string? projectRole,
        decimal hourlyRate)
    {
        var workspaceId = CurrentWorkspace.GetId();
        Project? project = null;
        var role = ProjectMemberRoles.Normalize(projectRole ?? ProjectMemberRoles.Worker);
        if (projectId.HasValue)
        {
            project = await _projectRepository.GetAsync(projectId.Value);
            if (await _projectMemberRepository.AnyAsync(m => m.ProjectId == project.Id && m.UserId == userId))
            {
                throw new BusinessException(TrackerDomainErrorCodes.DuplicateProjectMember)
                    .WithData("projectId", project.Id)
                    .WithData("userId", userId);
            }

            if (!ProjectMemberRoles.IsClient(role))
            {
                await _seatCounter.CheckSeatAvailableAsync(userId);
            }
        }

        var member = await _workspaceMemberships.AddAsync(workspaceId, userId, EffectiveRole(workspaceRole, projectId, role));

        ProjectMember? projectMember = null;
        if (project != null)
        {
            projectMember = await _projectMemberships.EnrollAsync(project.Id, userId, role, hourlyRate);
            await _notificationRepository.InsertAsync(new Notification(
                GuidGenerator.Create(), workspaceId, userId, $"You were added to project '{project.Title}'."));
        }

        return (member, projectMember);
    }
}
