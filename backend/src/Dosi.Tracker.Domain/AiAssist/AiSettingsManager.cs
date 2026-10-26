using System;
using System.Globalization;
using System.Threading.Tasks;
using Dosi.Tracker.Settings;
using Microsoft.Extensions.Options;
using Volo.Abp;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.SettingManagement;
using Volo.Abp.Settings;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// AI assist's settings: each workspace's opt-in (owner-managed, with its consent history) and the platform's runtime
/// switch. Reads fail closed: only the workspace's own stored value counts (a global or configured value never turns AI
/// assist on for a workspace), and an unset or malformed value reads as its code default, which is off for everything
/// that sends data. The settings are defined for the workspace (or, for the platform switch, the global) provider only,
/// so ABP refuses a value for any other provider. Writes always store the value (<c>forceToSet</c>): ABP would otherwise
/// drop a value that equals its fallback, and the owner's explicit choice would then depend on defaults.
/// </summary>
public class AiSettingsManager : TrackerDomainService
{
    private readonly ISettingManager _settingManager;
    private readonly IRepository<AiConsentEvent, Guid> _consentEventRepository;
    private readonly IOptions<AiOptions> _options;

    public AiSettingsManager(
        ISettingManager settingManager,
        IRepository<AiConsentEvent, Guid> consentEventRepository,
        IOptions<AiOptions> options)
    {
        _settingManager = settingManager;
        _consentEventRepository = consentEventRepository;
        _options = options;
    }

    /// <summary>The disclosure version owners consent to: "{code}.{operator}" (<see cref="AiAssistConsts.CodeDisclosureVersion"/>
    /// and <c>AI:Disclosure:Version</c>). Raising either asks every owner to consent again.</summary>
    public virtual string CurrentDisclosureVersion =>
        string.Create(CultureInfo.InvariantCulture, $"{AiAssistConsts.CodeDisclosureVersion}.{_options.Value.Disclosure.Version}");

    /// <summary>The workspace's AI assist settings (see the class remarks on how they are read).</summary>
    public virtual async Task<AiWorkspacePolicy> GetPolicyAsync(Guid workspaceId)
    {
        var defaults = AiWorkspacePolicy.Default;
        return new AiWorkspacePolicy(
            IsEnabled: ReadBool(await GetAsync(workspaceId, TrackerSettings.Workspace.Ai.IsEnabled), defaults.IsEnabled),
            AllowManagers: ReadBool(await GetAsync(workspaceId, TrackerSettings.Workspace.Ai.AllowManagers), defaults.AllowManagers),
            AllowMemberSelfService: ReadBool(
                await GetAsync(workspaceId, TrackerSettings.Workspace.Ai.AllowMemberSelfService), defaults.AllowMemberSelfService),
            AllowWindowTitles: ReadBool(
                await GetAsync(workspaceId, TrackerSettings.Workspace.Ai.AllowWindowTitles), defaults.AllowWindowTitles),
            AllowNotes: ReadBool(await GetAsync(workspaceId, TrackerSettings.Workspace.Ai.AllowNotes), defaults.AllowNotes),
            ConsentVersion: NullIfBlank(await GetAsync(workspaceId, TrackerSettings.Workspace.Ai.ConsentVersion)),
            EnabledAt: ReadInstant(await GetAsync(workspaceId, TrackerSettings.Workspace.Ai.EnabledAt)),
            EnabledBy: Guid.TryParse(await GetAsync(workspaceId, TrackerSettings.Workspace.Ai.EnabledBy), out var enabledBy)
                ? enabledBy
                : null);
    }

    /// <summary>
    /// Applies an owner's change (the caller checks that <paramref name="userId"/> is an owner). Enabling, re-consenting
    /// to a changed disclosure, or turning on window titles or notes requires <paramref name="acknowledgedVersion"/> to
    /// be <see cref="CurrentDisclosureVersion"/>; otherwise <c>Tracker:AiDisclosureNotAccepted</c> (data.version). Every
    /// change is appended to the consent history; a request that changes nothing records nothing.
    /// </summary>
    public virtual async Task<AiSettingsChange> UpdateAsync(
        Guid workspaceId,
        Guid userId,
        AiWorkspaceSettingsUpdate update,
        string? acknowledgedVersion)
    {
        Check.NotNull(update, nameof(update));
        var current = await GetPolicyAsync(workspaceId);
        var version = CurrentDisclosureVersion;

        var enabling = update.IsEnabled && !current.IsEnabled;
        var reconsenting = update.IsEnabled && current.NeedsReconsent(version);
        var titlesTurnedOn = update.AllowWindowTitles && !current.AllowWindowTitles;
        var notesTurnedOn = update.AllowNotes && !current.AllowNotes;

        if ((enabling || reconsenting || titlesTurnedOn || notesTurnedOn) &&
            !string.Equals(acknowledgedVersion?.Trim(), version, StringComparison.Ordinal))
        {
            throw new BusinessException(TrackerDomainErrorCodes.AiDisclosureNotAccepted).WithData("version", version);
        }

        await SetAsync(workspaceId, TrackerSettings.Workspace.Ai.IsEnabled, SettingValueFormat.Write(update.IsEnabled));
        await SetAsync(workspaceId, TrackerSettings.Workspace.Ai.AllowManagers, SettingValueFormat.Write(update.AllowManagers));
        await SetAsync(workspaceId, TrackerSettings.Workspace.Ai.AllowMemberSelfService,
            SettingValueFormat.Write(update.AllowMemberSelfService));
        await SetAsync(workspaceId, TrackerSettings.Workspace.Ai.AllowWindowTitles, SettingValueFormat.Write(update.AllowWindowTitles));
        await SetAsync(workspaceId, TrackerSettings.Workspace.Ai.AllowNotes, SettingValueFormat.Write(update.AllowNotes));

        var consentVersion = current.ConsentVersion;
        var enabledAt = current.EnabledAt;
        var enabledBy = current.EnabledBy;
        if (enabling || reconsenting)
        {
            consentVersion = version;
            enabledAt = Clock.Now;
            enabledBy = userId;
            await SetAsync(workspaceId, TrackerSettings.Workspace.Ai.ConsentVersion, consentVersion);
            await SetAsync(workspaceId, TrackerSettings.Workspace.Ai.EnabledAt,
                DateTime.SpecifyKind(enabledAt.Value, DateTimeKind.Utc).ToString("O", CultureInfo.InvariantCulture));
            await SetAsync(workspaceId, TrackerSettings.Workspace.Ai.EnabledBy, userId.ToString());
        }

        var policy = new AiWorkspacePolicy(
            update.IsEnabled,
            update.AllowManagers,
            update.AllowMemberSelfService,
            update.AllowWindowTitles,
            update.AllowNotes,
            consentVersion,
            enabledAt,
            enabledBy);

        var action = enabling ? AiConsentEvent.EnabledAction
            : !update.IsEnabled && current.IsEnabled ? AiConsentEvent.DisabledAction
            : reconsenting ? AiConsentEvent.ReconsentedAction
            : policy.Flags != current.Flags ? AiConsentEvent.OptionsChangedAction
            : null;
        if (action != null)
        {
            await _consentEventRepository.InsertAsync(
                new AiConsentEvent(GuidGenerator.Create(), workspaceId, userId, action, version, policy.Flags));
        }

        // Members hear about it when AI assist starts (or restarts under a changed disclosure) and whenever it may send
        // more of their data than before.
        var turnedOn = enabling || reconsenting;
        var titlesAdded = update.IsEnabled && update.AllowWindowTitles && (titlesTurnedOn || turnedOn);
        var notesAdded = update.IsEnabled && update.AllowNotes && (notesTurnedOn || turnedOn);
        var notify = turnedOn || (update.IsEnabled && (titlesTurnedOn || notesTurnedOn));

        return new AiSettingsChange(policy, action, notify, titlesAdded, notesAdded);
    }

    /// <summary>The platform's runtime switch (<see cref="TrackerSettings.Platform.AiEnabled"/>, global value only): on
    /// when unset, off when set to anything but a boolean.</summary>
    public virtual async Task<bool> IsPlatformEnabledAsync()
    {
        var value = await _settingManager.GetOrNullAsync(
            TrackerSettings.Platform.AiEnabled, GlobalSettingValueProvider.ProviderName, null, fallback: false);
        return value == null || ReadBool(value, fallback: false);
    }

    /// <summary>Turns AI assist on or off for the whole platform at once (each API instance sees it as soon as its setting
    /// cache does).</summary>
    public virtual Task SetPlatformEnabledAsync(bool enabled)
    {
        return _settingManager.SetAsync(
            TrackerSettings.Platform.AiEnabled, SettingValueFormat.Write(enabled), GlobalSettingValueProvider.ProviderName,
            null, forceToSet: true);
    }

    private Task<string?> GetAsync(Guid workspaceId, string name)
    {
        return _settingManager.GetOrNullAsync(name, WorkspaceSettingValueProvider.ProviderName, workspaceId.ToString(), fallback: false);
    }

    private Task SetAsync(Guid workspaceId, string name, string value)
    {
        return _settingManager.SetAsync(name, value, WorkspaceSettingValueProvider.ProviderName, workspaceId.ToString(), forceToSet: true);
    }

    /// <summary>The stored flag, <paramref name="fallback"/> when unset, and false (never the fallback) when malformed.</summary>
    private static bool ReadBool(string? value, bool fallback)
    {
        if (value == null)
        {
            return fallback;
        }

        return bool.TryParse(value, out var flag) && flag;
    }

    private static DateTime? ReadInstant(string? value)
    {
        return DateTime.TryParse(value, CultureInfo.InvariantCulture,
            DateTimeStyles.AdjustToUniversal | DateTimeStyles.AssumeUniversal, out var instant)
            ? DateTime.SpecifyKind(instant, DateTimeKind.Utc)
            : null;
    }

    private static string? NullIfBlank(string? value) => string.IsNullOrWhiteSpace(value) ? null : value.Trim();
}
