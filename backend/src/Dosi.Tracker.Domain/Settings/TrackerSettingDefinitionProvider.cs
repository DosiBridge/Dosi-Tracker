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
            WorkspaceSetting(TrackerSettings.Workspace.CaptureRetention.IsEnabled, defaultValue: null),
            WorkspaceSetting(TrackerSettings.Workspace.CaptureRetention.RetentionDays, defaultValue: null),
            WorkspaceSetting(TrackerSettings.Workspace.ActivityRetention.IsEnabled, defaultValue: null),
            WorkspaceSetting(TrackerSettings.Workspace.ActivityRetention.RetentionDays, defaultValue: null),
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
}
