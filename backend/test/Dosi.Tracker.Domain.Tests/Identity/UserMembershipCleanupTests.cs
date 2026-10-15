using System;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Workspaces;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Identity;

/// <summary>Deleting an account takes it out of every workspace: its memberships end and its project memberships (seats)
/// go; rate history stays.</summary>
public abstract class UserMembershipCleanupTests<TStartupModule> : TrackerDomainTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IRepository<ProjectMemberRate, Guid> _rateRepository;
    private readonly IGuidGenerator _guidGenerator;
    private readonly ICurrentWorkspace _currentWorkspace;

    protected UserMembershipCleanupTests()
    {
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _memberRepository = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _rateRepository = GetRequiredService<IRepository<ProjectMemberRate, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
    }

    private async Task<Guid> SeedProjectsAsync(Guid workspaceId, Guid userId, Guid colleagueId)
    {
        var projectA = _guidGenerator.Create();
        var projectB = _guidGenerator.Create();
        using (_currentWorkspace.Change(workspaceId))
        {
            await WithUnitOfWorkAsync(async () =>
            {
                await _projectRepository.InsertAsync(new Project(projectA, workspaceId, "A"));
                await _projectRepository.InsertAsync(new Project(projectB, workspaceId, "B"));
                await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), workspaceId, projectA, userId, "Admin", 20m));
                await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), workspaceId, projectB, userId));
                await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), workspaceId, projectA, colleagueId));
                await _rateRepository.InsertAsync(new ProjectMemberRate(_guidGenerator.Create(), workspaceId, projectA, userId, 20m, DateTime.UtcNow));
            });
        }

        await ServiceProvider.AddWorkspaceMemberAsync(userId, WorkspaceRoles.Member, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(colleagueId, WorkspaceRoles.Member, workspaceId);
        return projectA;
    }

    [Fact]
    public async Task Deleting_A_User_Should_Remove_Their_Memberships_In_Every_Workspace()
    {
        var userId = _guidGenerator.Create();
        var colleagueId = _guidGenerator.Create();
        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            (await userManager.CreateAsync(new IdentityUser(userId, "leaver", "leaver@acme.test"))).Succeeded.ShouldBeTrue();
            (await userManager.CreateAsync(new IdentityUser(colleagueId, "colleague", "colleague@acme.test"))).Succeeded.ShouldBeTrue();
        });
        var other = await ServiceProvider.CreateWorkspaceAsync("Other");
        var projectId = await SeedProjectsAsync(TrackerTestConsts.WorkspaceId, userId, colleagueId);
        await SeedProjectsAsync(other, userId, colleagueId);

        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            (await userManager.DeleteAsync(await userManager.GetByIdAsync(userId))).Succeeded.ShouldBeTrue();
        });

        foreach (var workspaceId in new[] { TrackerTestConsts.WorkspaceId, other })
        {
            using (_currentWorkspace.Change(workspaceId))
            {
                await WithUnitOfWorkAsync(async () =>
                {
                    (await _memberRepository.GetListAsync(m => m.UserId == userId)).ShouldBeEmpty();
                    (await _memberRepository.GetListAsync(m => m.UserId == colleagueId)).Count.ShouldBe(1);
                    (await _rateRepository.GetListAsync(r => r.UserId == userId)).ShouldHaveSingleItem(); // payroll history stays
                });
            }

            (await ServiceProvider.FindWorkspaceMemberAsync(userId, workspaceId))!.IsActive.ShouldBeFalse();
            (await ServiceProvider.FindWorkspaceMemberAsync(colleagueId, workspaceId))!.IsActive.ShouldBeTrue();
        }

        projectId.ShouldNotBe(Guid.Empty);
    }
}
