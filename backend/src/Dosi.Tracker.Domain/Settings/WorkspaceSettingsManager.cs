using System;
using System.Threading.Tasks;
using Volo.Abp;
using Dosi.Tracker.Workspaces;
using Volo.Abp.SettingManagement;

namespace Dosi.Tracker.Settings;

/// <summary>
/// The current workspace's settings (workspace-level ABP settings, provider "W": the default time zone and the new-project defaults)
/// and the current user's preferences (user-level: their time zone). Values are stored only when they differ from
/// the platform default, so a workspace keeps following defaults it never changed.
/// </summary>
public class WorkspaceSettingsManager : TrackerDomainService
{
    private readonly ISettingManager _settingManager;

    public WorkspaceSettingsManager(ISettingManager settingManager)
    {
        _settingManager = settingManager;
    }

    /// <summary>The workspace's default reporting time zone (an IANA id), or null when not set.</summary>
    public virtual async Task<string?> GetDefaultTimeZoneAsync()
    {
        return await GetOrNullAsync(TrackerSettings.Workspace.DefaultTimeZone);
    }

    /// <param name="timeZoneId">An IANA id (see <see cref="TimeZoneIds.TryNormalize"/>), or null to unset it.</param>
    public virtual Task SetDefaultTimeZoneAsync(string? timeZoneId)
    {
        return SetAsync(TrackerSettings.Workspace.DefaultTimeZone, CheckTimeZone(timeZoneId));
    }

    public virtual async Task<NewProjectDefaults> GetNewProjectDefaultsAsync()
    {
        var fallback = new NewProjectDefaults();
        return new NewProjectDefaults
        {
            IntervalMinutes = await GetIntAsync(TrackerSettings.Workspace.NewProject.IntervalMinutes, fallback.IntervalMinutes),
            AllowScreenshot = await GetBoolAsync(TrackerSettings.Workspace.NewProject.AllowScreenshot, fallback.AllowScreenshot),
            AllowWebcam = await GetBoolAsync(TrackerSettings.Workspace.NewProject.AllowWebcam, fallback.AllowWebcam),
            AllowKeyboard = await GetBoolAsync(TrackerSettings.Workspace.NewProject.AllowKeyboard, fallback.AllowKeyboard),
            AllowMouse = await GetBoolAsync(TrackerSettings.Workspace.NewProject.AllowMouse, fallback.AllowMouse),
            AllowActiveWindow = await GetBoolAsync(TrackerSettings.Workspace.NewProject.AllowActiveWindow, fallback.AllowActiveWindow),
            AllowRunningPrograms = await GetBoolAsync(TrackerSettings.Workspace.NewProject.AllowRunningPrograms, fallback.AllowRunningPrograms),
            BlurScreenshots = await GetBoolAsync(TrackerSettings.Workspace.NewProject.BlurScreenshots, fallback.BlurScreenshots),
            IdleThresholdMinutes = await GetIntAsync(TrackerSettings.Workspace.NewProject.IdleThresholdMinutes, fallback.IdleThresholdMinutes)
        };
    }

    public virtual async Task SetNewProjectDefaultsAsync(NewProjectDefaults defaults)
    {
        Check.NotNull(defaults, nameof(defaults));
        Check.Range(defaults.IntervalMinutes, nameof(defaults.IntervalMinutes),
            NewProjectDefaults.MinIntervalMinutes, NewProjectDefaults.MaxIntervalMinutes);
        Check.Range(defaults.IdleThresholdMinutes, nameof(defaults.IdleThresholdMinutes),
            0, NewProjectDefaults.MaxIdleThresholdMinutes);

        await SetAsync(TrackerSettings.Workspace.NewProject.IntervalMinutes, SettingValueFormat.Write(defaults.IntervalMinutes));
        await SetAsync(TrackerSettings.Workspace.NewProject.AllowScreenshot, SettingValueFormat.Write(defaults.AllowScreenshot));
        await SetAsync(TrackerSettings.Workspace.NewProject.AllowWebcam, SettingValueFormat.Write(defaults.AllowWebcam));
        await SetAsync(TrackerSettings.Workspace.NewProject.AllowKeyboard, SettingValueFormat.Write(defaults.AllowKeyboard));
        await SetAsync(TrackerSettings.Workspace.NewProject.AllowMouse, SettingValueFormat.Write(defaults.AllowMouse));
        await SetAsync(TrackerSettings.Workspace.NewProject.AllowActiveWindow, SettingValueFormat.Write(defaults.AllowActiveWindow));
        await SetAsync(TrackerSettings.Workspace.NewProject.AllowRunningPrograms, SettingValueFormat.Write(defaults.AllowRunningPrograms));
        await SetAsync(TrackerSettings.Workspace.NewProject.BlurScreenshots, SettingValueFormat.Write(defaults.BlurScreenshots));
        await SetAsync(TrackerSettings.Workspace.NewProject.IdleThresholdMinutes, SettingValueFormat.Write(defaults.IdleThresholdMinutes));
    }

    /// <summary>The current user's own time zone preference (an IANA id), or null when not set.</summary>
    public virtual async Task<string?> GetUserTimeZoneAsync()
    {
        return await _settingManager.GetOrNullForCurrentUserAsync(TrackerSettings.User.TimeZone, fallback: false);
    }

    /// <param name="timeZoneId">An IANA id (see <see cref="TimeZoneIds.TryNormalize"/>), or null to unset it.</param>
    public virtual Task SetUserTimeZoneAsync(string? timeZoneId)
    {
        return _settingManager.SetForCurrentUserAsync(TrackerSettings.User.TimeZone, CheckTimeZone(timeZoneId));
    }

    /// <summary>The zone the current user's reports use: their own preference, else the workspace default. Null when
    /// neither is set; the client then uses the browser's zone.</summary>
    public virtual async Task<string?> GetEffectiveTimeZoneAsync()
    {
        return await GetUserTimeZoneAsync() ?? await GetDefaultTimeZoneAsync();
    }

    /// <summary>
    /// Gets the capture retention policy for the specified workspace (REQ-DAT-01).
    /// Safe default-off: unset or malformed policy fails closed (IsEnabled = false, RetentionDays = 30).
    /// </summary>
    public virtual async Task<(bool IsEnabled, int RetentionDays)> GetCaptureRetentionPolicyAsync(Guid workspaceId)
    {
        var isEnabledStr = await _settingManager.GetOrNullAsync(
            TrackerSettings.Workspace.CaptureRetention.IsEnabled,
            WorkspaceSettingValueProvider.ProviderName,
            workspaceId.ToString(),
            fallback: false);

        var daysStr = await _settingManager.GetOrNullAsync(
            TrackerSettings.Workspace.CaptureRetention.RetentionDays,
            WorkspaceSettingValueProvider.ProviderName,
            workspaceId.ToString(),
            fallback: false);

        // Safe default-off: unset policy fails closed with IsEnabled = false
        if (isEnabledStr == null)
        {
            return (false, 30);
        }

        // Malformed boolean fails closed
        if (!bool.TryParse(isEnabledStr, out var isEnabled))
        {
            return (false, 30);
        }

        // Strict integer parsing and bounded range validation [1, 730].
        // If days is missing/null, empty/whitespace, non-numeric, integer overflow, or out-of-bounds,
        // fail closed: do NOT silently substitute a destructive valid duration!
        if (!int.TryParse(daysStr, System.Globalization.NumberStyles.Integer, System.Globalization.CultureInfo.InvariantCulture, out var days) ||
            days < 1 || days > 730)
        {
            return (false, 30);
        }

        return (isEnabled, days);
    }

    /// <summary>
    /// Sets the capture retention policy for the specified workspace (REQ-DAT-01).
    /// Validates bounded positive retention days (1 to 730).
    /// </summary>
    public virtual async Task SetCaptureRetentionPolicyAsync(Guid workspaceId, bool isEnabled, int retentionDays)
    {
        Check.Range(retentionDays, nameof(retentionDays), 1, 730);

        await _settingManager.SetAsync(
            TrackerSettings.Workspace.CaptureRetention.IsEnabled,
            SettingValueFormat.Write(isEnabled),
            WorkspaceSettingValueProvider.ProviderName,
            workspaceId.ToString());

        await _settingManager.SetAsync(
            TrackerSettings.Workspace.CaptureRetention.RetentionDays,
            SettingValueFormat.Write(retentionDays),
            WorkspaceSettingValueProvider.ProviderName,
            workspaceId.ToString());
    }

    private Task SetAsync(string name, string? value)
    {
        return _settingManager.SetAsync(name, value, WorkspaceSettingValueProvider.ProviderName, CurrentWorkspace.GetId().ToString());
    }

    private Task<string?> GetOrNullAsync(string name)
    {
        return _settingManager.GetOrNullAsync(name, WorkspaceSettingValueProvider.ProviderName, CurrentWorkspace.GetId().ToString());
    }

    private async Task<int> GetIntAsync(string name, int fallback)
    {
        return SettingValueFormat.ReadInt(await GetOrNullAsync(name), fallback);
    }

    private async Task<bool> GetBoolAsync(string name, bool fallback)
    {
        return SettingValueFormat.ReadBool(await GetOrNullAsync(name), fallback);
    }

    private static string? CheckTimeZone(string? timeZoneId)
    {
        if (timeZoneId != null && (!TimeZoneIds.TryNormalize(timeZoneId, out var ianaId) || ianaId != timeZoneId))
        {
            throw new ArgumentException($"'{timeZoneId}' is not a normalized IANA time zone id.", nameof(timeZoneId));
        }

        return timeZoneId;
    }
}
