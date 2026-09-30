using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;

namespace Dosi.Tracker.Activities;

public class CreateActivityDto
{
    /// <summary>Upper bound on the ActiveWindows / RunningPrograms lists of a single block.</summary>
    public const int MaxWindowEntries = 200;

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

    [Range(0, int.MaxValue)]
    public int MouseClicks { get; set; }

    [Range(0, int.MaxValue)]
    public int KeyboardHits { get; set; }

    [MaxLength(2048)]
    public string? Description { get; set; }

    // Accept structured data from clients
    [MaxLength(MaxWindowEntries)]
    public List<WindowInfoDto> ActiveWindows { get; set; } = new();

    [MaxLength(MaxWindowEntries)]
    public List<WindowInfoDto> RunningPrograms { get; set; } = new();

    // Accept base64 images from desktop agents
    public string? ScreenshotPngBase64 { get; set; }
    public string? WebcamJpgBase64 { get; set; }
}

public class WindowInfoDto
{
    public string? AppName { get; set; }
    public string? WindowTitle { get; set; }
}
