using System;
using System.Collections.Generic;
using System.Security.Claims;
using System.Threading.Tasks;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Modularity;
using Dosi.Tracker.Workspaces;
using Volo.Abp.Security.Claims;
using Xunit;

namespace Dosi.Tracker.Settings;

public abstract class WorkspaceSettingsManagerTests<TStartupModule> : TrackerDomainTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly WorkspaceSettingsManager _settings;
    private readonly ICurrentWorkspace _currentWorkspace;
    private readonly ICurrentPrincipalAccessor _principalAccessor;

    protected WorkspaceSettingsManagerTests()
    {
        _settings = GetRequiredService<WorkspaceSettingsManager>();
        _currentWorkspace = GetRequiredService<ICurrentWorkspace>();
        _principalAccessor = GetRequiredService<ICurrentPrincipalAccessor>();
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
    }

    private async Task<T> InWorkspaceAsync<T>(Guid? tenantId, Func<Task<T>> action)
    {
        using (_currentWorkspace.Change(tenantId))
        {
            return await WithUnitOfWorkAsync(action);
        }
    }

    private async Task InWorkspaceAsync(Guid? tenantId, Func<Task> action)
    {
        using (_currentWorkspace.Change(tenantId))
        {
            await WithUnitOfWorkAsync(action);
        }
    }

    private IDisposable ActAs(Guid userId)
    {
        return _principalAccessor.Change(new ClaimsPrincipal(new ClaimsIdentity(
            new List<Claim> { new(AbpClaimTypes.UserId, userId.ToString()) }, "Test")));
    }

    [Fact]
    public async Task A_Workspace_Without_Settings_Should_Get_The_Platform_Defaults()
    {
        var tenantId = Guid.NewGuid();

        var defaults = await InWorkspaceAsync(tenantId, () => _settings.GetNewProjectDefaultsAsync());

        defaults.ShouldBeEquivalentTo(new NewProjectDefaults());
        defaults.IntervalMinutes.ShouldBe(10);
        defaults.AllowWebcam.ShouldBeFalse();
        defaults.IdleThresholdMinutes.ShouldBe(5);
        (await InWorkspaceAsync(tenantId, () => _settings.GetDefaultTimeZoneAsync())).ShouldBeNull();
    }

    [Fact]
    public async Task New_Project_Defaults_Should_Be_Kept_Per_Workspace()
    {
        var tenantA = Guid.NewGuid();
        var tenantB = Guid.NewGuid();
        var custom = new NewProjectDefaults
        {
            IntervalMinutes = 30,
            AllowScreenshot = false,
            AllowWebcam = true,
            AllowKeyboard = false,
            AllowMouse = false,
            AllowActiveWindow = false,
            AllowRunningPrograms = false,
            BlurScreenshots = true,
            IdleThresholdMinutes = 0
        };

        await InWorkspaceAsync(tenantA, () => _settings.SetNewProjectDefaultsAsync(custom));

        (await InWorkspaceAsync(tenantA, () => _settings.GetNewProjectDefaultsAsync())).ShouldBeEquivalentTo(custom);
        (await InWorkspaceAsync(tenantB, () => _settings.GetNewProjectDefaultsAsync())).ShouldBeEquivalentTo(new NewProjectDefaults());
    }

    [Fact]
    public async Task Going_Back_To_The_Defaults_Should_Restore_Them()
    {
        var tenantId = Guid.NewGuid();
        await InWorkspaceAsync(tenantId, () => _settings.SetNewProjectDefaultsAsync(new NewProjectDefaults { IntervalMinutes = 45, BlurScreenshots = true }));

        await InWorkspaceAsync(tenantId, () => _settings.SetNewProjectDefaultsAsync(new NewProjectDefaults()));

        (await InWorkspaceAsync(tenantId, () => _settings.GetNewProjectDefaultsAsync())).ShouldBeEquivalentTo(new NewProjectDefaults());
    }

    [Theory]
    [InlineData(4, 5)]
    [InlineData(61, 5)]
    [InlineData(10, -1)]
    [InlineData(10, 61)]
    public async Task Out_Of_Range_Defaults_Should_Be_Refused(int intervalMinutes, int idleThresholdMinutes)
    {
        await Should.ThrowAsync<ArgumentException>(() => InWorkspaceAsync(Guid.NewGuid(), () =>
            _settings.SetNewProjectDefaultsAsync(new NewProjectDefaults
            {
                IntervalMinutes = intervalMinutes,
                IdleThresholdMinutes = idleThresholdMinutes
            })));
    }

    [Fact]
    public async Task The_Default_Time_Zone_Should_Be_Kept_Per_Workspace_And_Can_Be_Unset()
    {
        var tenantA = Guid.NewGuid();
        var tenantB = Guid.NewGuid();

        await InWorkspaceAsync(tenantA, () => _settings.SetDefaultTimeZoneAsync("Asia/Dhaka"));

        (await InWorkspaceAsync(tenantA, () => _settings.GetDefaultTimeZoneAsync())).ShouldBe("Asia/Dhaka");
        (await InWorkspaceAsync(tenantB, () => _settings.GetDefaultTimeZoneAsync())).ShouldBeNull();

        await InWorkspaceAsync(tenantA, () => _settings.SetDefaultTimeZoneAsync(null));
        (await InWorkspaceAsync(tenantA, () => _settings.GetDefaultTimeZoneAsync())).ShouldBeNull();
    }

    [Theory]
    [InlineData("Mars/Olympus_Mons")]
    [InlineData("Bangladesh Standard Time")] // must be normalized (to Asia/Dhaka) before it is stored
    [InlineData(" Asia/Dhaka")]
    public async Task A_Time_Zone_That_Is_Not_A_Normalized_Iana_Id_Should_Be_Refused(string timeZoneId)
    {
        await Should.ThrowAsync<ArgumentException>(
            () => InWorkspaceAsync(Guid.NewGuid(), () => _settings.SetDefaultTimeZoneAsync(timeZoneId)));
        await Should.ThrowAsync<ArgumentException>(
            () => InWorkspaceAsync(Guid.NewGuid(), () => _settings.SetUserTimeZoneAsync(timeZoneId)));
    }

    [Fact]
    public async Task The_User_Time_Zone_Should_Be_Kept_Per_User_And_Override_The_Workspace_Default()
    {
        var tenantId = Guid.NewGuid();
        var alice = Guid.NewGuid();
        var bob = Guid.NewGuid();
        await InWorkspaceAsync(tenantId, () => _settings.SetDefaultTimeZoneAsync("Europe/London"));

        using (ActAs(alice))
        {
            await InWorkspaceAsync(tenantId, () => _settings.SetUserTimeZoneAsync("Asia/Dhaka"));
            (await InWorkspaceAsync(tenantId, () => _settings.GetUserTimeZoneAsync())).ShouldBe("Asia/Dhaka");
            (await InWorkspaceAsync(tenantId, () => _settings.GetEffectiveTimeZoneAsync())).ShouldBe("Asia/Dhaka");
        }

        using (ActAs(bob))
        {
            (await InWorkspaceAsync(tenantId, () => _settings.GetUserTimeZoneAsync())).ShouldBeNull();
            (await InWorkspaceAsync(tenantId, () => _settings.GetEffectiveTimeZoneAsync())).ShouldBe("Europe/London");
        }

        using (ActAs(alice))
        {
            await InWorkspaceAsync(tenantId, () => _settings.SetUserTimeZoneAsync(null));
            (await InWorkspaceAsync(tenantId, () => _settings.GetEffectiveTimeZoneAsync())).ShouldBe("Europe/London");
        }
    }

    [Fact]
    public async Task Without_Any_Time_Zone_The_Effective_Zone_Should_Be_Unset()
    {
        (await InWorkspaceAsync(Guid.NewGuid(), () => _settings.GetEffectiveTimeZoneAsync())).ShouldBeNull();
    }
}
