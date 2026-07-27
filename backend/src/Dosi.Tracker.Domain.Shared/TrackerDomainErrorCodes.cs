namespace Dosi.Tracker;

public static class TrackerDomainErrorCodes
{
    public const string ActivityTimeRangeInvalid = "Tracker:ActivityTimeRangeInvalid";
    public const string ActivityClientIdRequired = "Tracker:ActivityClientIdRequired";
    public const string DuplicateProjectMember = "Tracker:DuplicateProjectMember";
    public const string InvalidCaptureData = "Tracker:InvalidCaptureData";
    public const string CaptureTooLarge = "Tracker:CaptureTooLarge";
    public const string PlanNotFound = "Tracker:PlanNotFound";
    public const string SeatLimitReached = "Tracker:SeatLimitReached";
}
