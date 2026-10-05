using System;
using Dosi.Tracker.Identity;

namespace Dosi.Tracker.Teams;

/// <summary>
/// The project-membership role labels (<c>ProjectMember.Role</c>) the web app and the agents use. Older data may hold
/// other free-form labels ("Employee", "member", …), which behave like <see cref="Worker"/>.
/// </summary>
public static class ProjectMemberRoles
{
    /// <summary>Manages the project; confers the Identity "manager" role. Only workspace owners may grant it.</summary>
    public const string Admin = "Admin";

    /// <summary>Team lead on the project; confers the Identity "manager" role. Only workspace owners may grant it.</summary>
    public const string Manager = "Manager";

    /// <summary>Tracks time on the project.</summary>
    public const string Worker = "Worker";

    /// <summary>A stakeholder who follows the project's progress: never tracks time (the project is not offered to the
    /// agents), needs no seat, and sees only the projects they are a client of (<see cref="TrackerRoles.ClientProjectRole"/>).</summary>
    public const string Client = TrackerRoles.ClientProjectRole;

    private static readonly string[] Known = { Admin, Manager, Worker, Client };

    /// <summary>True for the <see cref="Client"/> role, whatever its case or surrounding spaces.</summary>
    public static bool IsClient(string? role)
    {
        return TrackerRoles.IsClientProjectRole(role);
    }

    /// <summary>True for Admin/Manager (any case), the roles that confer the Identity "manager" role.</summary>
    public static bool IsManagerLevel(string? role)
    {
        return TrackerRoles.IsManagerProjectRole(role);
    }

    /// <summary>The label to store: trimmed, with the known roles in their canonical spelling (" manager" → "Manager");
    /// any other label is kept as typed.</summary>
    public static string Normalize(string role)
    {
        var trimmed = role.Trim();
        foreach (var known in Known)
        {
            if (string.Equals(trimmed, known, StringComparison.OrdinalIgnoreCase))
            {
                return known;
            }
        }

        return trimmed;
    }

    /// <summary>Whether two labels name the same role (case and surrounding spaces ignored).</summary>
    public static bool AreSame(string? left, string? right)
    {
        return string.Equals(left?.Trim(), right?.Trim(), StringComparison.OrdinalIgnoreCase);
    }
}
