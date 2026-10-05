using System;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Projects;

public class ProjectDto : FullAuditedEntityDto<Guid>
{
    public string Title { get; set; }
    public string? Description { get; set; }
    public string Color { get; set; }
    public int IntervalMinutes { get; set; }
    public bool IsArchived { get; set; }

    /// <summary>When the project was archived; null while it is active.</summary>
    public DateTime? ArchivedAt { get; set; }

    public bool AllowScreenshot { get; set; }
    public bool AllowWebcam { get; set; }
    public bool AllowKeyboard { get; set; }
    public bool AllowMouse { get; set; }
    public bool AllowActiveWindow { get; set; }
    public bool AllowRunningPrograms { get; set; }

    /// <summary>Minutes without input after which the agents end the block as idle; 0 = idle detection off.</summary>
    public int IdleThresholdMinutes { get; set; }

    /// <summary>The agents blur screen captures before uploading them.</summary>
    public bool BlurScreenshots { get; set; }
}
