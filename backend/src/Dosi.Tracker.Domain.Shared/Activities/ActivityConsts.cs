namespace Dosi.Tracker.Activities;

/// <summary>
/// Limits and allowed values of the block context the desktop agents upload with each activity (shared
/// contract §1). These fields are optional: the server normalizes them (trims/truncates strings, stores an
/// unknown or out-of-range value as null) and never rejects a block because of them.
/// </summary>
public static class ActivityConsts
{
    public const int MaxPlatformLength = 16;
    public const int MaxOsVersionLength = 64;
    public const int MaxAgentVersionLength = 32;
    public const int MaxDeviceNameLength = 128;

    /// <summary>IANA time zone id of the device, e.g. "Asia/Dhaka".</summary>
    public const int MaxTimeZoneLength = 64;

    public const int MaxEndReasonLength = 16;

    /// <summary>Bounds of a device's UTC offset in minutes (UTC-14:00 .. UTC+14:00).</summary>
    public const int MinUtcOffsetMinutes = -840;
    public const int MaxUtcOffsetMinutes = 840;

    /// <summary>Upper bound of <c>ActivityMinuteDto.ActiveSeconds</c>: the last minute bucket of a block can
    /// absorb a remainder of up to 30 s.</summary>
    public const int MaxMinuteActiveSeconds = 90;

    /// <summary>Known agent platforms (lower case); anything else is stored as null.</summary>
    public static readonly string[] Platforms = { "windows", "macos" };

    /// <summary>Why a block ended; an unknown value is stored as null.</summary>
    public static readonly string[] EndReasons =
    {
        "interval", "pause", "break", "idle", "lock", "sleep", "project", "task", "signout", "quit", "shutdown"
    };
}
