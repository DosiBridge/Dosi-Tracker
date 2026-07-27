using System;

namespace Dosi.Tracker.Projects;

/// <summary>
/// Trackable project as consumed by the desktop agents:
/// GET /api/app/project/my-projects returns a plain JSON array of these.
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
}
