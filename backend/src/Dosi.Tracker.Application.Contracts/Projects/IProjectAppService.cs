using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Projects;

public interface IProjectAppService :
    ICrudAppService<
        ProjectDto,
        Guid,
        PagedAndSortedResultRequestDto,
        CreateUpdateProjectDto>
{
    /// <summary>
    /// GET /api/app/project/my-projects — the projects the current user tracks time on: active (non-archived) projects
    /// where they are a member with a role other than Client, with the capture and idle policy the desktop agents must
    /// honor. Agents refresh it every 5 minutes and after a 403/409 from an upload.
    /// </summary>
    Task<List<MyProjectDto>> GetMyProjectsAsync();

    Task<ProjectDto> ArchiveAsync(Guid id);

    Task<ProjectDto> UnarchiveAsync(Guid id);
}
