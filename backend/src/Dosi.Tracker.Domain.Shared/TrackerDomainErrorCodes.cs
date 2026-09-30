namespace Dosi.Tracker;

public static class TrackerDomainErrorCodes
{
    public const string ActivityTimeRangeInvalid = "Tracker:ActivityTimeRangeInvalid";
    public const string ActivityClientIdRequired = "Tracker:ActivityClientIdRequired";
    public const string ActivityClientIdConflict = "Tracker:ActivityClientIdConflict";
    public const string ActivityInFuture = "Tracker:ActivityInFuture";
    public const string ActivityTooLong = "Tracker:ActivityTooLong";
    public const string ActivityOverlaps = "Tracker:ActivityOverlaps";
    public const string ProjectArchived = "Tracker:ProjectArchived";
    public const string DuplicateProjectMember = "Tracker:DuplicateProjectMember";
    public const string InvalidCaptureData = "Tracker:InvalidCaptureData";
    public const string CaptureTooLarge = "Tracker:CaptureTooLarge";
    public const string PlanNotFound = "Tracker:PlanNotFound";
    public const string PlanInUse = "Tracker:PlanInUse";
    public const string SeatLimitReached = "Tracker:SeatLimitReached";
    public const string ReportRangeInvalid = "Tracker:ReportRangeInvalid";
}
