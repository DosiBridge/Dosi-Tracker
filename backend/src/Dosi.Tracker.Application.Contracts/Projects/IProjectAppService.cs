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
    /// Active (non-archived) projects the current user is a member of,
    /// with the capture permissions the desktop agents must honor.
    /// </summary>
    Task<List<MyProjectDto>> GetMyProjectsAsync();

    Task<ProjectDto> ArchiveAsync(Guid id);

    Task<ProjectDto> UnarchiveAsync(Guid id);
}
