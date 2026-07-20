using System;
using System.Collections.Generic;
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
    
    // Accept structured data from clients
    public List<WindowInfoDto> ActiveWindows { get; set; } = new();
    public List<WindowInfoDto> RunningPrograms { get; set; } = new();
    
    // Accept base64 images from desktop agents
    public string ScreenshotPngBase64 { get; set; }
    public string WebcamJpgBase64 { get; set; }
}

public class WindowInfoDto
{
    public string AppName { get; set; }
    public string WindowTitle { get; set; }
}
