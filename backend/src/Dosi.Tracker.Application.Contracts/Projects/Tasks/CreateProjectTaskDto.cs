using System;
using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.Projects;

public class CreateProjectTaskDto
{
    [Required]
    public Guid ProjectId { get; set; }

    [Required]
    [StringLength(ProjectTaskConsts.MaxTitleLength, MinimumLength = 1)]
    public string Title { get; set; } = string.Empty;

    [StringLength(ProjectTaskConsts.MaxDescriptionLength)]
    public string? Description { get; set; }

    public bool IsCompleted { get; set; }
}
