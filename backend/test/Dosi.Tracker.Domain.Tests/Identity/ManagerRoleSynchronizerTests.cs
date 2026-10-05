using System;
using System.Threading.Tasks;
using Dosi.Tracker.Projects;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Identity;

public abstract class ManagerRoleSynchronizerTests<TStartupModule> : TrackerDomainTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly ManagerRoleSynchronizer _synchronizer;
    private readonly IRepository<Project, Guid> _projectRepository;
    private readonly IRepository<ProjectMember, Guid> _memberRepository;
    private readonly IGuidGenerator _guidGenerator;

    protected ManagerRoleSynchronizerTests()
    {
        _synchronizer = GetRequiredService<ManagerRoleSynchronizer>();
        _projectRepository = GetRequiredService<IRepository<Project, Guid>>();
        _memberRepository = GetRequiredService<IRepository<ProjectMember, Guid>>();
        _guidGenerator = GetRequiredService<IGuidGenerator>();
    }

    /// <summary>A team lead (Identity "manager" role) who is <paramref name="role"/> on a new project.</summary>
    private async Task<(Guid UserId, Guid ProjectId)> SeedLeadAsync(string role)
    {
        var userId = _guidGenerator.Create();
        var projectId = _guidGenerator.Create();

        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            var user = new IdentityUser(userId, "lead", "lead@acme.test");
            (await userManager.CreateAsync(user)).Succeeded.ShouldBeTrue();
            await _projectRepository.InsertAsync(new Project(projectId, null, "Led project"));
            await _memberRepository.InsertAsync(new ProjectMember(_guidGenerator.Create(), null, projectId, userId, role));
        });
        await WithUnitOfWorkAsync(() => _synchronizer.GrantAsync(userId));

        return (userId, projectId);
    }

    private Task<bool> IsManagerAsync(Guid userId)
    {
        return WithUnitOfWorkAsync(async () =>
            (await GetRequiredService<IIdentityUserRepository>().GetRoleNamesAsync(userId)).Contains(TrackerRoles.Manager));
    }

    [Fact]
    public async Task Grant_Should_Give_The_Manager_Role_And_Ignore_Unknown_Users()
    {
        var (userId, _) = await SeedLeadAsync("Admin");

        (await IsManagerAsync(userId)).ShouldBeTrue();
        await WithUnitOfWorkAsync(() => _synchronizer.GrantAsync(Guid.NewGuid())); // no account: nothing to grant
    }

    [Fact]
    public async Task Revoke_Should_Keep_The_Role_While_A_Manager_Membership_Remains()
    {
        var (userId, _) = await SeedLeadAsync("Manager");

        await WithUnitOfWorkAsync(() => _synchronizer.RevokeIfUnusedAsync(userId));

        (await IsManagerAsync(userId)).ShouldBeTrue();
    }

    [Fact]
    public async Task Revoke_Should_Ignore_Memberships_Left_Behind_By_A_Deleted_Project()
    {
        var (userId, projectId) = await SeedLeadAsync("Admin");
        // A project deleted before deletions removed memberships: its membership is still there.
        await WithUnitOfWorkAsync(() => _projectRepository.DeleteAsync(projectId));

        (await WithUnitOfWorkAsync(() => _synchronizer.HoldsManagerMembershipAsync(userId))).ShouldBeFalse();
        await WithUnitOfWorkAsync(() => _synchronizer.RevokeIfUnusedAsync(userId));

        (await IsManagerAsync(userId)).ShouldBeFalse();
    }

    [Fact]
    public async Task Revoke_Should_Drop_The_Role_Once_Only_Worker_Memberships_Remain()
    {
        var (userId, projectId) = await SeedLeadAsync("Worker");

        await WithUnitOfWorkAsync(() => _synchronizer.RevokeIfUnusedAsync(userId));

        (await IsManagerAsync(userId)).ShouldBeFalse();
        (await WithUnitOfWorkAsync(() => _memberRepository.GetListAsync(m => m.ProjectId == projectId))).ShouldHaveSingleItem();
    }
}
