using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.Projects;

public class CreateUpdateProjectDto
{
    [Required]
    [MaxLength(128)]
    public string Title { get; set; }

    public string? Description { get; set; }

    [Required]
    public string Color { get; set; } = "#006bff";

    // Both desktop agents clamp the snapshot interval to 5..60 minutes; enforce the same rule server-side.
    [Required]
    [Range(5, 60)]
    public int IntervalMinutes { get; set; } = 10;

    // Per-project capture permissions pushed to the desktop agents.
    public bool AllowScreenshot { get; set; } = true;
    public bool AllowWebcam { get; set; } = false;
    public bool AllowKeyboard { get; set; } = true;
    public bool AllowMouse { get; set; } = true;
    public bool AllowActiveWindow { get; set; } = true;
    public bool AllowRunningPrograms { get; set; } = true;
}
