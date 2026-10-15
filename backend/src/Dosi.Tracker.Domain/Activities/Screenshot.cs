using System;
using Volo.Abp.Domain.Entities.Auditing;
using Dosi.Tracker.Workspaces;

namespace Dosi.Tracker.Activities;

public class Screenshot : CreationAuditedEntity<Guid>, IWorkspaceScoped
{
    public const string ScreenKind = "screen";
    public const string WebcamKind = "webcam";

    /// <summary>A small JPEG rendition of the block's screen capture, used for list thumbnails.</summary>
    public const string ThumbKind = "thumb";

    public Guid WorkspaceId { get; set; }
    public Guid ActivityId { get; set; }
    public string Kind { get; set; } // "screen", "thumb" or "webcam"
    public string StorageUrl { get; set; } // Blob name (S3/R2 key or database blob name)
    public bool Blurred { get; set; }
    public DateTime CapturedAt { get; set; }
    public long SizeBytes { get; set; }
    public string ContentType { get; set; }

    protected Screenshot()
    {
    }

    public Screenshot(
        Guid id,
        Guid workspaceId,
        Guid activityId,
        string storageUrl,
        DateTime capturedAt,
        long sizeBytes,
        string contentType = "image/jpeg",
        string kind = ScreenKind)
    {
        Id = id;
        WorkspaceId = workspaceId;
        ActivityId = activityId;
        StorageUrl = storageUrl;
        CapturedAt = capturedAt;
        SizeBytes = sizeBytes;
        ContentType = contentType;
        Kind = kind;
    }
}
