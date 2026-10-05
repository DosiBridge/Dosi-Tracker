using System;
using System.Threading.Tasks;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Teams;

/// <summary>
/// The workspace team. Workspace owners (Identity "admin" role) are the only ones who may grant Admin/Manager project
/// roles, set or change hourly rates, or manage an owner's memberships (403 Tracker:RoleNotAllowed); nobody may change
/// their own role or rate (403 Tracker:CannotModifySelf).
/// </summary>
public interface ITeamAppService : IApplicationService
{
    /// <summary>GET /api/app/team/project-members/{projectId} — the project's roster with names. Members without
    /// Tracker.Team.Manage may only read projects they belong to, and never see emails or rates.</summary>
    Task<ListResultDto<ProjectMemberDto>> GetProjectMembersAsync(Guid projectId);

    /// <summary>POST /api/app/team/member — adds an active user of the workspace (404 Tracker:InvalidUser otherwise) and
    /// records their first hourly rate.</summary>
    Task<ProjectMemberDto> AddMemberAsync(CreateProjectMemberDto input);

    /// <summary>POST /api/app/team/invite-member — adds the person with this email, creating their account (with a
    /// one-time password they must change at the first sign-in) when they have none.</summary>
    Task<InviteMemberResultDto> InviteMemberAsync(InviteMemberDto input);

    /// <summary>DELETE /api/app/team/member — removes one membership; the Identity "manager" role goes with the user's
    /// last Admin/Manager membership.</summary>
    Task RemoveMemberAsync(Guid memberId);

    /// <summary>GET /api/app/team/members — the workspace's people. Callers with Tracker.Team.Manage or
    /// Tracker.Activities.ViewAll get every user of the tenant; anyone else gets only themselves.</summary>
    Task<ListResultDto<TeamMemberDto>> GetMembersAsync();

    /// <summary>PUT /api/app/team/member?projectId=&amp;userId= — change a member's project role and/or
    /// hourly rate (Tracker.Team.Manage). Promoting to Admin/Manager grants the Identity "manager" role; a rate change
    /// applies from now on (earlier time keeps the earlier rate).</summary>
    Task<ProjectMemberDto> UpdateMemberAsync(Guid projectId, Guid userId, UpdateProjectMemberDto input);

    /// <summary>POST /api/app/team/reset-password?userId= — gives the member a new one-time password that must be changed at
    /// the next sign-in (Tracker.Team.Manage; an owner's password only by an owner; not your own — use
    /// /api/account/my-profile/change-password).</summary>
    Task<ResetPasswordResultDto> ResetPasswordAsync(Guid userId);

    /// <summary>DELETE /api/app/team/from-workspace?userId= — removes every membership of the user and deactivates the
    /// account, which frees their seat (Tracker.Team.Manage). Cannot target yourself or an owner.</summary>
    Task RemoveFromWorkspaceAsync(Guid userId);
}
