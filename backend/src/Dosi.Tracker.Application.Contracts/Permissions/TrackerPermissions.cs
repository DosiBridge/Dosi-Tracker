namespace Dosi.Tracker.Permissions;

public static class TrackerPermissions
{
    public const string GroupName = "Tracker";

    /// <summary>Management permissions. Regular members only need to be authenticated
    /// for member-scoped endpoints (submit activity, my projects, my notifications, …);
    /// everything that changes shared tenant state requires one of these.</summary>
    public static class Projects
    {
        public const string Default = GroupName + ".Projects";
        public const string Create = Default + ".Create";
        public const string Edit = Default + ".Edit";
        public const string Delete = Default + ".Delete";
        public const string Archive = Default + ".Archive";
    }

    public static class Activities
    {
        public const string Default = GroupName + ".Activities";

        /// <summary>See every member's activities/screenshots (owners/admins).
        /// Without it, activity queries are scoped to the caller's own data.</summary>
        public const string ViewAll = Default + ".ViewAll";
    }

    public static class Team
    {
        public const string Default = GroupName + ".Team";
        public const string Manage = Default + ".Manage";
    }

    public static class Billing
    {
        public const string Default = GroupName + ".Billing";
    }

    public static class Reporting
    {
        public const string Default = GroupName + ".Reporting";
    }
}
