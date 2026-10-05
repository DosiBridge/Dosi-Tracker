using System.Collections.Generic;
using System.Threading.Tasks;
using Dosi.Tracker.Data;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.Identity;

/// <summary>
/// What the DbMigrator does with the platform (host) administrator when <c>App:HostAdminPassword</c> is not configured
/// (as in a deployment that forgot it). Shared by the Production and Development variants below.
/// </summary>
public abstract class HostAdministratorBootstrapTestBase<TStartupModule> : TrackerDomainTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected TrackerDbMigrationService Migrator => GetRequiredService<TrackerDbMigrationService>();

    protected override void BeforeAddApplication(IServiceCollection services)
    {
        // No host administrator password, and no ABP telemetry from a Development test application.
        services.ReplaceConfiguration(new ConfigurationBuilder()
            .AddJsonFile("appsettings.json", false)
            .AddInMemoryCollection(new Dictionary<string, string?> { ["Abp:Telemetry:IsEnabled"] = "false" })
            .Build());
    }

    /// <summary>A host database without any user: what the very first DbMigrator run of a deployment sees.</summary>
    protected Task RemoveEveryHostUserAsync()
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var users = GetRequiredService<IIdentityUserRepository>();
            foreach (var user in await users.GetListAsync(sorting: nameof(IdentityUser.UserName)))
            {
                await users.HardDeleteAsync(user);
            }
        });
    }

    protected Task<List<IdentityUser>> HostUsersAsync()
    {
        return WithUnitOfWorkAsync(() =>
            GetRequiredService<IIdentityUserRepository>().GetListAsync(sorting: nameof(IdentityUser.UserName)));
    }
}

/// <summary>Outside Development (here: Production, ABP's default) the first host administrator is never created with the
/// template password <c>1q2w3E*</c>, which anyone can look up.</summary>
public abstract class HostAdministratorBootstrapTests<TStartupModule> : HostAdministratorBootstrapTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    [Fact]
    public async Task A_New_Host_Should_Not_Get_An_Administrator_With_The_Template_Password()
    {
        await RemoveEveryHostUserAsync();
        Migrator.CreateHostSeedContext()[IdentityDataSeedContributor.AdminPasswordPropertyName]
            .ShouldBe(TrackerConsts.AdminPasswordDefaultValue); // nothing configured

        var exception = await Should.ThrowAsync<AbpException>(() => Migrator.SeedHostAsync());

        exception.Message.ShouldContain(TrackerDbMigrationService.HostAdminPasswordKey);
        (await HostUsersAsync()).ShouldBeEmpty();
    }

    [Fact]
    public async Task An_Existing_Host_Should_Still_Be_Seeded_Without_A_Configured_Password()
    {
        var before = await HostUsersAsync();
        before.ShouldNotBeEmpty(); // its administrator was created long ago

        await Migrator.SeedHostAsync();

        (await HostUsersAsync()).Count.ShouldBe(before.Count);
    }
}

/// <summary>In Development (a local database) the first host administrator gets the template password, and must change
/// it at the first sign-in.</summary>
public abstract class DevelopmentHostAdministratorBootstrapTests<TStartupModule> : HostAdministratorBootstrapTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override void SetAbpApplicationCreationOptions(AbpApplicationCreationOptions options)
    {
        base.SetAbpApplicationCreationOptions(options);
        options.Environment = Environments.Development;
    }

    [Fact]
    public async Task A_New_Development_Host_Should_Get_The_Template_Password_To_Change_At_The_First_Sign_In()
    {
        await RemoveEveryHostUserAsync();

        await Migrator.SeedHostAsync();

        var admin = (await HostUsersAsync()).ShouldHaveSingleItem();
        admin.UserName.ShouldBe(TrackerIdentityDataSeeder.DefaultAdminUserName);
        admin.ShouldChangePasswordOnNextLogin.ShouldBeTrue();
        await WithUnitOfWorkAsync(async () =>
        {
            var userManager = GetRequiredService<IdentityUserManager>();
            (await userManager.CheckPasswordAsync(await userManager.GetByIdAsync(admin.Id), TrackerConsts.AdminPasswordDefaultValue))
                .ShouldBeTrue();
        });
    }
}
