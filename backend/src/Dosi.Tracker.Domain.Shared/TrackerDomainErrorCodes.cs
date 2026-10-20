namespace Dosi.Tracker;

public static class TrackerDomainErrorCodes
{
    public const string ActivityTimeRangeInvalid = "Tracker:ActivityTimeRangeInvalid";
    public const string ActivityClientIdRequired = "Tracker:ActivityClientIdRequired";
    public const string ActivityClientIdConflict = "Tracker:ActivityClientIdConflict";
    public const string ActivityInFuture = "Tracker:ActivityInFuture";
    public const string ActivityTooLong = "Tracker:ActivityTooLong";
    public const string ActivityTooOld = "Tracker:ActivityTooOld";
    public const string ActivityOverlaps = "Tracker:ActivityOverlaps";
    public const string ProjectArchived = "Tracker:ProjectArchived";
    public const string WorkspaceArchived = "Tracker:WorkspaceArchived";
    public const string NotProjectMember = "Tracker:NotProjectMember";
    public const string DuplicateProjectMember = "Tracker:DuplicateProjectMember";
    public const string InvalidCaptureData = "Tracker:InvalidCaptureData";
    public const string CaptureTooLarge = "Tracker:CaptureTooLarge";
    public const string SubscriptionInactive = "Tracker:SubscriptionInactive";
    public const string PlanNotFound = "Tracker:PlanNotFound";
    public const string PlanInUse = "Tracker:PlanInUse";
    public const string SeatLimitReached = "Tracker:SeatLimitReached";
    public const string ReportRangeInvalid = "Tracker:ReportRangeInvalid";

    /// <summary>The target user does not exist in the current workspace.</summary>
    public const string InvalidUser = "Tracker:InvalidUser";

    /// <summary>The caller tried to change their own membership or account (e.g. raise their own rate or role).</summary>
    public const string CannotModifySelf = "Tracker:CannotModifySelf";

    /// <summary>The caller may not grant the requested project role.</summary>
    public const string RoleNotAllowed = "Tracker:RoleNotAllowed";

    public const string WorkspaceRequired = "Tracker:WorkspaceRequired";
    public const string NotWorkspaceMember = "Tracker:NotWorkspaceMember";
    public const string EmailAlreadyRegistered = "Tracker:EmailAlreadyRegistered";
    public const string DuplicateWorkspaceName = "Tracker:DuplicateWorkspaceName";
    public const string LastOwnerCannotLeave = "Tracker:LastOwnerCannotLeave";
    public const string InvitationNotFound = "Tracker:InvitationNotFound";
    public const string InvitationExpired = "Tracker:InvitationExpired";
    public const string InvitationEmailMismatch = "Tracker:InvitationEmailMismatch";
    public const string AlreadyWorkspaceMember = "Tracker:AlreadyWorkspaceMember";
}
