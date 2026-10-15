using System;

namespace Dosi.Tracker.Identity;

/// <summary>Global Identity role names and project-membership role labels used by the tracker.</summary>
public static class TrackerRoles
{
    /// <summary>The global role of the platform (host console) administrators.</summary>
    public const string PlatformAdmin = "platform-admin";

    /// <summary>Project-member role of a stakeholder who follows a project's progress: never tracks time, takes no seat,
    /// and sees only the projects they are a client of.</summary>
    public const string ClientProjectRole = "Client";

    /// <summary>True when a project-member role label ("Admin", "manager", …) makes its holder a workspace manager.</summary>
    public static bool IsManagerProjectRole(string? projectRole)
    {
        var role = projectRole?.Trim();
        return string.Equals(role, "Admin", StringComparison.OrdinalIgnoreCase) ||
               string.Equals(role, "Manager", StringComparison.OrdinalIgnoreCase);
    }

    public static bool IsClientProjectRole(string? projectRole)
    {
        return string.Equals(projectRole?.Trim(), ClientProjectRole, StringComparison.OrdinalIgnoreCase);
    }
}
