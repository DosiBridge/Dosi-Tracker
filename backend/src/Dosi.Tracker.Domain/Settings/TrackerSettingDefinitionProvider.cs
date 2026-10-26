using Volo.Abp.Settings;

namespace Dosi.Tracker.Settings;

public class TrackerSettingDefinitionProvider : SettingDefinitionProvider
{
    public override void Define(ISettingDefinitionContext context)
    {
        var defaults = new NewProjectDefaults();
        context.Add(
            WorkspaceSetting(TrackerSettings.Workspace.DefaultTimeZone, defaultValue: null),
            WorkspaceSetting(TrackerSettings.Workspace.NewProject.IntervalMinutes, SettingValueFormat.Write(defaults.IntervalMinutes)),
            WorkspaceSetting(TrackerSettings.Workspace.NewProject.AllowScreenshot, SettingValueFormat.Write(defaults.AllowScreenshot)),
            WorkspaceSetting(TrackerSettings.Workspace.NewProject.AllowWebcam, SettingValueFormat.Write(defaults.AllowWebcam)),
            WorkspaceSetting(TrackerSettings.Workspace.NewProject.AllowKeyboard, SettingValueFormat.Write(defaults.AllowKeyboard)),
            WorkspaceSetting(TrackerSettings.Workspace.NewProject.AllowMouse, SettingValueFormat.Write(defaults.AllowMouse)),
            WorkspaceSetting(TrackerSettings.Workspace.NewProject.AllowActiveWindow, SettingValueFormat.Write(defaults.AllowActiveWindow)),
            WorkspaceSetting(TrackerSettings.Workspace.NewProject.AllowRunningPrograms, SettingValueFormat.Write(defaults.AllowRunningPrograms)),
            WorkspaceSetting(TrackerSettings.Workspace.NewProject.BlurScreenshots, SettingValueFormat.Write(defaults.BlurScreenshots)),
            WorkspaceSetting(TrackerSettings.Workspace.NewProject.IdleThresholdMinutes, SettingValueFormat.Write(defaults.IdleThresholdMinutes)),
            WorkspaceSetting(TrackerSettings.Workspace.MemberCaptureDeleteMinutes,
                SettingValueFormat.Write(Activities.MemberCaptureDeletion.DefaultWindowMinutes)),
            WorkspaceSetting(TrackerSettings.Workspace.CaptureRetention.IsEnabled, defaultValue: null),
            WorkspaceSetting(TrackerSettings.Workspace.CaptureRetention.RetentionDays, defaultValue: null),
            WorkspaceSetting(TrackerSettings.Workspace.ActivityRetention.IsEnabled, defaultValue: null),
            WorkspaceSetting(TrackerSettings.Workspace.ActivityRetention.RetentionDays, defaultValue: null),
            WorkspaceOnlySetting(TrackerSettings.Workspace.Ai.IsEnabled, SettingValueFormat.Write(false)),
            WorkspaceOnlySetting(TrackerSettings.Workspace.Ai.AllowManagers, SettingValueFormat.Write(true)),
            WorkspaceOnlySetting(TrackerSettings.Workspace.Ai.AllowMemberSelfService, SettingValueFormat.Write(true)),
            WorkspaceOnlySetting(TrackerSettings.Workspace.Ai.AllowWindowTitles, SettingValueFormat.Write(false)),
            WorkspaceOnlySetting(TrackerSettings.Workspace.Ai.AllowNotes, SettingValueFormat.Write(false)),
            WorkspaceOnlySetting(TrackerSettings.Workspace.Ai.ConsentVersion, defaultValue: null),
            WorkspaceOnlySetting(TrackerSettings.Workspace.Ai.EnabledAt, defaultValue: null),
            WorkspaceOnlySetting(TrackerSettings.Workspace.Ai.EnabledBy, defaultValue: null),
            GlobalOnlySetting(TrackerSettings.Platform.AiEnabled, SettingValueFormat.Write(true)),
            new SettingDefinition(TrackerSettings.User.TimeZone)
                .WithProviders(DefaultValueSettingValueProvider.ProviderName, UserSettingValueProvider.ProviderName));
    }

    /// <summary>A workspace-wide setting: the workspace's own value, else a platform-wide (global or configured)
    /// value, else the default. Never a per-user value.</summary>
    private static SettingDefinition WorkspaceSetting(string name, string? defaultValue)
    {
        return new SettingDefinition(name, defaultValue).WithProviders(
            DefaultValueSettingValueProvider.ProviderName,
            ConfigurationSettingValueProvider.ProviderName,
            GlobalSettingValueProvider.ProviderName,
            WorkspaceSettingValueProvider.ProviderName);
    }

    /// <summary>A setting only the workspace itself can set: a global or configured value never applies to it (AI assist
    /// must never be switched on for a workspace by anyone but its owner).</summary>
    private static SettingDefinition WorkspaceOnlySetting(string name, string? defaultValue)
    {
        return new SettingDefinition(name, defaultValue).WithProviders(
            DefaultValueSettingValueProvider.ProviderName,
            WorkspaceSettingValueProvider.ProviderName);
    }

    /// <summary>A platform-wide setting the platform administrators set at run time (never per workspace or user).</summary>
    private static SettingDefinition GlobalOnlySetting(string name, string? defaultValue)
    {
        return new SettingDefinition(name, defaultValue).WithProviders(
            DefaultValueSettingValueProvider.ProviderName,
            GlobalSettingValueProvider.ProviderName);
    }
}
