using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Notifications;
using Dosi.Tracker.Workspaces;
using Shouldly;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.AiAssist;

public abstract class WorkspaceMemberNotifierTests<TStartupModule> : TrackerDomainTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private static readonly Guid Owner = Guid.Parse("4c701e62-0953-4dd3-910b-dc6cc93ccb01");
    private static readonly Guid Manager = Guid.Parse("4c701e62-0953-4dd3-910b-dc6cc93ccb02");
    private static readonly Guid Member = Guid.Parse("4c701e62-0953-4dd3-910b-dc6cc93ccb03");
    private static readonly Guid Removed = Guid.Parse("4c701e62-0953-4dd3-910b-dc6cc93ccb04");
    private static readonly Guid Deactivated = Guid.Parse("4c701e62-0953-4dd3-910b-dc6cc93ccb05");
    private static readonly Guid Elsewhere = Guid.Parse("4c701e62-0953-4dd3-910b-dc6cc93ccb06");

    private async Task CreateUserAsync(Guid id, string userName, bool isActive = true)
    {
        await WithUnitOfWorkAsync(async () =>
        {
            var user = new IdentityUser(id, userName, $"{userName}@acme.test");
            user.SetIsActive(isActive);
            (await GetRequiredService<IdentityUserManager>().CreateAsync(user)).Succeeded.ShouldBeTrue();
        });
    }

    private async Task<Guid> SeedAsync()
    {
        foreach (var (id, name) in new[]
                 {
                     (Owner, "notify.owner"), (Manager, "notify.manager"), (Member, "notify.member"), (Removed, "notify.removed"),
                     (Elsewhere, "notify.elsewhere")
                 })
        {
            await CreateUserAsync(id, name);
        }

        await CreateUserAsync(Deactivated, "notify.deactivated", isActive: false);

        var workspaceId = await ServiceProvider.CreateWorkspaceAsync("Notify Co");
        await ServiceProvider.AddWorkspaceMemberAsync(Owner, WorkspaceRoles.Owner, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(Manager, WorkspaceRoles.Manager, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(Member, WorkspaceRoles.Member, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(Removed, WorkspaceRoles.Member, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(Deactivated, WorkspaceRoles.Member, workspaceId);
        await ServiceProvider.AddWorkspaceMemberAsync(Elsewhere, WorkspaceRoles.Member, await ServiceProvider.CreateWorkspaceAsync("Other Co"));

        await WithUnitOfWorkAsync(async () =>
        {
            var memberships = GetRequiredService<WorkspaceMembershipManager>();
            await memberships.RemoveAsync((await memberships.FindActiveAsync(workspaceId, Removed)).ShouldNotBeNull());
        });

        return workspaceId;
    }

    [Fact]
    public async Task Every_Active_Member_With_An_Active_Account_Should_Be_Notified_Once_With_The_Text_Capped()
    {
        var workspaceId = await SeedAsync();
        var message = new string('a', WorkspaceMemberNotifier.MaxMessageLength + 40);

        int notified;
        using (GetRequiredService<ICurrentWorkspace>().Change(workspaceId))
        {
            notified = await WithUnitOfWorkAsync(() => GetRequiredService<WorkspaceMemberNotifier>().NotifyAsync(message));
        }

        notified.ShouldBe(3);
        var notifications = await WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<IDataFilter>().Disable<IWorkspaceScoped>())
            {
                return await GetRequiredService<IRepository<Notification, Guid>>().GetListAsync();
            }
        });

        notifications.Select(n => n.UserId).ShouldBe(new[] { Owner, Manager, Member }, ignoreOrder: true);
        notifications.ShouldAllBe(n => n.WorkspaceId == workspaceId && !n.IsRead);
        notifications.ShouldAllBe(n => n.Message.Length == WorkspaceMemberNotifier.MaxMessageLength);
    }

    [Fact]
    public async Task A_Short_Message_Should_Be_Kept_As_Written()
    {
        var workspaceId = await SeedAsync();

        using (GetRequiredService<ICurrentWorkspace>().Change(workspaceId))
        {
            await WithUnitOfWorkAsync(() => GetRequiredService<WorkspaceMemberNotifier>().NotifyAsync("AI assist is now on."));
            var notifications = await WithUnitOfWorkAsync(() => GetRequiredService<IRepository<Notification, Guid>>().GetListAsync());
            notifications.Count.ShouldBe(3);
            notifications.ShouldAllBe(n => n.Message == "AI assist is now on.");
        }
    }
}
