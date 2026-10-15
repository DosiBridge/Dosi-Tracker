using System;
using System.Collections.Generic;
using Volo.Abp;
using Volo.Abp.Domain.Entities.Auditing;
using Dosi.Tracker.Workspaces;

namespace Dosi.Tracker.Activities;

public class Activity : CreationAuditedAggregateRoot<Guid>, IWorkspaceScoped
{
    public Guid WorkspaceId { get; set; }
    public Guid UserId { get; set; }
    public Guid ProjectId { get; set; }
    public Guid ClientActivityId { get; set; } // Offline Idempotency key from Agent

    public DateTime StartedAt { get; set; } // Must be UTC
    public DateTime EndedAt { get; set; }   // Must be UTC

    public int? Productivity { get; set; } // 0 - 100; null = not measured (input capture unavailable or not allowed)
    public int MouseClicks { get; set; }
    public int KeyboardHits { get; set; }

    public string? Description { get; set; } // Optional: agents typically send no description
    public string ActiveWindowsJson { get; set; } = "[]"; // JSON array of active windows
    public string RunningProgramsJson { get; set; } = "[]"; // JSON array of running apps
    public string TimelineJson { get; set; } = "[]"; // JSON array of per-minute input/activity buckets

    // Device and agent context reported with the block (all optional: null from older agents).
    public string? Platform { get; set; } // "windows" | "macos"
    public string? OsVersion { get; set; }
    public string? AgentVersion { get; set; }
    public Guid? DeviceId { get; set; } // Random id generated once per agent install
    public string? DeviceName { get; set; }
    public string? TimeZone { get; set; } // IANA id of the device, e.g. "Asia/Dhaka"
    public int? UtcOffsetMinutes { get; set; } // Device's UTC offset at StartedAt

    /// <summary>Seconds of the block without keyboard/mouse input (null = not measured).</summary>
    public int? IdleSeconds { get; set; }

    /// <summary>Why the block ended ("interval", "idle", "lock", "sleep", …); null when unknown.</summary>
    public string? EndReason { get; set; }

    /// <summary>Server time minus the device clock at upload, in seconds (positive = device clock behind).</summary>
    public int? ClockSkewSeconds { get; set; }

    /// <summary>Software-injected input events seen during the block; excluded from the counts and Productivity.</summary>
    public int? InjectedInputEvents { get; set; }

    // Navigation property: screen + webcam captures taken during this block
    public ICollection<Screenshot> Screenshots { get; set; } = new List<Screenshot>();

    protected Activity()
    {
    }

    public Activity(
        Guid id, 
        Guid workspaceId, 
        Guid userId, 
        Guid projectId, 
        Guid clientActivityId, 
        DateTime startedAt, 
        DateTime endedAt) 
        : base(id)
    {
        if (clientActivityId == Guid.Empty)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ActivityClientIdRequired)
                .WithData("clientActivityId", clientActivityId);
        }

        if (endedAt <= startedAt)
        {
            throw new BusinessException(TrackerDomainErrorCodes.ActivityTimeRangeInvalid)
                .WithData("startedAt", startedAt)
                .WithData("endedAt", endedAt);
        }

        WorkspaceId = workspaceId;
        UserId = userId;
        ProjectId = projectId;
        ClientActivityId = clientActivityId;
        StartedAt = startedAt;
        EndedAt = endedAt;
    }
}
