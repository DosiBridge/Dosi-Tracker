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
}
