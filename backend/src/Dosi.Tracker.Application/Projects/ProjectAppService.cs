using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Dosi.Tracker.Permissions;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Users;

namespace Dosi.Tracker.Projects;

[Authorize]
public class ProjectAppService :
    CrudAppService<
        Project,
        ProjectDto,
        Guid,
        PagedAndSortedResultRequestDto,
        CreateUpdateProjectDto>,
    IProjectAppService
{
    private readonly IRepository<ProjectMember, Guid> _memberRepository;

    public ProjectAppService(
        IRepository<Project, Guid> repository,
        IRepository<ProjectMember, Guid> memberRepository)
        : base(repository)
    {
        _memberRepository = memberRepository;

        CreatePolicyName = TrackerPermissions.Projects.Create;
        UpdatePolicyName = TrackerPermissions.Projects.Edit;
        DeletePolicyName = TrackerPermissions.Projects.Delete;
    }

    public override async Task<ProjectDto> CreateAsync(CreateUpdateProjectDto input)
    {
        var dto = await base.CreateAsync(input);

        // The creator manages the project and can immediately track against it.
        await _memberRepository.InsertAsync(new ProjectMember(
            GuidGenerator.Create(),
            CurrentTenant.Id,
            dto.Id,
            CurrentUser.GetId(),
            "Admin"
        ));

        return dto;
    }

    /// <summary>Managers (Projects.Edit or Activities.ViewAll) see every project of the workspace;
    /// anyone else only the projects they are a member of.</summary>
    protected override async Task<IQueryable<Project>> CreateFilteredQueryAsync(PagedAndSortedResultRequestDto input)
    {
        var query = await base.CreateFilteredQueryAsync(input);
        if (await CanSeeAllProjectsAsync())
        {
            return query;
        }

        var userId = CurrentUser.GetId();
        var memberQuery = await _memberRepository.GetQueryableAsync();
        return query.Where(p => memberQuery.Any(m => m.ProjectId == p.Id && m.UserId == userId));
    }

    public override async Task<ProjectDto> GetAsync(Guid id)
    {
        if (!await CanSeeAllProjectsAsync())
        {
            var userId = CurrentUser.GetId();
            if (!await _memberRepository.AnyAsync(m => m.ProjectId == id && m.UserId == userId))
            {
                // Indistinguishable from a missing project: don't reveal other teams' projects exist.
                throw new EntityNotFoundException(typeof(Project), id);
            }
        }

        return await base.GetAsync(id);
    }

    private async Task<bool> CanSeeAllProjectsAsync()
    {
        return await AuthorizationService.IsGrantedAsync(TrackerPermissions.Projects.Edit) ||
               await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll);
    }

    [Authorize] // member-scoped: any authenticated user sees only their own projects
    public async Task<List<MyProjectDto>> GetMyProjectsAsync()
    {
        var userId = CurrentUser.GetId();

        var memberQuery = await _memberRepository.GetQueryableAsync();
        var projectQuery = await Repository.GetQueryableAsync();

        var query =
            from member in memberQuery
            join project in projectQuery on member.ProjectId equals project.Id
            where member.UserId == userId && !project.IsArchived
            orderby project.Title
            select project;

        var projects = await AsyncExecuter.ToListAsync(query);

        return projects.Select(p => new MyProjectDto
        {
            Id = p.Id,
            Title = p.Title,
            IntervalMinutes = p.IntervalMinutes,
            AllowScreenshot = p.AllowScreenshot,
            AllowWebcam = p.AllowWebcam,
            AllowKeyboard = p.AllowKeyboard,
            AllowMouse = p.AllowMouse,
            AllowActiveWindow = p.AllowActiveWindow,
            AllowRunningPrograms = p.AllowRunningPrograms
        }).ToList();
    }

    [Authorize(TrackerPermissions.Projects.Archive)]
    public async Task<ProjectDto> ArchiveAsync(Guid id)
    {
        var project = await Repository.GetAsync(id);
        project.Archive();
        await Repository.UpdateAsync(project);
        return MapToGetOutputDto(project);
    }

    [Authorize(TrackerPermissions.Projects.Archive)]
    public async Task<ProjectDto> UnarchiveAsync(Guid id)
    {
        var project = await Repository.GetAsync(id);
        project.Unarchive();
        await Repository.UpdateAsync(project);
        return MapToGetOutputDto(project);
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
            IsArchived = entity.IsArchived,
            AllowScreenshot = entity.AllowScreenshot,
            AllowWebcam = entity.AllowWebcam,
            AllowKeyboard = entity.AllowKeyboard,
            AllowMouse = entity.AllowMouse,
            AllowActiveWindow = entity.AllowActiveWindow,
            AllowRunningPrograms = entity.AllowRunningPrograms
        };
    }

    protected override Project MapToEntity(CreateUpdateProjectDto createInput)
    {
        return new Project(GuidGenerator.Create(), CurrentTenant.Id, createInput.Title, createInput.IntervalMinutes, createInput.Color)
        {
            Description = createInput.Description,
            AllowScreenshot = createInput.AllowScreenshot,
            AllowWebcam = createInput.AllowWebcam,
            AllowKeyboard = createInput.AllowKeyboard,
            AllowMouse = createInput.AllowMouse,
            AllowActiveWindow = createInput.AllowActiveWindow,
            AllowRunningPrograms = createInput.AllowRunningPrograms
        };
    }

    protected override void MapToEntity(CreateUpdateProjectDto updateInput, Project entity)
    {
        entity.Title = updateInput.Title;
        entity.Description = updateInput.Description;
        entity.Color = updateInput.Color;
        entity.IntervalMinutes = updateInput.IntervalMinutes;
        entity.AllowScreenshot = updateInput.AllowScreenshot;
        entity.AllowWebcam = updateInput.AllowWebcam;
        entity.AllowKeyboard = updateInput.AllowKeyboard;
        entity.AllowMouse = updateInput.AllowMouse;
        entity.AllowActiveWindow = updateInput.AllowActiveWindow;
        entity.AllowRunningPrograms = updateInput.AllowRunningPrograms;
    }
}
