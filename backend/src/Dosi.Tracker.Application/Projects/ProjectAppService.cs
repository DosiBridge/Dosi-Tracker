using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Linq;
using System.Text.RegularExpressions;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Authorization;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.SaaS;
using Dosi.Tracker.Teams;
using Volo.Abp;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Users;
using Volo.Abp.Validation;

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
    /// <summary>Longest project description (same limit as <see cref="CreateUpdateProjectDto.Description"/> and the column).</summary>
    public const int MaxDescriptionLength = ProjectConsts.MaxDescriptionLength;

    /// <summary>A project colour is a hex colour (#RRGGBB) only: the web app renders it as a style on project pages.</summary>
    private static readonly Regex HexColor = new(ProjectConsts.ColorPattern, RegexOptions.CultureInvariant);

    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly ProjectMembershipManager _memberships;
    private readonly ISubscriptionAccessChecker _subscriptionAccess;

    public ProjectAppService(
        IRepository<Project, Guid> repository,
        IRepository<ProjectMember, Guid> memberRepository,
        ProjectMembershipManager memberships,
        ISubscriptionAccessChecker subscriptionAccess)
        : base(repository)
    {
        _memberRepository = memberRepository;
        _memberships = memberships;
        _subscriptionAccess = subscriptionAccess;

        CreatePolicyName = TrackerPermissions.Projects.Create;
        UpdatePolicyName = TrackerPermissions.Projects.Edit;
        DeletePolicyName = TrackerPermissions.Projects.Delete;
    }

    public override async Task<ProjectDto> CreateAsync(CreateUpdateProjectDto input)
    {
        await CheckCreatePolicyAsync();

        // A workspace whose subscription lapsed keeps its data readable but cannot grow.
        if (!await _subscriptionAccess.IsUsableAsync())
        {
            throw new BusinessException(TrackerDomainErrorCodes.SubscriptionInactive);
        }

        var dto = await base.CreateAsync(input);

        // The creator manages the project and can immediately track against it. They already hold project-management
        // rights (they could create it), so no Identity role is granted here.
        await _memberships.EnrollAsync(dto.Id, CurrentUser.GetId(), ProjectMemberRoles.Admin, 0m, grantManagerRole: false);

        return dto;
    }

    /// <summary>
    /// Deletes the project together with its memberships: they would otherwise keep occupying (and billing) seats, and
    /// the Identity "manager" role of a member whose last Admin/Manager membership was on this project is revoked.
    /// The members' rate history stays for payroll of the time already tracked.
    /// </summary>
    public override async Task DeleteAsync(Guid id)
    {
        await CheckDeletePolicyAsync();

        var project = await Repository.FindAsync(id);
        if (project != null)
        {
            await Repository.DeleteAsync(project, autoSave: true);
        }

        // Also when the project is already gone: clears memberships an earlier deletion left behind.
        await _memberships.RemoveAllOfProjectAsync(id);
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

    /// <summary>The projects the caller tracks time on: non-archived projects where they are a member with a role other
    /// than <see cref="ProjectMemberRoles.Client"/> (clients follow progress, they never track).</summary>
    [Authorize] // member-scoped: any authenticated user sees only their own projects
    public async Task<List<MyProjectDto>> GetMyProjectsAsync()
    {
        var userId = CurrentUser.GetId();
        var clientRole = ProjectMemberRoles.Client.ToUpperInvariant();

        var memberQuery = await _memberRepository.GetQueryableAsync();
        var projectQuery = await Repository.GetQueryableAsync();

        var query =
            from member in memberQuery
            join project in projectQuery on member.ProjectId equals project.Id
            where member.UserId == userId && !project.IsArchived && member.Role.Trim().ToUpper() != clientRole
            orderby project.Title
            select project;

        var projects = await AsyncExecuter.ToListAsync(query);

        // A duplicated membership must not list the project twice in the agents' picker.
        return projects.DistinctBy(p => p.Id).Select(p => new MyProjectDto
        {
            Id = p.Id,
            Title = p.Title,
            IntervalMinutes = p.IntervalMinutes,
            AllowScreenshot = p.AllowScreenshot,
            AllowWebcam = p.AllowWebcam,
            AllowKeyboard = p.AllowKeyboard,
            AllowMouse = p.AllowMouse,
            AllowActiveWindow = p.AllowActiveWindow,
            AllowRunningPrograms = p.AllowRunningPrograms,
            IdleThresholdMinutes = p.IdleThresholdMinutes,
            BlurScreenshots = p.BlurScreenshots
        }).ToList();
    }

    [Authorize(TrackerPermissions.Projects.Archive)]
    public async Task<ProjectDto> ArchiveAsync(Guid id)
    {
        var project = await Repository.GetAsync(id);
        project.Archive(Clock.Now);
        // Saved now so the returned audit properties (LastModificationTime, …) are the stored ones.
        await Repository.UpdateAsync(project, autoSave: true);
        return await MapToGetOutputDtoAsync(project);
    }

    [Authorize(TrackerPermissions.Projects.Archive)]
    public async Task<ProjectDto> UnarchiveAsync(Guid id)
    {
        var project = await Repository.GetAsync(id);
        project.Unarchive();
        await Repository.UpdateAsync(project, autoSave: true);
        return await MapToGetOutputDtoAsync(project);
    }

    // Project -> ProjectDto goes through ProjectMapper (TrackerApplicationMappers), so every field, the audit
    // properties included, reaches the client.

    protected override Project MapToEntity(CreateUpdateProjectDto createInput)
    {
        NormalizeAndValidate(createInput);

        return new Project(GuidGenerator.Create(), CurrentTenant.Id, createInput.Title, createInput.IntervalMinutes, createInput.Color)
        {
            Description = createInput.Description,
            AllowScreenshot = createInput.AllowScreenshot,
            AllowWebcam = createInput.AllowWebcam,
            AllowKeyboard = createInput.AllowKeyboard,
            AllowMouse = createInput.AllowMouse,
            AllowActiveWindow = createInput.AllowActiveWindow,
            AllowRunningPrograms = createInput.AllowRunningPrograms,
            IdleThresholdMinutes = createInput.IdleThresholdMinutes,
            BlurScreenshots = createInput.BlurScreenshots
        };
    }

    protected override void MapToEntity(CreateUpdateProjectDto updateInput, Project entity)
    {
        NormalizeAndValidate(updateInput);

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
        entity.IdleThresholdMinutes = updateInput.IdleThresholdMinutes;
        entity.BlurScreenshots = updateInput.BlurScreenshots;
    }

    /// <summary>
    /// Enforces the project rules in the service itself, not only through the DTO's attributes: the colour is rendered
    /// as a style (a value such as <c>url(https://…)</c> would make every member's browser call a third party) and the
    /// description is returned in every project list. Trims the title and the description (an empty description is
    /// stored as none). Violations are validation errors (400 with <c>validationErrors</c>).
    /// </summary>
    internal static void NormalizeAndValidate(CreateUpdateProjectDto input)
    {
        input.Title = input.Title?.Trim()!;
        input.Description = string.IsNullOrWhiteSpace(input.Description) ? null : input.Description.Trim();

        var errors = new List<ValidationResult>();
        if (string.IsNullOrEmpty(input.Title))
        {
            errors.Add(new ValidationResult("The project needs a title.", new[] { nameof(CreateUpdateProjectDto.Title) }));
        }

        if (input.Color == null || !HexColor.IsMatch(input.Color))
        {
            errors.Add(new ValidationResult(
                "The colour must be a hex colour such as #006bff.", new[] { nameof(CreateUpdateProjectDto.Color) }));
        }

        if (input.Description?.Length > MaxDescriptionLength)
        {
            errors.Add(new ValidationResult(
                $"The description can be at most {MaxDescriptionLength} characters long.",
                new[] { nameof(CreateUpdateProjectDto.Description) }));
        }

        if (errors.Count > 0)
        {
            throw new AbpValidationException(errors);
        }
    }
}
