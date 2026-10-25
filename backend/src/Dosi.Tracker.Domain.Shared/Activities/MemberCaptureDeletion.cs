namespace Dosi.Tracker.Activities;

/// <summary>
/// How long after a capture was uploaded the member who took it may still delete it themselves (for instance a
/// screenshot that caught a banking page). The workspace owner sets the window: 0 turns self-deletion off, and owners and
/// managers can always delete captures (with a reason).
/// </summary>
public static class MemberCaptureDeletion
{
    public const int DefaultWindowMinutes = 60;

    /// <summary>A week.</summary>
    public const int MaxWindowMinutes = 7 * 24 * 60;
}
