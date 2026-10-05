using System;

namespace Dosi.Tracker.Projects;

/// <summary>
/// Trackable project as consumed by the desktop agents:
/// GET /api/app/project/my-projects returns a plain JSON array of these (archived projects and projects the caller
/// is only a Client of are left out).
/// </summary>
public class MyProjectDto
{
    public Guid Id { get; set; }
    public string Title { get; set; }
    public int IntervalMinutes { get; set; }
    public bool AllowScreenshot { get; set; }
    public bool AllowWebcam { get; set; }
    public bool AllowKeyboard { get; set; }
    public bool AllowMouse { get; set; }
    public bool AllowActiveWindow { get; set; }
    public bool AllowRunningPrograms { get; set; }

    /// <summary>Minutes without input after which the agent ends the block as idle; 0 = idle detection off.
    /// Agents default it to 5 when an older server omits it.</summary>
    public int IdleThresholdMinutes { get; set; }

    /// <summary>The agent must blur the screen capture and its thumbnail before uploading them
    /// (agents default it to false when an older server omits it).</summary>
    public bool BlurScreenshots { get; set; }
}
