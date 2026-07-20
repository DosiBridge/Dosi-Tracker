using System;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;
using Volo.Abp.Domain.Repositories;

namespace Dosi.Tracker.Projects;

public class ProjectAppService : 
    CrudAppService<
        Project,
        ProjectDto,
        Guid,
        PagedAndSortedResultRequestDto,
        CreateUpdateProjectDto>,
    IProjectAppService
{
    public ProjectAppService(IRepository<Project, Guid> repository) 
        : base(repository)
    {
    }

    protected override ProjectDto MapToGetOutputDto(Project entity)
    {
        return new ProjectDto
        {
            Id = entity.Id,
            Title = entity.Title,
            Description = entity.Description,
            Color = entity.Color,
            IntervalMinutes = entity.IntervalMinutes,
            IsArchived = entity.IsArchived
        };
    }

    protected override Project MapToEntity(CreateUpdateProjectDto createInput)
    {
        return new Project(GuidGenerator.Create(), CurrentTenant.Id, createInput.Title, createInput.IntervalMinutes, createInput.Color)
        {
            Description = createInput.Description
        };
    }

    protected override void MapToEntity(CreateUpdateProjectDto updateInput, Project entity)
    {
        entity.Title = updateInput.Title;
        entity.Description = updateInput.Description;
        entity.Color = updateInput.Color;
        entity.IntervalMinutes = updateInput.IntervalMinutes;
    }
}
