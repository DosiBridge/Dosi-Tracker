using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Teams;
using Dosi.Tracker.Workspaces;
using Microsoft.AspNetCore.Authorization;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Users;
using Volo.Abp.Validation;

namespace Dosi.Tracker.Projects;

[Authorize]
[Volo.Abp.RemoteService(IsEnabled = false)]
public class ProjectTaskAppService : TrackerAppService, IProjectTaskAppService
{
    private readonly IRepository<ProjectTask, Guid> _taskRepository;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;

    public ProjectTaskAppService(
        IRepository<ProjectTask, Guid> taskRepository,
        IRepository<Project, Guid> projectRepository,
        IRepository<ProjectMember, Guid> memberRepository)
    {
        _taskRepository = taskRepository;
        _projectRepository = projectRepository;
        _memberRepository = memberRepository;
    }

    public async Task<ProjectTaskDto> GetAsync(Guid id)
    {
        var task = await _taskRepository.FindAsync(id);
        if (task == null)
        {
            throw new EntityNotFoundException(typeof(ProjectTask), id);
        }

        var project = await _projectRepository.FindAsync(task.ProjectId);
        if (project == null)
        {
            throw new EntityNotFoundException(typeof(ProjectTask), id);
        }

        await CheckCanReadProjectAsync(project);

        return ToDto(task);
    }

    public async Task<PagedResultDto<ProjectTaskDto>> GetListAsync(GetProjectTasksInput input)
    {
        if (input.ProjectId == Guid.Empty)
        {
            throw new AbpValidationException(new List<ValidationResult>
            {
                new("ProjectId is required.", new[] { nameof(GetProjectTasksInput.ProjectId) })
            });
        }

        var project = await _projectRepository.FindAsync(input.ProjectId);
        if (project == null)
        {
            throw new EntityNotFoundException(typeof(Project), input.ProjectId);
        }

        await CheckCanReadProjectAsync(project);

        var query = (await _taskRepository.GetQueryableAsync())
            .Where(t => t.ProjectId == input.ProjectId);

        if (input.IsCompleted.HasValue)
        {
            query = query.Where(t => t.IsCompleted == input.IsCompleted.Value);
        }

        var totalCount = await AsyncExecuter.CountAsync(query);

        var pageSize = input.MaxResultCount <= 0
            ? ProjectTaskConsts.DefaultPageSize
            : Math.Min(input.MaxResultCount, ProjectTaskConsts.MaxPageSize);
        var skipCount = Math.Max(0, input.SkipCount);

        var items = await AsyncExecuter.ToListAsync(
            query.OrderBy(t => t.IsCompleted)
                 .ThenByDescending(t => t.CreationTime)
                 .ThenBy(t => t.Id)
                 .Skip(skipCount)
                 .Take(pageSize));

        return new PagedResultDto<ProjectTaskDto>(
            totalCount,
            items.Select(ToDto).ToList());
    }

    public async Task<ProjectTaskDto> CreateAsync(CreateProjectTaskDto input)
    {
        ValidateInput(input.Title, input.Description);

        if (input.ProjectId == Guid.Empty)
        {
            throw new AbpValidationException(new List<ValidationResult>
            {
                new("ProjectId is required.", new[] { nameof(CreateProjectTaskDto.ProjectId) })
            });
        }

        var project = await _projectRepository.FindAsync(input.ProjectId);
        if (project == null)
        {
            throw new EntityNotFoundException(typeof(Project), input.ProjectId);
        }

        await CheckCanMutateProjectTasksAsync(project);

        var workspaceId = CurrentWorkspace.GetId();
        var title = input.Title.Trim();
        var description = string.IsNullOrWhiteSpace(input.Description) ? null : input.Description.Trim();
        var isCompleted = input.IsCompleted;
        var completedAt = isCompleted ? Clock.Now : (DateTime?)null;

        var task = new ProjectTask(
            GuidGenerator.Create(),
            workspaceId,
            input.ProjectId,
            title,
            description,
            isCompleted,
            completedAt);

        await _taskRepository.InsertAsync(task, autoSave: true);

        return ToDto(task);
    }

    public async Task<ProjectTaskDto> UpdateAsync(Guid id, UpdateProjectTaskDto input)
    {
        ValidateInput(input.Title, input.Description);

        var task = await _taskRepository.FindAsync(id);
        if (task == null)
        {
            throw new EntityNotFoundException(typeof(ProjectTask), id);
        }

        var project = await _projectRepository.FindAsync(task.ProjectId);
        if (project == null)
        {
            throw new EntityNotFoundException(typeof(Project), task.ProjectId);
        }

        await CheckCanMutateProjectTasksAsync(project);

        task.SetTitle(input.Title.Trim());
        task.SetDescription(string.IsNullOrWhiteSpace(input.Description) ? null : input.Description.Trim());

        if (input.IsCompleted && !task.IsCompleted)
        {
            task.Complete(Clock.Now);
        }
        else if (!input.IsCompleted && task.IsCompleted)
        {
            task.Reopen();
        }

        await _taskRepository.UpdateAsync(task, autoSave: true);

        return ToDto(task);
    }

    private async Task CheckCanReadProjectAsync(Project project)
    {
        if (await AuthorizationService.IsGrantedAsync(TrackerPermissions.Projects.Edit) ||
            await AuthorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll))
        {
            return;
        }

        var userId = CurrentUser.GetId();
        var isMember = await _memberRepository.AnyAsync(m => m.ProjectId == project.Id && m.UserId == userId);
        if (!isMember)
        {
            throw new EntityNotFoundException(typeof(Project), project.Id);
        }
    }

    private async Task CheckCanMutateProjectTasksAsync(Project project)
    {
        if (project.IsArchived)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ProjectArchived);
        }

        if (await AuthorizationService.IsGrantedAsync(TrackerPermissions.Projects.Edit))
        {
            return;
        }

        var userId = CurrentUser.GetId();
        var member = await _memberRepository.FirstOrDefaultAsync(m => m.ProjectId == project.Id && m.UserId == userId);
        if (member == null)
        {
            throw new BusinessException(TrackerDomainErrorCodes.NotProjectMember);
        }

        if (ProjectMemberRoles.IsClient(member.Role))
        {
            throw new BusinessException(TrackerDomainErrorCodes.RoleNotAllowed);
        }
    }

    private static void ValidateInput(string? title, string? description)
    {
        var errors = new List<ValidationResult>();

        if (string.IsNullOrWhiteSpace(title))
        {
            errors.Add(new ValidationResult("The task needs a title.", new[] { "Title" }));
        }
        else if (title.Trim().Length > ProjectTaskConsts.MaxTitleLength)
        {
            errors.Add(new ValidationResult($"The title cannot exceed {ProjectTaskConsts.MaxTitleLength} characters.", new[] { "Title" }));
        }

        if (description != null && description.Length > ProjectTaskConsts.MaxDescriptionLength)
        {
            errors.Add(new ValidationResult($"The description cannot exceed {ProjectTaskConsts.MaxDescriptionLength} characters.", new[] { "Description" }));
        }

        if (errors.Count > 0)
        {
            throw new AbpValidationException(errors);
        }
    }

    private static ProjectTaskDto ToDto(ProjectTask entity)
    {
        return new ProjectTaskDto
        {
            Id = entity.Id,
            WorkspaceId = entity.WorkspaceId,
            ProjectId = entity.ProjectId,
            Title = entity.Title,
            Description = entity.Description,
            IsCompleted = entity.IsCompleted,
            CompletedAt = entity.CompletedAt,
            CreationTime = entity.CreationTime,
            CreatorId = entity.CreatorId,
            LastModificationTime = entity.LastModificationTime,
            LastModifierId = entity.LastModifierId
        };
    }
}
