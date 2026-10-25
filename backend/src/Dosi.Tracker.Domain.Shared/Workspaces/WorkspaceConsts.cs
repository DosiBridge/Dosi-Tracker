using System;
using System.Linq;
using Dosi.Tracker.Permissions;

namespace Dosi.Tracker.Workspaces;

public static class WorkspaceConsts
{
    public const int MaxNameLength = 64;

    /// <summary>Request header naming the workspace a call is made in.</summary>
    public const string HeaderName = "X-Workspace-Id";

    public const int InvitationLifetimeDays = 14;
}

/// <summary>A member's role in a workspace (<c>WorkspaceMember.Role</c>).</summary>
public static class WorkspaceRoles
{
    public const string Owner = "owner";
    public const string Manager = "manager";
    public const string Member = "member";

    public static readonly string[] All = { Owner, Manager, Member };

    public static bool IsValid(string? role) => role != null && All.Contains(role);

    /// <summary>The canonical role for a label (case and spaces ignored), or null when unknown.</summary>
    public static string? Normalize(string? role)
    {
        var trimmed = role?.Trim();
        return All.FirstOrDefault(r => string.Equals(r, trimmed, StringComparison.OrdinalIgnoreCase));
    }

    /// <summary>The permissions a role grants inside its workspace.</summary>
    public static string[] PermissionsOf(string? role) => role switch
    {
        Owner => OwnerPermissions,
        Manager => ManagerPermissions,
        _ => Array.Empty<string>()
    };

    public static readonly string[] OwnerPermissions =
    {
        TrackerPermissions.Projects.Default,
        TrackerPermissions.Projects.Create,
        TrackerPermissions.Projects.Edit,
        TrackerPermissions.Projects.Delete,
        TrackerPermissions.Projects.Archive,
        TrackerPermissions.Activities.Default,
        TrackerPermissions.Activities.ViewAll,
        TrackerPermissions.Team.Default,
        TrackerPermissions.Team.Manage,
        TrackerPermissions.Devices.Default,
        TrackerPermissions.Devices.Manage,
        TrackerPermissions.Billing.Default,
        TrackerPermissions.Reporting.Default
    };

    /// <summary>Team lead: sees everyone's activity, manages the team, its devices and projects, runs reports — but cannot delete
    /// projects, manage billing or change the workspace settings.</summary>
    public static readonly string[] ManagerPermissions =
    {
        TrackerPermissions.Activities.Default,
        TrackerPermissions.Activities.ViewAll,
        TrackerPermissions.Team.Default,
        TrackerPermissions.Team.Manage,
        TrackerPermissions.Devices.Default,
        TrackerPermissions.Devices.Manage,
        TrackerPermissions.Projects.Default,
        TrackerPermissions.Projects.Create,
        TrackerPermissions.Projects.Edit,
        TrackerPermissions.Projects.Archive,
        TrackerPermissions.Reporting.Default
    };
}

public static class WorkspaceMemberStatus
{
    public const string Active = "active";
    public const string Removed = "removed";
}

/// <summary>Marks an application service (class) or method that works without an <c>X-Workspace-Id</c>.</summary>
[AttributeUsage(AttributeTargets.Class | AttributeTargets.Method)]
public sealed class WorkspaceNotRequiredAttribute : Attribute
{
}
