using System;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Microsoft.EntityFrameworkCore;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.MultiTenancy;
using Xunit;

namespace Dosi.Tracker.EntityFrameworkCore.Persistence;

/* Schema-level guarantees of AppProjectMembers (the AgentDataV2Followup migration), checked against the real
 * database rather than through the team service. */
[Collection(TrackerTestConsts.CollectionDefinitionName)]
public class ProjectMemberPersistenceTests : TrackerEntityFrameworkCoreTestBase
{
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly ICurrentTenant _currentTenant;

    public ProjectMemberPersistenceTests()
    {
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _memberRepository = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _currentTenant = GetRequiredService<ICurrentTenant>();
    }

    private Task<Guid> CreateProjectAsync(Guid tenantId)
    {
        return WithUnitOfWorkAsync(async () =>
            (await _projectRepository.InsertAsync(new Project(Guid.NewGuid(), tenantId, "Website"), autoSave: true)).Id);
    }

    private Task InsertAsync(Guid tenantId, Guid projectId, Guid userId, string role = "Worker")
    {
        return WithUnitOfWorkAsync(() => _memberRepository.InsertAsync(
            new ProjectMember(Guid.NewGuid(), tenantId, projectId, userId, role),
            autoSave: true));
    }

    [Fact]
    public async Task A_Person_Can_Join_A_Project_Only_Once()
    {
        var tenantId = Guid.NewGuid();
        var userId = Guid.NewGuid();

        using (_currentTenant.Change(tenantId))
        {
            var projectId = await CreateProjectAsync(tenantId);
            await InsertAsync(tenantId, projectId, userId);

            // The unique (TenantId, ProjectId, UserId) index is the backstop for two concurrent adds, whatever the role.
            await Should.ThrowAsync<DbUpdateException>(() => InsertAsync(tenantId, projectId, userId, "Client"));

            (await WithUnitOfWorkAsync(() => _memberRepository.CountAsync(m => m.ProjectId == projectId && m.UserId == userId)))
                .ShouldBe(1);
        }
    }

    [Fact]
    public async Task A_Person_Can_Join_Several_Projects_And_A_Project_Can_Have_Several_Members()
    {
        var tenantId = Guid.NewGuid();
        var userId = Guid.NewGuid();

        using (_currentTenant.Change(tenantId))
        {
            var website = await CreateProjectAsync(tenantId);
            var app = await CreateProjectAsync(tenantId);

            await InsertAsync(tenantId, website, userId);
            await InsertAsync(tenantId, app, userId);
            await InsertAsync(tenantId, website, Guid.NewGuid());

            (await WithUnitOfWorkAsync(() => _memberRepository.GetCountAsync())).ShouldBe(3);
        }
    }
}
