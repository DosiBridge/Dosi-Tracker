namespace Dosi.Tracker.Activities;

/// <summary>
/// Bounds of a workspace's activity retention policy (how long tracked time blocks are kept). The minimum lies well past
/// the agents' upload window (45 days by default): a block still inside it is never deleted, since an agent replaying it
/// would otherwise upload it again, and closed pay periods keep their time for at least a quarter.
/// </summary>
public static class ActivityRetentionLimits
{
    public const int MinDays = 90;

    /// <summary>About ten years.</summary>
    public const int MaxDays = 3650;

    /// <summary>The window shown for a workspace that never set a policy (which is off).</summary>
    public const int DefaultDays = 365;
}
