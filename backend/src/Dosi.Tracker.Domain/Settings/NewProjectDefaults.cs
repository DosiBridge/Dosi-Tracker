namespace Dosi.Tracker.Settings;

/// <summary>What a new project of the workspace starts with. The initial values are the platform defaults: the same
/// a project gets without them (privacy-first: everything but the webcam, no blur, 10-minute interval, 5-minute idle).</summary>
public class NewProjectDefaults
{
    public const int MinIntervalMinutes = 5;
    public const int MaxIntervalMinutes = 60;
    public const int MaxIdleThresholdMinutes = 60;

    public int IntervalMinutes { get; init; } = 10;
    public bool AllowScreenshot { get; init; } = true;
    public bool AllowWebcam { get; init; } = false;
    public bool AllowKeyboard { get; init; } = true;
    public bool AllowMouse { get; init; } = true;
    public bool AllowActiveWindow { get; init; } = true;
    public bool AllowRunningPrograms { get; init; } = true;
    public bool BlurScreenshots { get; init; } = false;

    /// <summary>0 turns idle detection off.</summary>
    public int IdleThresholdMinutes { get; init; } = 5;
}
