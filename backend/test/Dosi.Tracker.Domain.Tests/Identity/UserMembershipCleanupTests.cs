using System;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Volo.Abp.MultiTenancy;
using Xunit;

namespace Dosi.Tracker.Identity;

/// <summary>Deleting an Identity user takes their project memberships (their seats) with them; rate history stays.</summary>
public abstract class UserMembershipCleanupTests<TStartupModule> : TrackerDomainTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<ProjectMemberRate, Guid> _rateRepository;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentTenant _currentTenant;

    protected UserMembershipCleanupTests()
    {
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _memberRepository = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _rateRepository = GetRequiredService<IRepository<ProjectMemberRate, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentTenant = GetRequiredService<ICurrentTenant>();
    }

    /// <summary>A user of <paramref name="tenantId"/> who is a member of two projects, next to a colleague on one of them.</summary>
    private async Task<(Guid UserId, Guid ColleagueId, Guid ProjectId)> SeedAsync(Guid? tenantId)
    {
        var userId = _guidGenerator.Create();
        var colleagueId = _guidGenerator.Create();
        var projectA = _guidGenerator.Create();
        var projectB = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentTenant.Change(tenantId))
            {
                var userManager = GetRequiredService<IdentityUserManager>();
                (await userManager.CreateAsync(new IdentityUser(userId, "leaver", "leaver@acme.test", tenantId))).Succeeded.ShouldBeTrue();
                (await userManager.CreateAsync(new IdentityUser(colleagueId, "colleague", "colleague@acme.test", tenantId))).Succeeded.ShouldBeTrue();

                await _projectRepository.InsertAsync(new Project(projectA, tenantId, "A"));
                await _projectRepository.InsertAsync(new Project(projectB, tenantId, "B"));
                await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), tenantId, projectA, userId, "Admin", 20m));
                await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), tenantId, projectB, userId));
                await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), tenantId, projectA, colleagueId));
                await _rateRepository.InsertAsync(new ProjectMemberRate(_guidGenerator.Create(), tenantId, projectA, userId, 20m, DateTime.UtcNow));
            }
        });

        return (userId, colleagueId, projectA);
    }

    private async Task DeleteUserAsync(Guid? tenantId, Guid userId)
    {
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentTenant.Change(tenantId))
            {
                var userManager = GetRequiredService<IdentityUserManager>();
                (await userManager.DeleteAsync(await userManager.GetByIdAsync(userId))).Succeeded.ShouldBeTrue();
            }
        });
    }

    [Fact]
    public async Task Deleting_A_User_Should_Remove_Their_Memberships()
    {
        var (userId, colleagueId, projectId) = await SeedAsync(null);

        await DeleteUserAsync(null, userId);

        await WithUnitOfWorkAsync(async () =>
        {
            (await _memberRepository.GetListAsync(m => m.UserId == userId)).ShouldBeEmpty();
            (await _memberRepository.GetListAsync(m => m.UserId == colleagueId)).ShouldHaveSingleItem().ProjectId.ShouldBe(projectId);
            (await _rateRepository.GetListAsync(r => r.UserId == userId)).ShouldHaveSingleItem(); // payroll history stays
        });
    }

    [Fact]
    public async Task Deleting_A_Workspace_User_Should_Remove_Their_Memberships_In_That_Workspace()
    {
        var tenantId = Guid.NewGuid();
        var (userId, colleagueId, _) = await SeedAsync(tenantId);

        await DeleteUserAsync(tenantId, userId);

        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentTenant.Change(tenantId))
            {
                (await _memberRepository.GetListAsync(m => m.UserId == userId)).ShouldBeEmpty();
                (await _memberRepository.GetListAsync(m => m.UserId == colleagueId)).ShouldHaveSingleItem();
            }
        });
    }
}
