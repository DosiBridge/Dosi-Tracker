using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Data;
using Dosi.Tracker.Permissions;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.DynamicProxy;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Volo.Abp.PermissionManagement;
using Xunit;

namespace Dosi.Tracker.Identity;

/// <summary>
/// The platform administrator (global role <c>platform-admin</c>) is seeded only into a brand-new database. Re-running
/// the seed (every DbMigrator run does) must never bring back a template <c>admin</c> / <c>1q2w3E*</c> account.
/// </summary>
public abstract class AdminSeedingTests<TStartupModule> : TrackerDomainTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private const string ConfiguredHostEmail = "platform@dosi.test";
    private const string ConfiguredHostPassword = "Configured-Pa55";

    private readonly IIdentityUserRepository _userRepository;
    private readonly IDataFilter _dataFilter;

    protected AdminSeedingTests()
    {
        _userRepository = GetRequiredService<IIdentityUserRepository>();
        _dataFilter = GetRequiredService<IDataFilter>();
    }

    protected override void BeforeAddApplication(IServiceCollection services)
    {
        services.ReplaceConfiguration(new ConfigurationBuilder()
            .AddJsonFile("appsettings.json", false)
            .AddInMemoryCollection(new Dictionary<string, string?>
            {
                [TrackerDbMigrationService.HostAdminEmailKey] = ConfiguredHostEmail,
                [TrackerDbMigrationService.HostAdminPasswordKey] = ConfiguredHostPassword
            })
            .Build());
    }

    private Task<List<IdentityUser>> UsersAsync(bool includeDeleted = false)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            using (includeDeleted ? _dataFilter.Disable<ISoftDelete>() : null)
            {
                return await _userRepository.GetListAsync(sorting: nameof(IdentityUser.UserName));
            }
        });
    }

    private Task HardDeleteAllUsersAsync()
    {
        return WithUnitOfWorkAsync(async () =>
        {
            using (_dataFilter.Disable<ISoftDelete>())
            {
                foreach (var user in await _userRepository.GetListAsync(sorting: nameof(IdentityUser.UserName)))
                {
                    await _userRepository.HardDeleteAsync(user);
                }
            }
        });
    }

    [Fact]
    public void Abps_Identity_Data_Seeder_Should_Be_Replaced()
    {
        ProxyHelper.UnProxy(GetRequiredService<IIdentityDataSeeder>()).ShouldBeAssignableTo<TrackerIdentityDataSeeder>();
        ProxyHelper.UnProxy(GetRequiredService<IdentityDataSeeder>()).ShouldBeAssignableTo<TrackerIdentityDataSeeder>();
    }

    [Fact]
    public async Task The_First_Administrator_Should_Be_A_Platform_Admin()
    {
        var admin = (await UsersAsync()).ShouldHaveSingleItem(); // seeded when the test application started
        admin.UserName.ShouldBe("admin");
        admin.ShouldChangePasswordOnNextLogin.ShouldBeTrue(); // the template password

        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            var stored = await userManager.GetByIdAsync(admin.Id);
            (await userManager.GetRolesAsync(stored)).ShouldBe(new[] { TrackerRoles.PlatformAdmin });
        });
    }

    [Fact]
    public async Task The_Platform_Admin_Role_Should_Hold_The_Platform_Permission_But_No_Workspace_Permission()
    {
        await WithUnitOfWorkAsync(async () =>
        {
            var grants = await GetRequiredService<IPermissionGrantRepository>()
                .GetListAsync("R", TrackerRoles.PlatformAdmin);
            var names = grants.Select(g => g.Name).ToList();

            names.ShouldContain(TrackerPermissions.Platform.Default);
            names.ShouldNotContain(TrackerPermissions.Team.Manage);
            names.ShouldNotContain(TrackerPermissions.Activities.ViewAll);
            names.ShouldNotContain(TrackerPermissions.Billing.Default);
        });
    }

    [Fact]
    public async Task The_Migrator_Should_Leave_A_Renamed_Administrator_Alone()
    {
        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            var admin = await userManager.FindByNameAsync("admin");
            (await userManager.SetUserNameAsync(admin!, "platform-owner")).Succeeded.ShouldBeTrue();
        });

        await GetRequiredService<TrackerDbMigrationService>().SeedHostAsync();

        (await UsersAsync()).ShouldHaveSingleItem().UserName.ShouldBe("platform-owner");
    }

    [Fact]
    public async Task A_Database_Whose_Users_Were_All_Deleted_Should_Not_Get_A_New_Administrator()
    {
        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            var admin = await userManager.FindByNameAsync("admin");
            (await userManager.DeleteAsync(await userManager.GetByIdAsync(admin!.Id))).Succeeded.ShouldBeTrue();
        });

        await GetRequiredService<TrackerDbMigrationService>().SeedHostAsync();

        (await UsersAsync()).ShouldBeEmpty();
        (await UsersAsync(includeDeleted: true)).ShouldHaveSingleItem().IsDeleted.ShouldBeTrue();
    }

    [Fact]
    public async Task No_Administrator_Should_Be_Created_While_Admin_Creation_Is_Suppressed()
    {
        await HardDeleteAllUsersAsync();

        using (TrackerIdentityDataSeeder.SuppressAdminCreation())
        {
            TrackerIdentityDataSeeder.IsAdminCreationSuppressed.ShouldBeTrue();
            await GetRequiredService<TrackerDbMigrationService>().SeedHostAsync();
        }

        TrackerIdentityDataSeeder.IsAdminCreationSuppressed.ShouldBeFalse();
        (await UsersAsync()).ShouldBeEmpty();
    }

    [Fact]
    public async Task The_Migrator_Should_Create_The_First_Administrator_From_Configuration()
    {
        await HardDeleteAllUsersAsync();

        var migrator = GetRequiredService<TrackerDbMigrationService>();
        var context = migrator.CreateHostSeedContext();
        context.TenantId.ShouldBeNull();
        context[IdentityDataSeedContributor.AdminEmailPropertyName].ShouldBe(ConfiguredHostEmail);
        context[IdentityDataSeedContributor.AdminPasswordPropertyName].ShouldBe(ConfiguredHostPassword);

        await migrator.SeedHostAsync();

        var admin = (await UsersAsync()).ShouldHaveSingleItem();
        admin.UserName.ShouldBe("admin");
        admin.Email.ShouldBe(ConfiguredHostEmail);
        admin.ShouldChangePasswordOnNextLogin.ShouldBeFalse();
        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            var stored = await userManager.GetByIdAsync(admin.Id);
            (await userManager.CheckPasswordAsync(stored, ConfiguredHostPassword)).ShouldBeTrue();
            (await userManager.IsInRoleAsync(stored, TrackerRoles.PlatformAdmin)).ShouldBeTrue();
        });
    }
}
