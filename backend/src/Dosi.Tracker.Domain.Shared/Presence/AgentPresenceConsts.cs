namespace Dosi.Tracker.Presence;

/// <summary>Limits and allowed statuses of a desktop agent's live presence (shared contract §3).</summary>
public static class AgentPresenceConsts
{
    public const int MaxStatusLength = 16;

    /// <summary>The foreground application reported with a heartbeat (only when the project allows it).</summary>
    public const int MaxCurrentAppLength = 256;

    /// <summary>A device counts as online while its last heartbeat is at most this many seconds old.</summary>
    public const int OnlineWindowSeconds = 150;

    /// <summary>Most devices kept per member: when a new one reports, the least recently seen beyond this are dropped
    /// (every reinstall, or a script inventing device ids, would otherwise add a row for good).</summary>
    public const int MaxDevicesPerUser = 10;

    /// <summary>A member's device not seen for this many days is dropped when a new device of theirs reports.</summary>
    public const int StaleDeviceDays = 30;

    /// <summary>Every status an agent may report; anything else is a validation error.</summary>
    public static readonly string[] Statuses =
    {
        "tracking", "paused", "break", "idle", "locked", "choosing", "signedout", "stopped"
    };
}
