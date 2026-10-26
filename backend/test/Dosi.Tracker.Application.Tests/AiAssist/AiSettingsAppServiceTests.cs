using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Notifications;
using Dosi.Tracker.Workspaces;
using Shouldly;
using Volo.Abp;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// The owner's AI settings (<c>GET/PUT /api/app/ai-assist/settings</c>): who may read and change them, the disclosure
/// owners acknowledge, the consent history, and what members are told.
/// </summary>
public abstract class AiSettingsAppServiceTests<TStartupModule> : AiAssistTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private readonly IAiAssistAppService _service;

    protected AiSettingsAppServiceTests()
    {
        _service = GetRequiredService<IAiAssistAppService>();
    }

    private static UpdateAiWorkspaceSettingsDto Update(
        bool enabled = true, bool titles = false, bool notes = false, bool managers = true, string? ack = "1.0") => new()
    {
        IsEnabled = enabled,
        AllowManagers = managers,
        AllowMemberSelfService = true,
        AllowWindowTitles = titles,
        AllowNotes = notes,
        AcknowledgedDisclosureVersion = ack
    };

    private Task<List<Notification>> NotificationsAsync() =>
        WithUnitOfWorkAsync(() => GetRequiredService<IRepository<Notification, Guid>>().GetListAsync());

    private Task<List<AiConsentEvent>> ConsentEventsAsync() =>
        WithUnitOfWorkAsync(async () => (await GetRequiredService<IRepository<AiConsentEvent, Guid>>().GetListAsync())
            .OrderBy(e => e.CreationTime).ToList());

    [Fact]
    public async Task Owners_Should_Read_The_Settings_With_The_Server_Localized_Disclosure()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        AiOptions.Disclosure.ProcessingRegion = "  United States ";

        var settings = await _service.GetSettingsAsync();

        settings.IsEnabled.ShouldBeFalse();
        settings.AllowManagers.ShouldBeTrue();
        settings.AllowMemberSelfService.ShouldBeTrue();
        settings.AllowWindowTitles.ShouldBeFalse();
        settings.AllowNotes.ShouldBeFalse();
        settings.ConsentVersion.ShouldBeNull();
        settings.EnabledAt.ShouldBeNull();
        settings.EnabledByName.ShouldBeNull();

        var disclosure = settings.Disclosure;
        disclosure.Version.ShouldBe("1.0");
        disclosure.ProviderDisplayName.ShouldBe("OpenAI");
        disclosure.ProcessingRegion.ShouldBe("United States");
        disclosure.RetentionNote.ShouldBeNull();
        disclosure.AlwaysSent.Count.ShouldBe(AiAssistAppService.AlwaysSentKeys.Length);
        disclosure.SentWhenAllowed.Count.ShouldBe(AiAssistAppService.SentWhenAllowedKeys.Length);
        disclosure.NeverSent.Count.ShouldBe(AiAssistAppService.NeverSentKeys.Length);
        disclosure.WhoCanUse.Count.ShouldBe(AiAssistAppService.WhoCanUseKeys.Length);
        // Every entry is a real sentence from en.json, not a missing key.
        disclosure.AlwaysSent.Concat(disclosure.SentWhenAllowed).Concat(disclosure.NeverSent).Concat(disclosure.WhoCanUse)
            .ShouldAllBe(text => !text.StartsWith("AiAssist:") && text.EndsWith("."));
        disclosure.NeverSent.ShouldContain(text => text.Contains("Screenshots"));
        disclosure.NeverSent.ShouldContain(text => text.Contains("Keystrokes"));
        disclosure.SentWhenAllowed.ShouldContain(text => text.Contains("Window titles"));
    }

    [Fact]
    public async Task Managers_Should_Read_But_Never_Change_The_Settings_And_Members_Should_Do_Neither()
    {
        await SignInAsAsync(WorkspaceRoles.Manager);
        var member = await CreateMemberAsync("ai.member", WorkspaceRoles.Member);

        (await _service.GetSettingsAsync()).IsEnabled.ShouldBeFalse();
        await Should.ThrowAsync<AbpAuthorizationException>(() => _service.UpdateSettingsAsync(Update()));

        await AsUserAsync(member, async () =>
        {
            await Should.ThrowAsync<AbpAuthorizationException>(() => _service.GetSettingsAsync());
            await Should.ThrowAsync<AbpAuthorizationException>(() => _service.UpdateSettingsAsync(Update()));
        });

        (await ConsentEventsAsync()).ShouldBeEmpty();
        (await NotificationsAsync()).ShouldBeEmpty();
    }

    [Fact]
    public async Task Enabling_Without_Acknowledging_The_Current_Disclosure_Should_Be_Refused()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);

        foreach (var ack in new[] { null, "0.1" })
        {
            var refused = await Should.ThrowAsync<BusinessException>(() => _service.UpdateSettingsAsync(Update(ack: ack)));
            refused.Code.ShouldBe(TrackerDomainErrorCodes.AiDisclosureNotAccepted);
            refused.Data["version"].ShouldBe("1.0");
        }

        (await _service.GetSettingsAsync()).IsEnabled.ShouldBeFalse();
        (await NotificationsAsync()).ShouldBeEmpty();
    }

    [Fact]
    public async Task Enabling_Should_Record_The_Consent_And_Tell_Every_Active_Member()
    {
        await SignInAsAsync(WorkspaceRoles.Owner, name: "Olivia Owner");
        var manager = await CreateMemberAsync("ai.manager", WorkspaceRoles.Manager);
        var member = await CreateMemberAsync("ai.member", WorkspaceRoles.Member);

        var settings = await _service.UpdateSettingsAsync(Update());

        settings.IsEnabled.ShouldBeTrue();
        settings.ConsentVersion.ShouldBe("1.0");
        settings.EnabledAt.ShouldNotBeNull();
        settings.EnabledByName.ShouldBe("Olivia Owner");
        (await _service.GetSettingsAsync()).IsEnabled.ShouldBeTrue();
        (await _service.GetStatusAsync()).Available.ShouldBeTrue();

        var notifications = await NotificationsAsync();
        notifications.Select(n => n.UserId).ShouldBe(new[] { CallerId, manager, member }, ignoreOrder: true);
        notifications.ShouldAllBe(n => n.Message.StartsWith("AI assist is now on in this workspace.") && n.Message.Contains(" to OpenAI."));
        notifications.ShouldAllBe(n => n.Message.Length <= WorkspaceMemberNotifier.MaxMessageLength);
        notifications.ShouldAllBe(n => !n.Message.Contains("window titles"));

        var events = await ConsentEventsAsync();
        events.ShouldHaveSingleItem().Action.ShouldBe(AiConsentEvent.EnabledAction);
        events[0].UserId.ShouldBe(CallerId);
        events[0].DisclosureVersion.ShouldBe("1.0");
        Provider.Client.Calls.ShouldBeEmpty();
    }

    [Fact]
    public async Task Members_Should_Be_Told_When_Window_Titles_Or_Notes_Are_Added_But_Not_When_Things_Are_Turned_Off()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        await CreateMemberAsync("ai.member", WorkspaceRoles.Member);
        await _service.UpdateSettingsAsync(Update());

        await _service.UpdateSettingsAsync(Update(titles: true, notes: true));

        var added = (await NotificationsAsync()).Where(n => n.Message.StartsWith("AI assist in this workspace may now also send")).ToList();
        added.Count.ShouldBe(2);
        added.ShouldAllBe(n => n.Message.Contains("window titles and task notes to OpenAI"));

        var before = (await NotificationsAsync()).Count;
        await _service.UpdateSettingsAsync(Update(titles: false, notes: true, managers: false, ack: null));
        await _service.UpdateSettingsAsync(Update(enabled: false, ack: null));
        (await NotificationsAsync()).Count.ShouldBe(before);

        (await ConsentEventsAsync()).Select(e => e.Action).ShouldBe(new[]
        {
            AiConsentEvent.EnabledAction, AiConsentEvent.OptionsChangedAction, AiConsentEvent.OptionsChangedAction,
            AiConsentEvent.DisabledAction
        });
    }

    [Fact]
    public async Task Enabling_With_Window_Titles_Should_Say_So_In_The_Same_Notification()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);

        await _service.UpdateSettingsAsync(Update(titles: true));

        var notification = (await NotificationsAsync()).ShouldHaveSingleItem();
        notification.Message.ShouldStartWith("AI assist is now on in this workspace.");
        notification.Message.ShouldEndWith("The owner also allowed window titles.");
    }

    [Fact]
    public async Task Re_Consenting_To_A_Changed_Disclosure_Should_Tell_Members_Again()
    {
        await SignInAsAsync(WorkspaceRoles.Owner);
        await _service.UpdateSettingsAsync(Update());
        AiOptions.Disclosure.Version = 2;
        AiOptions.Disclosure.ProviderDisplayName = "Azure OpenAI (EU)";

        (await Should.ThrowAsync<BusinessException>(() => _service.UpdateSettingsAsync(Update()))).Data["version"].ShouldBe("1.2");
        var settings = await _service.UpdateSettingsAsync(Update(ack: "1.2"));

        settings.ConsentVersion.ShouldBe("1.2");
        settings.Disclosure.ProviderDisplayName.ShouldBe("Azure OpenAI (EU)");
        (await NotificationsAsync()).ShouldContain(n => n.Message.Contains("to Azure OpenAI (EU)."));
        (await ConsentEventsAsync()).Last().Action.ShouldBe(AiConsentEvent.ReconsentedAction);
    }

    [Fact]
    public async Task Turning_On_Notes_Alone_Should_Name_Notes_And_The_Provider_Should_Default_To_The_Configured_One()
    {
        // No display name configured: the provider's own name is shown.
        AiOptions.Disclosure.ProviderDisplayName = " ";
        await SignInAsAsync(WorkspaceRoles.Owner, name: " ");
        await _service.UpdateSettingsAsync(Update());

        var settings = await _service.UpdateSettingsAsync(Update(notes: true));

        settings.Disclosure.ProviderDisplayName.ShouldBe("Fake");
        // A blank display name falls back to the user name.
        settings.EnabledByName.ShouldBe("ai.caller");
        (await NotificationsAsync()).ShouldContain(n => n.Message.StartsWith("AI assist in this workspace may now also send task notes to Fake."));
    }
}
