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

        /// <summary>Minutes after upload during which members may delete their own captures (0: never; see
        /// <c>MemberCaptureDeletion</c>).</summary>
        public const string MemberCaptureDeleteMinutes = WorkspacePrefix + ".MemberCaptureDeleteMinutes";

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

        /// <summary>AI assist, opted into by the workspace owner. Only the workspace's own value counts (never a
        /// platform-wide one), and an unset or malformed value reads as its code default (see <c>AiSettingsManager</c>).</summary>
        public static class Ai
        {
            private const string AiPrefix = WorkspacePrefix + ".Ai";

            /// <summary>The master opt-in (default false).</summary>
            public const string IsEnabled = AiPrefix + ".IsEnabled";

            /// <summary>Managers may use AI assist (default true).</summary>
            public const string AllowManagers = AiPrefix + ".AllowManagers";

            /// <summary>Members may use AI assist on their own data (default true).</summary>
            public const string AllowMemberSelfService = AiPrefix + ".AllowMemberSelfService";

            /// <summary>Window titles may be sent to the provider (default false).</summary>
            public const string AllowWindowTitles = AiPrefix + ".AllowWindowTitles";

            /// <summary>Task notes (text members wrote) may be sent to the provider (default false).</summary>
            public const string AllowNotes = AiPrefix + ".AllowNotes";

            /// <summary>The disclosure version the owner acknowledged ("{code}.{operator}").</summary>
            public const string ConsentVersion = AiPrefix + ".ConsentVersion";

            /// <summary>When AI assist was last enabled or re-consented (ISO-8601 UTC).</summary>
            public const string EnabledAt = AiPrefix + ".EnabledAt";

            /// <summary>Who last enabled or re-consented (a user id).</summary>
            public const string EnabledBy = AiPrefix + ".EnabledBy";
        }
    }

    /// <summary>Platform settings, managed by the platform (host) administrators (global values only).</summary>
    public static class Platform
    {
        /// <summary>The runtime switch of AI assist for the whole platform (default true; inert unless <c>AI:Enabled</c>).
        /// A malformed value reads as off.</summary>
        public const string AiEnabled = Prefix + ".Platform.Ai.Enabled";
    }

    /// <summary>Personal preferences (user-level values).</summary>
    public static class User
    {
        /// <summary>IANA time zone id the user reports in; overrides <see cref="Workspace.DefaultTimeZone"/>.</summary>
        public const string TimeZone = Prefix + ".User.TimeZone";
    }
}
