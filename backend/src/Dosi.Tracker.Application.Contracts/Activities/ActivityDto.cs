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

    /// <summary>Activity % of the block; null when it was not measured.</summary>
    public int? Productivity { get; set; }

    /// <summary>Mouse clicks of the block; null when they were not measured (never a fabricated 0).</summary>
    public int? MouseClicks { get; set; }

    /// <summary>Keystrokes of the block; null when they were not measured.</summary>
    public int? KeyboardHits { get; set; }
    public string? Description { get; set; }
    public string ActiveWindowsJson { get; set; } = "[]";
    public string RunningProgramsJson { get; set; } = "[]";

    /// <summary>JSON array of the block's per-minute breakdown (see <see cref="ActivityMinuteDto"/>); "[]" when none.</summary>
    public string TimelineJson { get; set; } = "[]";

    // Device and agent context of the block (null for blocks from older agents).
    public string? Platform { get; set; }
    public string? OsVersion { get; set; }
    public string? AgentVersion { get; set; }
    public Guid? DeviceId { get; set; }
    public string? DeviceName { get; set; }
    public string? TimeZone { get; set; }
    public int? UtcOffsetMinutes { get; set; }

    /// <summary>Seconds of the block without keyboard/mouse input; null when not measured.</summary>
    public int? IdleSeconds { get; set; }

    /// <summary>Why the block ended ("interval", "idle", "lock", …); null when unknown.</summary>
    public string? EndReason { get; set; }

    /// <summary>Server time minus the device clock at upload, in seconds (positive = device clock behind).</summary>
    public int? ClockSkewSeconds { get; set; }

    /// <summary>Software-injected input events seen during the block (excluded from the counts).</summary>
    public int? InjectedInputEvents { get; set; }

    /// <summary>The captures stored for this block (screen, thumb, webcam), without their content.</summary>
    public List<ActivityCaptureRefDto> Captures { get; set; } = new();
}

/// <summary>A reference to one stored capture of a block; its bytes are served by the screenshot content endpoint.</summary>
public class ActivityCaptureRefDto
{
    public Guid Id { get; set; }

    /// <summary>"screen", "thumb" or "webcam".</summary>
    public string Kind { get; set; } = string.Empty;

    /// <summary>When the capture was taken.</summary>
    public DateTime CapturedAt { get; set; }

    /// <summary>True when the agent blurred the capture before uploading it.</summary>
    public bool Blurred { get; set; }
}
