using System;
using System.Threading.Tasks;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Teams;

/// <summary>
/// The current workspace's team. Workspace owners are the only ones who may grant Admin/Manager project roles, set or
/// change hourly rates, change workspace roles or manage an owner (403 Tracker:RoleNotAllowed); nobody may change their
/// own role or rate (403 Tracker:CannotModifySelf).
/// </summary>
public interface ITeamAppService : IApplicationService
{
    /// <summary>GET /api/app/team/project-members/{projectId}.</summary>
    Task<ListResultDto<ProjectMemberDto>> GetProjectMembersAsync(Guid projectId);

    /// <summary>POST /api/app/team/member — adds an active member of the workspace to a project.</summary>
    Task<ProjectMemberDto> AddMemberAsync(CreateProjectMemberDto input);

    /// <summary>POST /api/app/team/invite-member.</summary>
    Task<InviteMemberResultDto> InviteMemberAsync(InviteMemberDto input);

    /// <summary>DELETE /api/app/team/member/{memberId} — removes one project membership.</summary>
    Task RemoveMemberAsync(Guid memberId);

    /// <summary>GET /api/app/team/members — the workspace's active members (callers without Team.Manage or
    /// Activities.ViewAll get only themselves).</summary>
    Task<ListResultDto<TeamMemberDto>> GetMembersAsync();

    /// <summary>PUT /api/app/team/member?projectId=&amp;userId=.</summary>
    Task<ProjectMemberDto> UpdateMemberAsync(Guid projectId, Guid userId, UpdateProjectMemberDto input);

    /// <summary>PUT /api/app/team/workspace-role?userId=&amp;role= — owners only.</summary>
    Task<TeamMemberDto> UpdateWorkspaceRoleAsync(Guid userId, string role);

    /// <summary>DELETE /api/app/team/from-workspace?userId= — ends the person's membership of the workspace and removes
    /// their project memberships. Their account is untouched.</summary>
    Task RemoveFromWorkspaceAsync(Guid userId);

    /// <summary>GET /api/app/team/invitations — pending invitations.</summary>
    Task<ListResultDto<WorkspaceInvitationDto>> GetInvitationsAsync();

    /// <summary>DELETE /api/app/team/invitation/{id} — withdraws a pending invitation.</summary>
    Task DeleteInvitationAsync(Guid id);
}
