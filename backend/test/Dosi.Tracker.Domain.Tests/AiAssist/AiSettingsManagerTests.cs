using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Settings;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.Configuration;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Options;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Data;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Modularity;
using Volo.Abp.SettingManagement;
using Volo.Abp.Settings;
using Xunit;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// AI assist's settings. Every test runs with hostile platform-wide values in the configuration ("Settings:..."), which
/// would switch AI assist on everywhere if anything but the workspace's own value counted.
/// </summary>
public abstract class AiSettingsManagerTests<TStartupModule> : TrackerDomainTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private static readonly Guid Owner = Guid.Parse("4b701e62-0953-4dd3-910b-dc6cc93ccb01");

    private readonly AiSettingsManager _manager;
    private readonly ISettingManager _settingManager;

    protected AiSettingsManagerTests()
    {
        _manager = GetRequiredService<AiSettingsManager>();
        _settingManager = GetRequiredService<ISettingManager>();
    }

    protected override void BeforeAddApplication(IServiceCollection services)
    {
        var configuration = new ConfigurationBuilder()
            .AddJsonFile("appsettings.json", false)
            .AddInMemoryCollection(new Dictionary<string, string?>
            {
                ["Settings:" + TrackerSettings.Workspace.Ai.IsEnabled] = "true",
                ["Settings:" + TrackerSettings.Workspace.Ai.AllowWindowTitles] = "true",
                ["Settings:" + TrackerSettings.Workspace.Ai.AllowNotes] = "true",
                ["Settings:" + TrackerSettings.Workspace.Ai.ConsentVersion] = "1.0",
                ["Settings:" + TrackerSettings.Platform.AiEnabled] = "false"
            })
            .Build();
        services.ReplaceConfiguration(configuration);
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        services.DisableStaticSettingSaving();
    }

    private static AiWorkspaceSettingsUpdate On(
        bool managers = true, bool selfService = true, bool titles = false, bool notes = false) =>
        new(true, managers, selfService, titles, notes);

    private static AiWorkspaceSettingsUpdate Off(
        bool managers = true, bool selfService = true, bool titles = false, bool notes = false) =>
        new(false, managers, selfService, titles, notes);

    private Task<AiWorkspacePolicy> PolicyAsync(Guid workspaceId) => WithUnitOfWorkAsync(() => _manager.GetPolicyAsync(workspaceId));

    private Task<AiSettingsChange> UpdateAsync(Guid workspaceId, AiWorkspaceSettingsUpdate update, string? ack) =>
        WithUnitOfWorkAsync(() => _manager.UpdateAsync(workspaceId, Owner, update, ack));

    private Task SetWorkspaceValueAsync(Guid workspaceId, string name, string value) =>
        WithUnitOfWorkAsync(() => _settingManager.SetAsync(
            name, value, WorkspaceSettingValueProvider.ProviderName, workspaceId.ToString(), forceToSet: true));

    /// <summary>Global setting rows written straight into the store (ABP's setting manager refuses them).</summary>
    private Task InsertGlobalRowsAsync(params (string Name, string Value)[] rows) =>
        WithUnitOfWorkAsync(async () =>
        {
            var repository = GetRequiredService<ISettingRepository>();
            foreach (var (name, value) in rows)
            {
                await repository.InsertAsync(new Setting(Guid.NewGuid(), name, value, GlobalSettingValueProvider.ProviderName), autoSave: true);
            }
        });

    private Task<List<AiConsentEvent>> EventsAsync(Guid workspaceId) =>
        WithUnitOfWorkAsync(async () =>
        {
            using (GetRequiredService<IDataFilter>().Disable<IWorkspaceScoped>())
            {
                return (await GetRequiredService<IRepository<AiConsentEvent, Guid>>().GetListAsync(e => e.WorkspaceId == workspaceId))
                    .OrderBy(e => e.CreationTime)
                    .ToList();
            }
        });

    [Fact]
    public async Task A_Workspace_Whose_Owner_Never_Chose_Should_Have_AI_Off_With_The_Code_Defaults()
    {
        var policy = await PolicyAsync(Guid.NewGuid());

        policy.ShouldBe(AiWorkspacePolicy.Default);
        policy.IsEnabled.ShouldBeFalse();
        policy.AllowManagers.ShouldBeTrue();
        policy.AllowMemberSelfService.ShouldBeTrue();
        policy.AllowWindowTitles.ShouldBeFalse();
        policy.AllowNotes.ShouldBeFalse();
        policy.IsActive(_manager.CurrentDisclosureVersion).ShouldBeFalse();
        _manager.CurrentDisclosureVersion.ShouldBe("1.0");
    }

    [Fact]
    public async Task Global_Or_Configured_Values_Should_Never_Switch_AI_On_For_A_Workspace()
    {
        // ABP refuses to store a global value for these settings at all ...
        var refused = await Should.ThrowAsync<AbpException>(() => WithUnitOfWorkAsync(() =>
            _settingManager.SetGlobalAsync(TrackerSettings.Workspace.Ai.IsEnabled, "true")));
        refused.Message.ShouldContain("not compatible");

        // ... and one that got into the store anyway (a raw row), like the configuration's "on" (see the class remarks),
        // is never read for a workspace.
        await InsertGlobalRowsAsync(
            (TrackerSettings.Workspace.Ai.IsEnabled, "true"),
            (TrackerSettings.Workspace.Ai.AllowNotes, "true"),
            (TrackerSettings.Workspace.Ai.ConsentVersion, "1.0"));

        (await PolicyAsync(Guid.NewGuid())).ShouldBe(AiWorkspacePolicy.Default);
    }

    [Theory]
    [InlineData("yes")]
    [InlineData("1")]
    [InlineData("on")]
    public async Task Malformed_Values_Should_Read_As_Off(string value)
    {
        var workspaceId = Guid.NewGuid();
        foreach (var name in new[]
                 {
                     TrackerSettings.Workspace.Ai.IsEnabled, TrackerSettings.Workspace.Ai.AllowManagers,
                     TrackerSettings.Workspace.Ai.AllowMemberSelfService, TrackerSettings.Workspace.Ai.AllowWindowTitles,
                     TrackerSettings.Workspace.Ai.AllowNotes
                 })
        {
            await SetWorkspaceValueAsync(workspaceId, name, value);
        }

        await SetWorkspaceValueAsync(workspaceId, TrackerSettings.Workspace.Ai.EnabledAt, "not a time");
        await SetWorkspaceValueAsync(workspaceId, TrackerSettings.Workspace.Ai.EnabledBy, "not a user");

        var policy = await PolicyAsync(workspaceId);

        policy.ShouldBe(new AiWorkspacePolicy(false, false, false, false, false, null, null, null));
    }

    [Fact]
    public async Task Enabling_Should_Need_The_Current_Disclosure_And_Record_Who_Consented_When()
    {
        var workspaceId = Guid.NewGuid();

        foreach (var ack in new[] { null, "", "0.9", "1.1", "1" })
        {
            var refused = await Should.ThrowAsync<BusinessException>(() => UpdateAsync(workspaceId, On(), ack));
            refused.Code.ShouldBe(TrackerDomainErrorCodes.AiDisclosureNotAccepted);
            refused.Data["version"].ShouldBe("1.0");
        }

        (await PolicyAsync(workspaceId)).IsEnabled.ShouldBeFalse();
        (await EventsAsync(workspaceId)).ShouldBeEmpty();

        var before = DateTime.UtcNow.AddSeconds(-1);
        var change = await UpdateAsync(workspaceId, On(), " 1.0 ");

        change.ConsentAction.ShouldBe(AiConsentEvent.EnabledAction);
        change.NotifyMembers.ShouldBeTrue();
        var policy = await PolicyAsync(workspaceId);
        policy.ShouldBe(change.Policy);
        policy.IsActive("1.0").ShouldBeTrue();
        policy.ConsentVersion.ShouldBe("1.0");
        policy.EnabledBy.ShouldBe(Owner);
        policy.EnabledAt.ShouldNotBeNull().Kind.ShouldBe(DateTimeKind.Utc);
        policy.EnabledAt.Value.ShouldBeGreaterThan(before);

        // Another workspace is untouched.
        (await PolicyAsync(Guid.NewGuid())).ShouldBe(AiWorkspacePolicy.Default);
    }

    [Fact]
    public async Task Turning_On_Window_Titles_Or_Notes_Should_Need_The_Disclosure_But_Turning_Things_Off_Should_Not()
    {
        var workspaceId = Guid.NewGuid();
        await UpdateAsync(workspaceId, On(), "1.0");

        foreach (var update in new[] { On(titles: true), On(notes: true), Off(titles: true) })
        {
            (await Should.ThrowAsync<BusinessException>(() => UpdateAsync(workspaceId, update, null)))
                .Code.ShouldBe(TrackerDomainErrorCodes.AiDisclosureNotAccepted);
        }

        var titles = await UpdateAsync(workspaceId, On(titles: true, notes: true), "1.0");
        titles.ConsentAction.ShouldBe(AiConsentEvent.OptionsChangedAction);
        titles.NotifyMembers.ShouldBeTrue();
        titles.WindowTitlesAdded.ShouldBeTrue();
        titles.NotesAdded.ShouldBeTrue();

        // Narrowing needs no acknowledgement and tells nobody.
        var narrowed = await UpdateAsync(workspaceId, On(managers: false, selfService: false, titles: false, notes: true), null);
        narrowed.ConsentAction.ShouldBe(AiConsentEvent.OptionsChangedAction);
        narrowed.NotifyMembers.ShouldBeFalse();
        (await PolicyAsync(workspaceId)).ShouldBe(narrowed.Policy);
        narrowed.Policy.AllowManagers.ShouldBeFalse();
        narrowed.Policy.AllowMemberSelfService.ShouldBeFalse();

        var disabled = await UpdateAsync(workspaceId, Off(), null);
        disabled.ConsentAction.ShouldBe(AiConsentEvent.DisabledAction);
        disabled.NotifyMembers.ShouldBeFalse();
        (await PolicyAsync(workspaceId)).IsActive("1.0").ShouldBeFalse();

        // Allowing titles while AI assist is off is acknowledged but tells nobody: nothing is sent yet.
        var offTitles = await UpdateAsync(workspaceId, Off(titles: true), "1.0");
        offTitles.NotifyMembers.ShouldBeFalse();
        offTitles.WindowTitlesAdded.ShouldBeFalse();
    }

    [Fact]
    public async Task A_Changed_Disclosure_Should_Make_Consent_Stale_Until_The_Owner_Consents_Again()
    {
        var workspaceId = Guid.NewGuid();
        await UpdateAsync(workspaceId, On(titles: true), "1.0");

        // The operator changes the disclosure (AI:Disclosure:Version).
        GetRequiredService<IOptions<AiOptions>>().Value.Disclosure.Version = 2;
        _manager.CurrentDisclosureVersion.ShouldBe("1.2");

        var stale = await PolicyAsync(workspaceId);
        stale.IsEnabled.ShouldBeTrue();
        stale.IsActive("1.2").ShouldBeFalse();
        stale.NeedsReconsent("1.2").ShouldBeTrue();

        // Keeping it on is re-consenting, which needs the new version.
        var refused = await Should.ThrowAsync<BusinessException>(() => UpdateAsync(workspaceId, On(titles: true), "1.0"));
        refused.Data["version"].ShouldBe("1.2");

        var change = await UpdateAsync(workspaceId, On(titles: true), "1.2");

        change.ConsentAction.ShouldBe(AiConsentEvent.ReconsentedAction);
        change.NotifyMembers.ShouldBeTrue();
        change.WindowTitlesAdded.ShouldBeTrue();
        (await PolicyAsync(workspaceId)).IsActive("1.2").ShouldBeTrue();

        // Turning it off under a stale consent needs nothing.
        GetRequiredService<IOptions<AiOptions>>().Value.Disclosure.Version = 3;
        (await UpdateAsync(workspaceId, Off(titles: true), null)).ConsentAction.ShouldBe(AiConsentEvent.DisabledAction);
    }

    [Fact]
    public async Task Every_Change_Should_Be_Appended_To_The_Consent_History_And_A_Request_That_Changes_Nothing_Should_Not()
    {
        var workspaceId = Guid.NewGuid();

        // Changing options while off is recorded too.
        await UpdateAsync(workspaceId, Off(managers: false), null);
        await UpdateAsync(workspaceId, On(managers: false), "1.0");
        (await UpdateAsync(workspaceId, On(managers: false), null)).ConsentAction.ShouldBeNull();
        await UpdateAsync(workspaceId, On(managers: false, notes: true), "1.0");
        await UpdateAsync(workspaceId, Off(managers: false, notes: true), null);
        (await UpdateAsync(workspaceId, Off(managers: false, notes: true), null)).ConsentAction.ShouldBeNull();

        var events = await EventsAsync(workspaceId);

        events.Select(e => e.Action).ShouldBe(new[]
        {
            AiConsentEvent.OptionsChangedAction, AiConsentEvent.EnabledAction, AiConsentEvent.OptionsChangedAction,
            AiConsentEvent.DisabledAction
        });
        events.Select(e => e.Flags).ShouldBe(new[]
        {
            "managers=0;selfService=1;windowTitles=0;notes=0", "managers=0;selfService=1;windowTitles=0;notes=0",
            "managers=0;selfService=1;windowTitles=0;notes=1", "managers=0;selfService=1;windowTitles=0;notes=1"
        });
        events.ShouldAllBe(e => e.UserId == Owner && e.DisclosureVersion == "1.0" && e.WorkspaceId == workspaceId);

        // Off keeps the record of the last consent.
        var policy = await PolicyAsync(workspaceId);
        policy.IsEnabled.ShouldBeFalse();
        policy.ConsentVersion.ShouldBe("1.0");
        policy.EnabledBy.ShouldBe(Owner);
    }

    [Fact]
    public async Task A_Workspace_Value_Equal_To_A_Platform_Value_Should_Still_Be_Kept()
    {
        var workspaceId = Guid.NewGuid();
        await InsertGlobalRowsAsync((TrackerSettings.Workspace.Ai.AllowManagers, "false"));

        // The owner's values are stored as chosen, whatever a platform-wide row or the configuration says.
        await UpdateAsync(workspaceId, On(managers: false, titles: true, notes: true), "1.0");

        var policy = await PolicyAsync(workspaceId);
        policy.IsEnabled.ShouldBeTrue();
        policy.AllowManagers.ShouldBeFalse();
        policy.AllowWindowTitles.ShouldBeTrue();
        policy.AllowNotes.ShouldBeTrue();
    }

    [Fact]
    public async Task The_Platform_Switch_Should_Be_On_Unless_Its_Global_Value_Says_Otherwise()
    {
        // Unset (the configuration's "false" is not a global value): on.
        (await WithUnitOfWorkAsync(() => _manager.IsPlatformEnabledAsync())).ShouldBeTrue();

        await WithUnitOfWorkAsync(() => _manager.SetPlatformEnabledAsync(false));
        (await WithUnitOfWorkAsync(() => _manager.IsPlatformEnabledAsync())).ShouldBeFalse();

        await WithUnitOfWorkAsync(() => _manager.SetPlatformEnabledAsync(true));
        (await WithUnitOfWorkAsync(() => _manager.IsPlatformEnabledAsync())).ShouldBeTrue();

        foreach (var malformed in new[] { "maybe", "0" })
        {
            await WithUnitOfWorkAsync(() => _settingManager.SetAsync(
                TrackerSettings.Platform.AiEnabled, malformed, GlobalSettingValueProvider.ProviderName, null, forceToSet: true));
            (await WithUnitOfWorkAsync(() => _manager.IsPlatformEnabledAsync())).ShouldBeFalse();
        }
    }
}
