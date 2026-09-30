using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Text.Json.Serialization;

namespace Dosi.Tracker.Activities;

public class CreateActivityDto
{
    /// <summary>Upper bound on the ActiveWindows / RunningPrograms lists of a single block.</summary>
    public const int MaxWindowEntries = 200;

    /// <summary>Upper bound on the per-minute <see cref="Timeline"/> of a single block (blocks are at most 2 x 60 min).</summary>
    public const int MaxTimelineEntries = 120;

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
    /// <summary>Per-(app, window title) usage during the whole block, most-used first. Older agents send a
    /// single entry (the window focused at capture time) without <see cref="WindowInfoDto.Seconds"/>.</summary>
    [MaxLength(MaxWindowEntries)]
    public List<WindowInfoDto> ActiveWindows { get; set; } = new();

    [MaxLength(MaxWindowEntries)]
    public List<WindowInfoDto> RunningPrograms { get; set; } = new();

    /// <summary>Optional per-minute breakdown of the block (older agents omit it).</summary>
    [MaxLength(MaxTimelineEntries)]
    public List<ActivityMinuteDto> Timeline { get; set; } = new();

    // Accept base64 images from desktop agents
    public string? ScreenshotPngBase64 { get; set; }

    /// <summary>Optional small JPEG rendition (at most 480px wide) of the same screenshot, used for list thumbnails.</summary>
    public string? ScreenshotThumbJpgBase64 { get; set; }

    public string? WebcamJpgBase64 { get; set; }
}

/// <summary>
/// A window/application seen during a block. Also the shape persisted in <c>Activity.ActiveWindowsJson</c> /
/// <c>RunningProgramsJson</c>; the optional usage fields are omitted from the stored JSON when absent so
/// legacy entries keep their original <c>{"AppName":...,"WindowTitle":...}</c> shape.
/// </summary>
public class WindowInfoDto
{
    public string? AppName { get; set; }
    public string? WindowTitle { get; set; }

    /// <summary>Foreground seconds attributed to this window during the block (null from older agents).</summary>
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public int? Seconds { get; set; }

    /// <summary>Keystrokes attributed to this window during the block.</summary>
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public int? KeyboardHits { get; set; }

    /// <summary>Mouse clicks attributed to this window during the block.</summary>
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public int? MouseClicks { get; set; }
}

/// <summary>
/// One minute of a block's timeline. Also the shape persisted in <c>Activity.TimelineJson</c>
/// (<see cref="AppName"/> is omitted from the stored JSON when absent).
/// </summary>
public class ActivityMinuteDto
{
    /// <summary>0-based minute bucket index, counted from the block's StartedAt.</summary>
    public int Minute { get; set; }

    public int KeyboardHits { get; set; }

    public int MouseClicks { get; set; }

    /// <summary>Whether the member was active (not idle) during this minute.</summary>
    public bool Active { get; set; }

    /// <summary>The application mostly in the foreground during this minute, if known.</summary>
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? AppName { get; set; }
}
