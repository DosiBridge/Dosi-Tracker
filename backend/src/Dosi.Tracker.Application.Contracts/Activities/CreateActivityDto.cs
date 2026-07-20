using System;
using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.Activities;

public class CreateActivityDto
{
    [Required]
    public Guid ProjectId { get; set; }

    [Required]
    public Guid ClientActivityId { get; set; }

    [Required]
    public DateTime StartedAt { get; set; }

    [Required]
    public DateTime EndedAt { get; set; }

    [Range(0, 100)]
    public int Productivity { get; set; }

    public int MouseClicks { get; set; }
    public int KeyboardHits { get; set; }
    
    public string Description { get; set; }
    public string ActiveWindowsJson { get; set; }
    public string RunningProgramsJson { get; set; }
}
