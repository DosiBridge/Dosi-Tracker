namespace Dosi.Tracker.Settings;

public static class TrackerSettings
{
    private const string Prefix = "Tracker";

    /// <summary>Workspace settings, managed by the workspace owner (tenant-level values).</summary>
    public static class Workspace
    {
        private const string WorkspacePrefix = Prefix + ".Workspace";

        /// <summary>IANA time zone id the workspace reports in by default (unset: the viewer's browser zone).</summary>
        public const string DefaultTimeZone = WorkspacePrefix + ".DefaultTimeZone";

        /// <summary>What a new project starts with (the project wizard is prefilled from these).</summary>
        public static class NewProject
        {
            private const string NewProjectPrefix = WorkspacePrefix + ".NewProject";

            public const string IntervalMinutes = NewProjectPrefix + ".IntervalMinutes";
            public const string AllowScreenshot = NewProjectPrefix + ".AllowScreenshot";
            public const string AllowWebcam = NewProjectPrefix + ".AllowWebcam";
            public const string AllowKeyboard = NewProjectPrefix + ".AllowKeyboard";
            public const string AllowMouse = NewProjectPrefix + ".AllowMouse";
            public const string AllowActiveWindow = NewProjectPrefix + ".AllowActiveWindow";
            public const string AllowRunningPrograms = NewProjectPrefix + ".AllowRunningPrograms";
            public const string BlurScreenshots = NewProjectPrefix + ".BlurScreenshots";
            public const string IdleThresholdMinutes = NewProjectPrefix + ".IdleThresholdMinutes";
        }

        /// <summary>Workspace capture retention policy (REQ-DAT-01).</summary>
        public static class CaptureRetention
        {
            private const string RetentionPrefix = WorkspacePrefix + ".CaptureRetention";

            /// <summary>Whether capture retention is explicitly opted-in for the workspace (defaults to false).</summary>
            public const string IsEnabled = RetentionPrefix + ".IsEnabled";

            /// <summary>Retention duration in days for the workspace (positive integer, 1 to 730).</summary>
            public const string RetentionDays = RetentionPrefix + ".RetentionDays";
        }

        /// <summary>Workspace activity (tracked time block) retention policy: opt-in, like capture retention.</summary>
        public static class ActivityRetention
        {
            private const string RetentionPrefix = WorkspacePrefix + ".ActivityRetention";

            /// <summary>Whether old time blocks are deleted for the workspace (defaults to false).</summary>
            public const string IsEnabled = RetentionPrefix + ".IsEnabled";

            /// <summary>Retention duration in days (see <c>ActivityRetentionLimits</c>).</summary>
            public const string RetentionDays = RetentionPrefix + ".RetentionDays";
        }
    }

    /// <summary>Personal preferences (user-level values).</summary>
    public static class User
    {
        /// <summary>IANA time zone id the user reports in; overrides <see cref="Workspace.DefaultTimeZone"/>.</summary>
        public const string TimeZone = Prefix + ".User.TimeZone";
    }
}
