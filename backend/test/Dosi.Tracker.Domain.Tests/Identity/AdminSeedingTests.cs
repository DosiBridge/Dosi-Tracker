using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Data;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.DynamicProxy;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Volo.Abp.MultiTenancy;
using Volo.Abp.TenantManagement;
using Xunit;

namespace Dosi.Tracker.Identity;

/// <summary>
/// Administrators are seeded only into a brand-new host or workspace. Re-running the seed (every DbMigrator run does)
/// must never bring back a template <c>admin</c> / <c>1q2w3E*</c> account where the owner renamed or removed it.
/// </summary>
public abstract class AdminSeedingTests<TStartupModule> : TrackerDomainTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private const string ConfiguredHostEmail = "platform@dosi.test";
    private const string ConfiguredHostPassword = "Configured-Pa55";

    private readonly IDataSeeder _dataSeeder;
    private readonly IIdentityUserRepository _userRepository;
    private readonly ICurrentTenant _currentTenant;
    private readonly IDataFilter _dataFilter;

    protected AdminSeedingTests()
    {
        _dataSeeder = GetRequiredService<IDataSeeder>();
        _userRepository = GetRequiredService<IIdentityUserRepository>();
        _currentTenant = GetRequiredService<ICurrentTenant>();
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

    /// <summary>Seeds a workspace the way ABP's tenant creation (and the old DbMigrator) did: with explicit admin credentials.</summary>
    private Task SeedWithAdminCredentialsAsync(Guid tenantId, string password = "1q2w3E*")
    {
        return _dataSeeder.SeedAsync(new DataSeedContext(tenantId)
            .WithProperty(IdentityDataSeedContributor.AdminEmailPropertyName, "admin@abp.io")
            .WithProperty(IdentityDataSeedContributor.AdminPasswordPropertyName, password));
    }

    private Task<List<IdentityUser>> UsersOfAsync(Guid? tenantId, bool includeDeleted = false)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            using (_currentTenant.Change(tenantId))
            using (includeDeleted ? _dataFilter.Disable<ISoftDelete>() : null)
            {
                return await _userRepository.GetListAsync(sorting: nameof(IdentityUser.UserName));
            }
        });
    }

    private Task UpdateUserAsync(Guid tenantId, string userName, Func<IdentityUserManager, IdentityUser, Task> change)
    {
        return WithUnitOfWorkAsync(async () =>
        {
            using (_currentTenant.Change(tenantId))
            {
                var userManager = GetRequiredService<IdentityUserManager>();
                var found = await userManager.FindByNameAsync(userName);
                found.ShouldNotBeNull();
                // Loaded with its details (roles, claims, …): deleting a user clears them.
                await change(userManager, await userManager.GetByIdAsync(found.Id));
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
    public async Task A_New_Workspace_Should_Get_Its_Administrator()
    {
        var tenantId = Guid.NewGuid();

        await SeedWithAdminCredentialsAsync(tenantId, "Owner-Chosen-1");

        var admin = (await UsersOfAsync(tenantId)).ShouldHaveSingleItem();
        admin.UserName.ShouldBe("admin");
        admin.ShouldChangePasswordOnNextLogin.ShouldBeFalse(); // the owner chose it
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentTenant.Change(tenantId))
            {
                var userManager = GetRequiredService<IdentityUserManager>();
                var stored = await userManager.GetByIdAsync(admin.Id);
                (await userManager.IsInRoleAsync(stored, TrackerRoles.Admin)).ShouldBeTrue();
                (await userManager.CheckPasswordAsync(stored, "Owner-Chosen-1")).ShouldBeTrue();
            }
        });
    }

    [Fact]
    public async Task An_Administrator_Created_With_The_Template_Password_Should_Have_To_Change_It()
    {
        // E.g. a workspace the host created in the console and gave the template password.
        var tenantId = Guid.NewGuid();

        await SeedWithAdminCredentialsAsync(tenantId);

        var admin = (await UsersOfAsync(tenantId)).ShouldHaveSingleItem();
        admin.ShouldChangePasswordOnNextLogin.ShouldBeTrue();
    }

    [Fact]
    public async Task A_Workspace_Whose_Admin_Was_Renamed_Should_Not_Get_A_New_One()
    {
        var tenantId = Guid.NewGuid();
        await SeedWithAdminCredentialsAsync(tenantId, "Owner-Chosen-1");
        await UpdateUserAsync(tenantId, "admin", async (userManager, user) =>
            (await userManager.SetUserNameAsync(user, "jane")).Succeeded.ShouldBeTrue());

        await SeedWithAdminCredentialsAsync(tenantId);

        (await UsersOfAsync(tenantId)).ShouldHaveSingleItem().UserName.ShouldBe("jane");
    }

    [Fact]
    public async Task A_Workspace_Whose_Users_Were_All_Deleted_Should_Not_Get_One_Either()
    {
        var tenantId = Guid.NewGuid();
        await SeedWithAdminCredentialsAsync(tenantId, "Owner-Chosen-1");
        await UpdateUserAsync(tenantId, "admin", async (userManager, user) =>
            (await userManager.DeleteAsync(user)).Succeeded.ShouldBeTrue());

        await SeedWithAdminCredentialsAsync(tenantId);

        (await UsersOfAsync(tenantId)).ShouldBeEmpty();
        (await UsersOfAsync(tenantId, includeDeleted: true)).ShouldHaveSingleItem().IsDeleted.ShouldBeTrue();
    }

    [Fact]
    public async Task No_Administrator_Should_Be_Created_While_Admin_Creation_Is_Suppressed()
    {
        var tenantId = Guid.NewGuid();

        using (TrackerIdentityDataSeeder.SuppressAdminCreation())
        {
            TrackerIdentityDataSeeder.IsAdminCreationSuppressed.ShouldBeTrue();
            await SeedWithAdminCredentialsAsync(tenantId);
        }

        TrackerIdentityDataSeeder.IsAdminCreationSuppressed.ShouldBeFalse();
        (await UsersOfAsync(tenantId)).ShouldBeEmpty();

        await SeedWithAdminCredentialsAsync(tenantId, "Owner-Chosen-1");
        (await UsersOfAsync(tenantId)).ShouldHaveSingleItem();
    }

    [Fact]
    public async Task The_Migrator_Should_Seed_Existing_Workspaces_Without_Creating_Administrators()
    {
        var tenant = await WithUnitOfWorkAsync(async () =>
        {
            var created = await GetRequiredService<ITenantManager>().CreateAsync("emptyco");
            return await GetRequiredService<ITenantRepository>().InsertAsync(created, autoSave: true);
        });

        await GetRequiredService<TrackerDbMigrationService>().SeedTenantAsync(tenant);

        (await UsersOfAsync(tenant.Id)).ShouldBeEmpty();

        // The other seeds still ran for the workspace: it has its "manager" role.
        await WithUnitOfWorkAsync(async () =>
        {
            using (_currentTenant.Change(tenant.Id))
            {
                (await GetRequiredService<IdentityRoleManager>().FindByNameAsync(TrackerRoles.Manager)).ShouldNotBeNull();
            }
        });
    }

    [Fact]
    public async Task The_Migrator_Should_Leave_A_Renamed_Host_Administrator_Alone()
    {
        var before = await UsersOfAsync(null);
        before.ShouldContain(u => u.UserName == "admin"); // seeded when the test application started

        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            var admin = await userManager.FindByNameAsync("admin");
            (await userManager.SetUserNameAsync(admin!, "platform-owner")).Succeeded.ShouldBeTrue();
        });

        await GetRequiredService<TrackerDbMigrationService>().SeedHostAsync();

        var after = await UsersOfAsync(null);
        after.Count.ShouldBe(before.Count);
        after.ShouldNotContain(u => u.UserName == "admin");
    }

    [Fact]
    public async Task The_Migrator_Should_Create_The_First_Host_Administrator_From_Configuration()
    {
        // A host database without any user: what the very first DbMigrator run sees.
        await WithUnitOfWorkAsync(async () =>
        {
            foreach (var user in await _userRepository.GetListAsync(sorting: nameof(IdentityUser.UserName)))
            {
                await _userRepository.HardDeleteAsync(user);
            }
        });

        var migrator = GetRequiredService<TrackerDbMigrationService>();
        var context = migrator.CreateHostSeedContext();
        context.TenantId.ShouldBeNull();
        context[IdentityDataSeedContributor.AdminEmailPropertyName].ShouldBe(ConfiguredHostEmail);
        context[IdentityDataSeedContributor.AdminPasswordPropertyName].ShouldBe(ConfiguredHostPassword);

        await migrator.SeedHostAsync();

        var admin = (await UsersOfAsync(null)).ShouldHaveSingleItem();
        admin.UserName.ShouldBe("admin");
        admin.Email.ShouldBe(ConfiguredHostEmail);
        admin.ShouldChangePasswordOnNextLogin.ShouldBeFalse();
        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            (await userManager.CheckPasswordAsync(await userManager.GetByIdAsync(admin.Id), ConfiguredHostPassword)).ShouldBeTrue();
        });
    }
}
