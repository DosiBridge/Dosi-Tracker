using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.Projects;

public class CreateUpdateProjectDto
{
    [Required]
    [MaxLength(ProjectConsts.MaxTitleLength)]
    public string Title { get; set; }

    [MaxLength(ProjectConsts.MaxDescriptionLength)]
    public string? Description { get; set; }

    // A hex color (#RRGGBB) only: the web app renders the value as a style on project pages.
    [Required]
    [RegularExpression(ProjectConsts.ColorPattern)]
    public string Color { get; set; } = ProjectConsts.DefaultColor;

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

    /// <summary>Minutes without input after which the agents end the block as idle; 0 turns idle detection off.</summary>
    [Range(0, 60)]
    public int IdleThresholdMinutes { get; set; } = 5;

    /// <summary>The agents must blur screen captures (and thumbnails) before uploading them.</summary>
    public bool BlurScreenshots { get; set; } = false;
}
