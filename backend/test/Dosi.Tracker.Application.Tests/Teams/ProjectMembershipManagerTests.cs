using System;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Modularity;
using Volo.Abp.MultiTenancy;
using Xunit;

namespace Dosi.Tracker.Teams;

/// <summary>
/// The backstop for two concurrent adds of the same person: the team service's duplicate check can be raced, and the
/// unique (tenant, project, user) index then refuses the second membership, which must be answered as a duplicate
/// (<c>Tracker:DuplicateProjectMember</c>, 409) rather than as a server error.
/// </summary>
public abstract class ProjectMembershipManagerTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly ProjectMembershipManager _memberships;
    private readonly IRepository<Project, Guid> _projects;
    private readonly IRepository<ProjectMember, Guid> _members;
    private readonly IRepository<ProjectMemberRate, Guid> _rates;
    private readonly ICurrentTenant _currentTenant;

    protected ProjectMembershipManagerTests()
    {
        _memberships = GetRequiredService<ProjectMembershipManager>();
        _projects = GetRequiredService<IRepository<Project, Guid>>();
        _members = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _rates = GetRequiredService<IRepository<ProjectMemberRate, Guid>>();
        _currentTenant = GetRequiredService<ICurrentTenant>();
    }

    [Fact]
    public async Task Enrolling_Someone_Already_On_The_Project_Should_Be_Refused_As_A_Duplicate()
    {
        // In a workspace: the in-memory test database enforces the index for a non-null tenant id (PostgreSQL for any).
        var tenantId = Guid.NewGuid();
        var userId = Guid.NewGuid();

        using (_currentTenant.Change(tenantId))
        {
            var projectId = await WithUnitOfWorkAsync(async () =>
                (await _projects.InsertAsync(new Project(Guid.NewGuid(), tenantId, "Website"), autoSave: true)).Id);
            await WithUnitOfWorkAsync(() => _memberships.EnrollAsync(projectId, userId, ProjectMemberRoles.Worker, 20m));

            // A second add that got past the service's check, as a concurrent one does.
            var exception = await Should.ThrowAsync<BusinessException>(() => WithUnitOfWorkAsync(
                () => _memberships.EnrollAsync(projectId, userId, ProjectMemberRoles.Admin, 30m)));

            exception.Code.ShouldBe(TrackerDomainErrorCodes.DuplicateProjectMember);
            exception.Data["projectId"].ShouldBe(projectId);
            exception.Data["userId"].ShouldBe(userId);

            // The first membership and its rate are untouched.
            (await WithUnitOfWorkAsync(() => _members.GetListAsync(m => m.ProjectId == projectId)))
                .ShouldHaveSingleItem().Role.ShouldBe(ProjectMemberRoles.Worker);
            (await WithUnitOfWorkAsync(() => _rates.GetListAsync(r => r.ProjectId == projectId)))
                .ShouldHaveSingleItem().HourlyRate.ShouldBe(20m);
        }
    }

    [Fact]
    public async Task Enrolling_On_Another_Project_Should_Not_Be_A_Duplicate()
    {
        var tenantId = Guid.NewGuid();
        var userId = Guid.NewGuid();

        using (_currentTenant.Change(tenantId))
        {
            var website = await WithUnitOfWorkAsync(async () =>
                (await _projects.InsertAsync(new Project(Guid.NewGuid(), tenantId, "Website"), autoSave: true)).Id);
            var app = await WithUnitOfWorkAsync(async () =>
                (await _projects.InsertAsync(new Project(Guid.NewGuid(), tenantId, "App"), autoSave: true)).Id);

            await WithUnitOfWorkAsync(() => _memberships.EnrollAsync(website, userId, ProjectMemberRoles.Worker, 0m));
            await WithUnitOfWorkAsync(() => _memberships.EnrollAsync(app, userId, ProjectMemberRoles.Worker, 0m));

            (await WithUnitOfWorkAsync(() => _members.GetListAsync(m => m.UserId == userId))).Count.ShouldBe(2);
        }
    }
}
