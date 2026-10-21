using System;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Mvc;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Controllers;

[Route("api/app/project-task")]
[Authorize]
public class ProjectTaskController : TrackerController, IProjectTaskAppService
{
    private readonly IProjectTaskAppService _projectTaskAppService;

    public ProjectTaskController(IProjectTaskAppService projectTaskAppService)
    {
        _projectTaskAppService = projectTaskAppService;
    }

    [HttpGet("{id}")]
    public Task<ProjectTaskDto> GetAsync(Guid id)
    {
        return _projectTaskAppService.GetAsync(id);
    }

    [HttpGet]
    public Task<PagedResultDto<ProjectTaskDto>> GetListAsync([FromQuery] GetProjectTasksInput input)
    {
        return _projectTaskAppService.GetListAsync(input);
    }

    [HttpPost]
    public Task<ProjectTaskDto> CreateAsync([FromBody] CreateProjectTaskDto input)
    {
        return _projectTaskAppService.CreateAsync(input);
    }

    [HttpPut("{id}")]
    public Task<ProjectTaskDto> UpdateAsync(Guid id, [FromBody] UpdateProjectTaskDto input)
    {
        return _projectTaskAppService.UpdateAsync(id, input);
    }
}
