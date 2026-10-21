using System;
using System.Threading.Tasks;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Projects;

public interface IProjectTaskAppService : IApplicationService
{
    Task<ProjectTaskDto> GetAsync(Guid id);

    Task<PagedResultDto<ProjectTaskDto>> GetListAsync(GetProjectTasksInput input);

    Task<ProjectTaskDto> CreateAsync(CreateProjectTaskDto input);

    Task<ProjectTaskDto> UpdateAsync(Guid id, UpdateProjectTaskDto input);
}
