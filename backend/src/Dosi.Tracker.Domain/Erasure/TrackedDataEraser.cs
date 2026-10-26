using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Microsoft.Extensions.Logging;
using Volo.Abp.Domain.Repositories;

namespace Dosi.Tracker.Erasure;

/// <summary>
/// Deletes tracked data on request — a block with all its captures, or captures alone — and records each deletion
/// (<see cref="ActivityErasure"/>). Who may delete what is decided by the caller. The files go first: a file that cannot
/// be deleted fails the request (it can be repeated) rather than leaving a capture that looks deleted but is not.
/// A capture's screen image and its thumbnail are one capture: deleting either deletes both.
/// </summary>
public class TrackedDataEraser : TrackerDomainService
{
    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IRepository<Screenshot, Guid> _screenshotRepository;
    private readonly IRepository<ActivityErasure, Guid> _erasureRepository;
    private readonly CaptureEraser _captureEraser;

    public TrackedDataEraser(
        IRepository<Activity, Guid> activityRepository,
        IRepository<Screenshot, Guid> screenshotRepository,
        IRepository<ActivityErasure, Guid> erasureRepository,
        CaptureEraser captureEraser)
    {
        _activityRepository = activityRepository;
        _screenshotRepository = screenshotRepository;
        _erasureRepository = erasureRepository;
        _captureEraser = captureEraser;
    }

    /// <summary>Deletes the block, its captures and their files, and records it (the tombstone that refuses a later
    /// upload of the same block).</summary>
    public virtual async Task<ActivityErasure> EraseBlockAsync(Activity activity, string? reason)
    {
        var captures = await _screenshotRepository.GetListAsync(s => s.ActivityId == activity.Id);
        await _captureEraser.DeleteAsync(captures, throwOnFileFailure: true);
        await _activityRepository.DeleteAsync(activity, autoSave: true);

        var record = new ActivityErasure(GuidGenerator.Create(), activity, ActivityErasure.BlockScope, captures, reason);
        await _erasureRepository.InsertAsync(record, autoSave: true);

        Logger.LogInformation(
            "Block {ActivityId} of user {UserId} in workspace {WorkspaceId} was deleted with {Count} capture(s) by {ErasedBy}.",
            activity.Id, activity.UserId, activity.WorkspaceId, captures.Count, CurrentUser.Id);
        return record;
    }

    /// <summary>The capture <paramref name="capture"/> belongs to, as the captures to delete together: the screen image
    /// with its thumbnail, or the webcam frame alone.</summary>
    public virtual async Task<List<Screenshot>> GetCaptureGroupAsync(Screenshot capture)
    {
        if (capture.Kind == Screenshot.WebcamKind)
        {
            return new List<Screenshot> { capture };
        }

        var activityId = capture.ActivityId;
        var group = await _screenshotRepository.GetListAsync(s =>
            s.ActivityId == activityId && (s.Kind == Screenshot.ScreenKind || s.Kind == Screenshot.ThumbKind));
        return group.Any(s => s.Id == capture.Id) ? group : group.Append(capture).ToList();
    }

    /// <summary>Deletes the captures (of one block) and their files, and records it; the block stays.</summary>
    public virtual async Task<ActivityErasure> EraseCapturesAsync(Activity activity, IReadOnlyCollection<Screenshot> captures, string? reason)
    {
        await _captureEraser.DeleteAsync(captures, throwOnFileFailure: true);

        var record = new ActivityErasure(GuidGenerator.Create(), activity, ActivityErasure.CaptureScope, captures, reason);
        await _erasureRepository.InsertAsync(record, autoSave: true);

        Logger.LogInformation(
            "{Count} capture(s) of block {ActivityId} in workspace {WorkspaceId} were deleted by {ErasedBy}.",
            captures.Count, activity.Id, activity.WorkspaceId, CurrentUser.Id);
        return record;
    }

    /// <summary>True when the member's block with this client id was deleted (in the current workspace).</summary>
    public virtual Task<bool> IsBlockErasedAsync(Guid userId, Guid clientActivityId)
    {
        return _erasureRepository.AnyAsync(e =>
            e.UserId == userId && e.ClientActivityId == clientActivityId && e.Scope == ActivityErasure.BlockScope);
    }
}
