using System;
using System.ComponentModel.DataAnnotations;
using System.Threading.Tasks;
using Volo.Abp.Application.Dtos;
using Volo.Abp.Application.Services;

namespace Dosi.Tracker.Activities;

/// <summary>
/// Deleting tracked data in the current workspace. Owners and managers (<c>Tracker.Activities.Delete</c>) delete any
/// member's block with its captures, or a capture alone, giving a reason; only an owner deletes an owner's data
/// (<c>Tracker:RoleNotAllowed</c>). A member deletes their own captures within the workspace's window after upload
/// (<c>Tracker:CaptureDeleteNotAllowed</c> after it). Every deletion removes the files from storage and is recorded
/// (who, when, what, why); a deleted block cannot be uploaded again (<c>Tracker:ActivityDeleted</c>).
/// </summary>
public interface IActivityErasureAppService : IApplicationService
{
    /// <summary><c>POST /api/app/activity/{id}/delete</c>: deletes the block with all its captures.</summary>
    Task<ActivityErasureDto> DeleteBlockAsync(Guid id, DeleteTrackedDataDto input);

    /// <summary><c>POST /api/app/activity/capture/{captureId}/delete</c>: deletes one capture (a screen image goes with
    /// its thumbnail); the block stays.</summary>
    Task<ActivityErasureDto> DeleteCaptureAsync(Guid captureId, DeleteCaptureDto input);

    /// <summary><c>GET /api/app/activity/erasures</c>: the record of deletions, newest first — all of the workspace's
    /// for owners and managers, those of the caller's own data for everyone else.</summary>
    Task<PagedResultDto<ActivityErasureDto>> GetErasuresAsync(PagedResultRequestDto input);
}

/// <summary>Why tracked data is deleted (kept in the record of the deletion).</summary>
public class DeleteTrackedDataDto
{
    [Required]
    [StringLength(ActivityErasureLimits.MaxReasonLength, MinimumLength = ActivityErasureLimits.MinReasonLength)]
    public string Reason { get; set; } = string.Empty;
}

/// <summary>Why a capture is deleted: required from owners and managers deleting someone else's capture, optional for
/// a member deleting their own.</summary>
public class DeleteCaptureDto
{
    [MaxLength(ActivityErasureLimits.MaxReasonLength)]
    public string? Reason { get; set; }
}

public static class ActivityErasureLimits
{
    public const int MinReasonLength = 3;
    public const int MaxReasonLength = 512;
}

/// <summary>One deletion of tracked data. It names what was deleted, never what it contained.</summary>
public class ActivityErasureDto
{
    public Guid Id { get; set; }
    public Guid ActivityId { get; set; }

    /// <summary>The member whose data was deleted.</summary>
    public Guid UserId { get; set; }

    public Guid ProjectId { get; set; }
    public DateTime StartedAt { get; set; }
    public DateTime EndedAt { get; set; }

    /// <summary>"block" (the block with its captures) or "capture" (captures only).</summary>
    public string Scope { get; set; } = string.Empty;

    /// <summary>Kinds of the deleted captures, comma-separated ("screen,thumb"), or null.</summary>
    public string? CaptureKinds { get; set; }

    public int CaptureCount { get; set; }
    public string? Reason { get; set; }

    /// <summary>Who deleted it.</summary>
    public Guid? DeletedBy { get; set; }

    public DateTime DeletionTime { get; set; }
}
