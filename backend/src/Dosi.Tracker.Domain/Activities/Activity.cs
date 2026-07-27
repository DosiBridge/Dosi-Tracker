using System;
using System.Collections.Generic;
using Volo.Abp;
using Volo.Abp.Domain.Entities.Auditing;
using Volo.Abp.MultiTenancy;

namespace Dosi.Tracker.Activities;

public class Activity : CreationAuditedAggregateRoot<Guid>, IMultiTenant
{
    public Guid? TenantId { get; set; }
    public Guid UserId { get; set; }
    public Guid ProjectId { get; set; }
    public Guid ClientActivityId { get; set; } // Offline Idempotency key from Agent

    public DateTime StartedAt { get; set; } // Must be UTC
    public DateTime EndedAt { get; set; }   // Must be UTC

    public int Productivity { get; set; } // 0 - 100
    public int MouseClicks { get; set; }
    public int KeyboardHits { get; set; }

    public string? Description { get; set; } // Optional: agents typically send no description
    public string ActiveWindowsJson { get; set; } = "[]"; // JSON array of active windows
    public string RunningProgramsJson { get; set; } = "[]"; // JSON array of running apps

    // Navigation property: screen + webcam captures taken during this block
    public ICollection<Screenshot> Screenshots { get; set; } = new List<Screenshot>();

    protected Activity()
    {
    }

    public Activity(
        Guid id, 
        Guid? tenantId, 
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

        TenantId = tenantId;
        UserId = userId;
        ProjectId = projectId;
        ClientActivityId = clientActivityId;
        StartedAt = startedAt;
        EndedAt = endedAt;
    }
}
