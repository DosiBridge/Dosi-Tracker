using System;
using System.Threading.Tasks;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Security;
using Dosi.Tracker.Teams;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Projects;

/// <summary>
/// <see cref="ProjectTaskRules"/> directly, per kind of caller: who may read a project's tasks, and who may change them.
/// </summary>
public abstract class ProjectTaskRulesTestBase<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    // Matches FakeCurrentPrincipalAccessor in Dosi.Tracker.TestBase.
    protected static readonly Guid CurrentUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");

    protected ProjectTaskRules Rules => GetRequiredService<ProjectTaskRules>();

    /// <summary>A project, with the caller as a member in <paramref name="callerRole"/> when one is given.</summary>
    protected async Task<Project> SeedProjectAsync(string? callerRole = null, bool archived = false)
    {
        var project = new Project(Guid.NewGuid(), TrackerTestConsts.WorkspaceId, "Tasks " + Guid.NewGuid().ToString("N")[..6]);
        if (archived)
        {
            project.Archive(DateTime.UtcNow.AddDays(-1));
        }

        await WithUnitOfWorkAsync(async () =>
        {
            await GetRequiredService<IRepository<Project, Guid>>().InsertAsync(project, autoSave: true);
            if (callerRole != null)
            {
                await GetRequiredService<IRepository<ProjectMember, Guid>>().InsertAsync(
                    new ProjectMember(Guid.NewGuid(), TrackerTestConsts.WorkspaceId, project.Id, CurrentUserId, callerRole),
                    autoSave: true);
            }
        });

        return project;
    }

    protected Task ReadAsync(Project project) => WithUnitOfWorkAsync(() => Rules.EnsureCanReadProjectAsync(project));

    protected Task MutateAsync(Project project) => WithUnitOfWorkAsync(() => Rules.EnsureCanMutateProjectTasksAsync(project));

    protected async Task ShouldRefuseMutationAsync(Project project, string code)
    {
        (await Should.ThrowAsync<BusinessException>(() => MutateAsync(project))).Code.ShouldBe(code);
    }
}

/// <summary>A project editor who sees everyone (the test authorization grants every permission).</summary>
public abstract class ProjectTaskRulesTests<TStartupModule> : ProjectTaskRulesTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    [Fact]
    public async Task Should_Allow_Reading_And_Changing_Any_Projects_Tasks()
    {
        var project = await SeedProjectAsync();

        await ReadAsync(project);
        await MutateAsync(project);
    }

    [Fact]
    public async Task Should_Freeze_An_Archived_Projects_Tasks_But_Keep_Them_Readable()
    {
        var project = await SeedProjectAsync(archived: true);

        await ReadAsync(project);
        await ShouldRefuseMutationAsync(project, TrackerDomainErrorCodes.ProjectArchived);
    }
}

/// <summary>A regular member: no Tracker.Projects.Edit and no Tracker.Activities.ViewAll.</summary>
public abstract class ProjectTaskRulesMemberTests<TStartupModule> : ProjectTaskRulesTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DenyPermissions(TrackerPermissions.Projects.Edit, TrackerPermissions.Activities.ViewAll);
    }

    [Fact]
    public async Task A_Worker_Should_Read_And_Change_Their_Projects_Tasks()
    {
        var project = await SeedProjectAsync(ProjectMemberRoles.Worker);

        await ReadAsync(project);
        await MutateAsync(project);
    }

    [Fact]
    public async Task A_Client_Should_Read_But_Not_Change_The_Tasks()
    {
        var project = await SeedProjectAsync(ProjectMemberRoles.Client);

        await ReadAsync(project);
        await ShouldRefuseMutationAsync(project, TrackerDomainErrorCodes.RoleNotAllowed);
    }

    [Fact]
    public async Task A_Non_Member_Should_Not_Find_The_Project_Nor_Change_Its_Tasks()
    {
        var project = await SeedProjectAsync();

        (await Should.ThrowAsync<EntityNotFoundException>(() => ReadAsync(project))).Id.ShouldBe(project.Id);
        await ShouldRefuseMutationAsync(project, TrackerDomainErrorCodes.NotProjectMember);
    }

    [Fact]
    public async Task An_Archived_Project_Should_Be_Refused_Before_The_Membership_Is_Checked()
    {
        var project = await SeedProjectAsync(archived: true);

        await ShouldRefuseMutationAsync(project, TrackerDomainErrorCodes.ProjectArchived);
    }
}

/// <summary>A viewer: sees everyone's activity (ViewAll) but may not edit projects.</summary>
public abstract class ProjectTaskRulesViewerTests<TStartupModule> : ProjectTaskRulesTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DenyPermissions(TrackerPermissions.Projects.Edit);
    }

    [Fact]
    public async Task Should_Read_Any_Projects_Tasks_But_Change_Only_Their_Own_Projects()
    {
        var foreign = await SeedProjectAsync();
        var own = await SeedProjectAsync(ProjectMemberRoles.Worker);

        await ReadAsync(foreign);
        await ShouldRefuseMutationAsync(foreign, TrackerDomainErrorCodes.NotProjectMember);
        await MutateAsync(own);
    }
}
