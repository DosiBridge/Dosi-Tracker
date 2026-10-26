using System;
using System.Collections.Generic;
using System.Linq;
using Dosi.Tracker.Workspaces;
using Volo.Abp.Domain.Entities.Auditing;

namespace Dosi.Tracker.Activities;

/// <summary>
/// The record of a deletion of tracked data: a whole block with its captures (<see cref="BlockScope"/>), or captures
/// alone (<see cref="CaptureScope"/>) — who deleted it (<c>CreatorId</c>), when, why, and what. It holds no content:
/// only ids, the block's time range and the reason. A deleted block's record is also its tombstone: the member's
/// client id stays taken, so an agent that uploads the block again is answered <c>Tracker:ActivityDeleted</c> (410)
/// instead of recreating it.
/// </summary>
public class ActivityErasure : CreationAuditedEntity<Guid>, IWorkspaceScoped
{
    public const string BlockScope = "block";
    public const string CaptureScope = "capture";

    public const int MaxReasonLength = 512;
    public const int MaxScopeLength = 16;
    public const int MaxCaptureKindsLength = 64;

    public Guid WorkspaceId { get; private set; }
    public Guid ActivityId { get; private set; }

    /// <summary>The member who tracked the block.</summary>
    public Guid UserId { get; private set; }

    public Guid ProjectId { get; private set; }
    public Guid ClientActivityId { get; private set; }
    public DateTime StartedAt { get; private set; }
    public DateTime EndedAt { get; private set; }

    /// <summary><see cref="BlockScope"/> or <see cref="CaptureScope"/>.</summary>
    public string Scope { get; private set; } = null!;

    /// <summary>The kinds of the deleted captures, comma-separated ("screen,thumb"); null when there were none.</summary>
    public string? CaptureKinds { get; private set; }

    public int CaptureCount { get; private set; }

    public string? Reason { get; private set; }

    protected ActivityErasure()
    {
    }

    public ActivityErasure(Guid id, Activity activity, string scope, IReadOnlyCollection<Screenshot> captures, string? reason)
    {
        Id = id;
        WorkspaceId = activity.WorkspaceId;
        ActivityId = activity.Id;
        UserId = activity.UserId;
        ProjectId = activity.ProjectId;
        ClientActivityId = activity.ClientActivityId;
        StartedAt = activity.StartedAt;
        EndedAt = activity.EndedAt;
        Scope = scope;
        CaptureCount = captures.Count;
        CaptureKinds = captures.Count == 0
            ? null
            : string.Join(",", captures.Select(c => c.Kind).Distinct().OrderBy(k => k, StringComparer.Ordinal));
        Reason = string.IsNullOrWhiteSpace(reason)
            ? null
            : reason.Trim().Length > MaxReasonLength ? reason.Trim()[..MaxReasonLength] : reason.Trim();
    }
}
