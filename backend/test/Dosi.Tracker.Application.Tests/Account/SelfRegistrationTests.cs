using System.Threading.Tasks;
using Dosi.Tracker.SaaS;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Account;
using Volo.Abp.Account.Settings;
using Volo.Abp.Modularity;
using Volo.Abp.Settings;
using Xunit;

namespace Dosi.Tracker.Account;

public abstract class SelfRegistrationTests<TStartupModule> : TrackerApplicationTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    [Fact]
    public async Task Abp_Self_Registration_Should_Be_Disabled_By_Default()
    {
        var value = await GetRequiredService<ISettingProvider>().GetOrNullAsync(AccountSettingNames.IsSelfRegistrationEnabled);
        value.ShouldBe("False");

        // Anonymous /api/account/register would otherwise create users straight in the host.
        await Should.ThrowAsync<UserFriendlyException>(() =>
            GetRequiredService<IAccountAppService>().RegisterAsync(new RegisterDto
            {
                AppName = "Tracker",
                UserName = "intruder",
                EmailAddress = "intruder@evil.test",
                Password = "1q2w3E*intruder"
            }));
    }

    [Fact]
    public async Task Workspace_Sign_Up_Should_Still_Work_With_Self_Registration_Disabled()
    {
        var result = await GetRequiredService<IWorkspaceAppService>().RegisterAsync(new RegisterWorkspaceDto
        {
            Name = "stillworks",
            AdminEmail = "owner@stillworks.test",
            AdminPassword = "1q2w3E*still"
        });

        result.TenantId.ShouldNotBe(System.Guid.Empty);
    }
}
