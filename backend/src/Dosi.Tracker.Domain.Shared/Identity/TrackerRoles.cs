using System;
using Dosi.Tracker.Permissions;

namespace Dosi.Tracker.Identity;

/// <summary>Identity role names used by the tracker, and the permissions of the built-in "manager" role.</summary>
public static class TrackerRoles
{
    /// <summary>ABP's built-in administrator role (the workspace owner); granted every permission on seed.</summary>
    public const string Admin = "admin";

    /// <summary>Team lead: sees everyone's activity, manages the team and projects, runs reports —
    /// but cannot delete projects, manage billing, or administer identity/settings.</summary>
    public const string Manager = "manager";

    public static readonly string[] ManagerPermissions =
    {
        TrackerPermissions.Activities.Default,
        TrackerPermissions.Activities.ViewAll,
        TrackerPermissions.Team.Default,
        TrackerPermissions.Team.Manage,
        TrackerPermissions.Projects.Default,
        TrackerPermissions.Projects.Create,
        TrackerPermissions.Projects.Edit,
        TrackerPermissions.Projects.Archive,
        TrackerPermissions.Reporting.Default
    };

    /// <summary>Project-member role of a stakeholder who follows a project's progress: never tracks time (the project
    /// is not offered to the agents and uploads are refused), takes no seat, and sees only the projects they are a
    /// client of.</summary>
    public const string ClientProjectRole = "Client";

    /// <summary>True when a project-member role label ("Admin", "manager", …) should confer the
    /// Identity "manager" role. Worker/Employee/Member (anything else) confers nothing extra.</summary>
    public static bool IsManagerProjectRole(string? projectRole)
    {
        var role = projectRole?.Trim();
        return string.Equals(role, Admin, StringComparison.OrdinalIgnoreCase) ||
               string.Equals(role, Manager, StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>True for the <see cref="ClientProjectRole"/> project role, whatever its case or surrounding spaces.</summary>
    public static bool IsClientProjectRole(string? projectRole)
    {
        return string.Equals(projectRole?.Trim(), ClientProjectRole, StringComparison.OrdinalIgnoreCase);
    }
}
