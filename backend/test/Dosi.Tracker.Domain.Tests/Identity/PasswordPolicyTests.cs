using System;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Identity;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Options;
using Shouldly;
using Volo.Abp.Identity;
using Volo.Abp.Identity.Settings;
using Volo.Abp.Modularity;
using Volo.Abp.Settings;
using Xunit;
using IdentityUser = Volo.Abp.Identity.IdentityUser;

namespace Dosi.Tracker.Identity;

/// <summary>The default password policy: at least 8 characters with a digit, an upper-case and a lower-case letter.</summary>
public abstract class PasswordPolicyTests<TStartupModule> : TrackerDomainTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    [Fact]
    public async Task The_Policy_Settings_Should_Default_To_Eight_Characters_With_Digit_Upper_And_Lower()
    {
        var settings = GetRequiredService<ISettingProvider>();

        (await settings.GetAsync<int>(IdentitySettingNames.Password.RequiredLength)).ShouldBe(8);
        (await settings.GetAsync<bool>(IdentitySettingNames.Password.RequireDigit)).ShouldBeTrue();
        (await settings.GetAsync<bool>(IdentitySettingNames.Password.RequireUppercase)).ShouldBeTrue();
        (await settings.GetAsync<bool>(IdentitySettingNames.Password.RequireLowercase)).ShouldBeTrue();
        (await settings.GetAsync<bool>(IdentitySettingNames.Password.RequireNonAlphanumeric)).ShouldBeFalse();
    }

    [Theory]
    [InlineData("Abcdefg1", true)]
    [InlineData("Correct-Horse-9", true)]
    [InlineData("Abcdef1", false)]   // 7 characters
    [InlineData("abcdefg1", false)]  // no upper-case letter
    [InlineData("ABCDEFG1", false)]  // no lower-case letter
    [InlineData("Abcdefgh", false)]  // no digit
    [InlineData("111111", false)]
    public async Task The_Identity_Validators_Should_Apply_The_Policy(string password, bool accepted)
    {
        using var scope = ServiceProvider.CreateScope();
        // ABP loads the policy settings into IdentityOptions on demand, as its own endpoints do before validating.
        await scope.ServiceProvider.GetRequiredService<IOptions<IdentityOptions>>().SetAsync();
        var userManager = scope.ServiceProvider.GetRequiredService<IdentityUserManager>();

        var result = await userManager.CallValidatePasswordAsync(
            new IdentityUser(Guid.NewGuid(), "policy", "policy@acme.test"), password);

        result.Succeeded.ShouldBe(accepted);
    }
}
