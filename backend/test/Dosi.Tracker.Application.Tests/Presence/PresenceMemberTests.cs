using System;
using System.Threading.Tasks;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Security;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Dosi.Tracker.Workspaces;
using Xunit;

namespace Dosi.Tracker.Presence;

/// <summary>
/// Presence for a regular member (no Activities.ViewAll): heartbeats need no permission, and the team list holds
/// only the caller's own devices.
/// </summary>
public abstract class PresenceMemberTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    // Matches FakeCurrentPrincipalAccessor in Dosi.Tracker.TestBase.
    private static readonly Guid CurrentUserId = Guid.Parse("2e701e62-0953-4dd3-910b-dc6cc93ccb0d");

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DenyPermissions(TrackerPermissions.Activities.ViewAll);
    }

    private Task CreateUserAsync(Guid id, string userName)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var user = new IdentityUser(id, userName, $"{userName}@acme.test");
            (await GetRequiredService<IdentityUserManager>().CreateAsync(user)).Succeeded.ShouldBeTrue();
            var members = GetRequiredService<IRepository<WorkspaceMember, Guid>>();
            if (await members.FirstOrDefaultAsync(m => m.WorkspaceId == TrackerTestConsts.WorkspaceId && m.UserId == id) == null)
            {
                await members.InsertAsync(new WorkspaceMember(Guid.NewGuid(), TrackerTestConsts.WorkspaceId, id, WorkspaceRoles.Member, DateTime.UtcNow));
            }
        });
    }

    [Fact]
    public async Task GetTeam_Should_Only_List_The_Callers_Own_Devices()
    {
        var colleagueId = Guid.NewGuid();
        await CreateUserAsync(CurrentUserId, "ana");
        await CreateUserAsync(colleagueId, "bob");
        await WithUnitOfWorkAsync(() => GetRequiredService<IRepository<AgentPresence, Guid>>().InsertAsync(
            new AgentPresence(Guid.NewGuid(), TrackerTestConsts.WorkspaceId, colleagueId, Guid.NewGuid(), "tracking", DateTime.UtcNow)));

        var ownDevice = Guid.NewGuid();
        await GetRequiredService<IPresenceAppService>().HeartbeatAsync(new HeartbeatDto { DeviceId = ownDevice, Status = "break" });

        var team = (await GetRequiredService<IPresenceAppService>().GetTeamAsync()).Items;

        var device = team.ShouldHaveSingleItem();
        device.UserId.ShouldBe(CurrentUserId);
        device.DeviceId.ShouldBe(ownDevice);
        device.Status.ShouldBe("break");
        device.Online.ShouldBeTrue();
    }
}
