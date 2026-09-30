using System;
using System.Collections.Generic;
using Volo.Abp.Application.Dtos;

namespace Dosi.Tracker.Activities;

public class ActivityDto : CreationAuditedEntityDto<Guid>
{
    public Guid UserId { get; set; }
    public Guid ProjectId { get; set; }
    public Guid ClientActivityId { get; set; }
    public DateTime StartedAt { get; set; }
    public DateTime EndedAt { get; set; }
    public int Productivity { get; set; }
    public int MouseClicks { get; set; }
    public int KeyboardHits { get; set; }
    public string? Description { get; set; }
    public string ActiveWindowsJson { get; set; } = "[]";
    public string RunningProgramsJson { get; set; } = "[]";

    /// <summary>JSON array of the block's per-minute breakdown (see <see cref="ActivityMinuteDto"/>); "[]" when none.</summary>
    public string TimelineJson { get; set; } = "[]";

    /// <summary>The captures stored for this block (screen, thumb, webcam), without their content.</summary>
    public List<ActivityCaptureRefDto> Captures { get; set; } = new();
}

/// <summary>A reference to one stored capture of a block; its bytes are served by the screenshot content endpoint.</summary>
public class ActivityCaptureRefDto
{
    public Guid Id { get; set; }

    /// <summary>"screen", "thumb" or "webcam".</summary>
    public string Kind { get; set; } = string.Empty;
}
