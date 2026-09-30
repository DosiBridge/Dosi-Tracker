using System;
using System.Threading.Tasks;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Teams;

public interface ITeamAppService : IApplicationService
{
    Task<ListResultDto<ProjectMemberDto>> GetProjectMembersAsync(Guid projectId);
    Task<ProjectMemberDto> AddMemberAsync(CreateProjectMemberDto input);
    Task<InviteMemberResultDto> InviteMemberAsync(InviteMemberDto input);
    Task RemoveMemberAsync(Guid memberId);

    /// <summary>GET /api/app/team/members — the workspace's people. Callers with Tracker.Team.Manage or
    /// Tracker.Activities.ViewAll get every user of the tenant; anyone else gets only themselves.</summary>
    Task<ListResultDto<TeamMemberDto>> GetMembersAsync();

    /// <summary>PUT /api/app/team/member?projectId=&amp;userId= — change a member's project role and/or
    /// hourly rate (Tracker.Team.Manage). Promoting to Admin/Manager grants the Identity "manager" role.</summary>
    Task<ProjectMemberDto> UpdateMemberAsync(Guid projectId, Guid userId, UpdateProjectMemberDto input);
}
