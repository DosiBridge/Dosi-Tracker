using System.Linq;
using System.Reflection;
using Microsoft.AspNetCore.Mvc;
using Shouldly;
using Volo.Abp.Account;
using Xunit;

namespace Dosi.Tracker.Account;

/// <summary>
/// The web app's "Change password" calls ABP's account endpoint (shared contract §7):
/// <c>POST /api/account/my-profile/change-password</c> with <c>{ currentPassword, newPassword }</c>. TrackerHttpApiModule
/// exposes it by depending on AbpAccountHttpApiModule; this guards the route and body shape against ABP upgrades.
/// </summary>
public class AccountRoutesTests
{
    [Fact]
    public void Self_Service_Password_Change_Should_Keep_Its_Route_And_Body()
    {
        typeof(ProfileController).GetCustomAttribute<RouteAttribute>()!.Template.ShouldBe("/api/account/my-profile");

        var changePassword = typeof(ProfileController).GetMethod(nameof(ProfileController.ChangePasswordAsync))!;
        changePassword.GetCustomAttribute<HttpPostAttribute>().ShouldNotBeNull();
        changePassword.GetCustomAttribute<RouteAttribute>()!.Template.ShouldBe("change-password");

        var body = changePassword.GetParameters().ShouldHaveSingleItem().ParameterType;
        body.ShouldBe(typeof(ChangePasswordInput));
        body.GetProperties().Select(p => p.Name).ShouldBe(new[] { "CurrentPassword", "NewPassword" }, ignoreOrder: true);
    }
}
