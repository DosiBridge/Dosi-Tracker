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

    /// <summary>The action matches the caller by their account's email, which is not verified yet.</summary>
    public const string EmailNotConfirmed = "Tracker:EmailNotConfirmed";

    public const string ExportExceedsLimit = "Tracker:ExportExceedsLimit";
    /// <summary>The device (the token's, or the one the request names) is revoked in this workspace. Terminal for an
    /// agent: its sessions were revoked with it.</summary>
    public const string DeviceRevoked = "Tracker:DeviceRevoked";

    public const string DeviceNotObserved = "Tracker:DeviceNotObserved";

    /// <summary>No revocation of that device exists to restore.</summary>
    public const string DeviceNotFound = "Tracker:DeviceNotFound";

    /// <summary>An upload or heartbeat names no device: neither its token is device-bound nor its body has a deviceId.</summary>
    public const string DeviceIdRequired = "Tracker:DeviceIdRequired";

    /// <summary>The uploaded block was deleted from the workspace by an owner or manager (410). An agent drops the row:
    /// resending it can never succeed.</summary>
    public const string ActivityDeleted = "Tracker:ActivityDeleted";

    /// <summary>A member may delete their own captures only within the workspace's window after upload, and only while
    /// the owner allows it at all.</summary>
    public const string CaptureDeleteNotAllowed = "Tracker:CaptureDeleteNotAllowed";

    /// <summary>The typed confirmation of a workspace deletion is not the workspace's name.</summary>
    public const string WorkspaceDeleteConfirmationMismatch = "Tracker:WorkspaceDeleteConfirmationMismatch";

    /// <summary>The account is the only owner of a workspace: transfer ownership or delete the workspace first.</summary>
    public const string SoleWorkspaceOwner = "Tracker:SoleWorkspaceOwner";

    /// <summary>A platform administrator's account cannot be deleted by self-service.</summary>
    public const string AccountDeletionNotAllowed = "Tracker:AccountDeletionNotAllowed";

    /// <summary>The password that confirms a sensitive change is wrong.</summary>
    public const string InvalidPassword = "Tracker:InvalidPassword";

    /// <summary>The email confirmation link is invalid or expired.</summary>
    public const string EmailConfirmationInvalid = "Tracker:EmailConfirmationInvalid";

    /// <summary>Billing is turned off on this deployment (<c>Billing:Enabled</c>): there is no plan to change or cancel.</summary>
    public const string BillingDisabled = "Tracker:BillingDisabled";

    /// <summary>Owners may only move to a free plan by themselves; paid plans are changed by the platform.</summary>
    public const string PlanChangeNotAllowed = "Tracker:PlanChangeNotAllowed";
}
