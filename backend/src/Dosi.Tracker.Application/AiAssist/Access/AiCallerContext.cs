using System;
using Dosi.Tracker.Workspaces;

namespace Dosi.Tracker.AiAssist.Access;

/// <summary>
/// Who is asking, in which workspace, and what they may see there — built once per AI request
/// (<see cref="AiCallerContextFactory"/>) and read by the access guard and the features.
/// </summary>
/// <param name="Role">The caller's workspace role (<see cref="WorkspaceRoles"/>); null when they are not an active
/// member.</param>
/// <param name="IsClientOnly">A member whose project memberships (at least one) are all <c>Client</c>: they follow
/// projects and never track, so no AI feature is for them.</param>
/// <param name="HasWorkerProjectMembership">The caller belongs to at least one project in a role other than Client.</param>
/// <param name="TimeZone">The IANA zone days are counted in: the request's, else the caller's effective reporting zone,
/// else UTC.</param>
/// <param name="DisclosureVersion">The disclosure version the server shows now ("{code}.{operator}").</param>
/// <param name="PlanName">The workspace's plan, or "Default" without a subscription (what its allowance is keyed by).</param>
public sealed record AiCallerContext(
    Guid WorkspaceId,
    Guid UserId,
    string? Role,
    bool IsClientOnly,
    bool HasWorkerProjectMembership,
    bool CanViewAll,
    bool CanReport,
    bool CanManageTeam,
    bool CanEditProjects,
    string TimeZone,
    AiWorkspacePolicy Policy,
    string DisclosureVersion,
    string PlanName)
{
    /// <summary>The plan name used when the workspace has no subscription (or its plan no longer exists).</summary>
    public const string DefaultPlanName = "Default";

    public bool IsOwner => Role == WorkspaceRoles.Owner;
    public bool IsManager => Role == WorkspaceRoles.Manager;
    public bool IsMember => Role == WorkspaceRoles.Member;

    /// <summary>May ask about the team or another member: an owner or manager holding both the ViewAll and the Reporting
    /// permission. Members only ever ask about themselves.</summary>
    public bool CanSeeTeam => (IsOwner || IsManager) && CanViewAll && CanReport;

    /// <summary>The owner turned AI assist on and consented to the disclosure shown now.</summary>
    public bool IsWorkspaceActive => Policy.IsActive(DisclosureVersion);
}
